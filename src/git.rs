//! Thin wrapper around the `git` command line: status parsing and repository operations.

use std::{
    env,
    path::Path,
    process::{Command, Stdio},
    sync::OnceLock,
};

/// One changed path as reported by `git status --porcelain=v2`.
#[derive(Clone)]
pub struct FileEntry {
    pub path: String,
    /// Index (staged) status letter; `' '` when unchanged, `'?'` when untracked.
    pub x: char,
    /// Working tree (unstaged) status letter; `' '` when unchanged, `'?'` when untracked.
    pub y: char,
}

impl FileEntry {
    pub fn untracked(&self) -> bool {
        self.x == '?'
    }
    pub fn staged(&self) -> bool {
        self.x != ' ' && !self.untracked() && !self.conflict()
    }
    pub fn unstaged(&self) -> bool {
        self.y != ' ' && !self.untracked()
    }
    pub fn conflict(&self) -> bool {
        matches!((self.x, self.y), ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D'))
    }
}

/// Parsed result of `git status --porcelain=v2 --branch`.
pub struct Status {
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<FileEntry>,
}

/// An action Oxide can run against a repository.
#[derive(Clone)]
pub enum Op {
    Fetch,
    Pull,
    Push,
    /// Merge the named branch; an empty name merges the upstream branch.
    Merge(String),
    Stage(String),
    Unstage(String),
    StageAll,
    UnstageAll,
    Commit(String),
}

impl Op {
    pub fn label(&self) -> &'static str {
        match self {
            Op::Fetch => "fetch",
            Op::Pull => "pull",
            Op::Push => "push",
            Op::Merge(_) => "merge",
            Op::Stage(_) => "stage",
            Op::Unstage(_) => "unstage",
            Op::StageAll => "stage all",
            Op::UnstageAll => "unstage all",
            Op::Commit(_) => "commit",
        }
    }
}

/// `ssh -o BatchMode=yes` unless the user configured their own ssh command.
/// Without it ssh would prompt on /dev/tty and corrupt the TUI.
fn ssh_batch() -> Option<&'static str> {
    static CMD: OnceLock<Option<&'static str>> = OnceLock::new();
    *CMD.get_or_init(|| {
        let custom = env::var_os("GIT_SSH_COMMAND").is_some()
            || env::var_os("GIT_SSH").is_some()
            || Command::new("git")
                .args(["config", "--get", "core.sshCommand"])
                .output()
                .is_ok_and(|o| o.status.success());
        (!custom).then_some("ssh -o BatchMode=yes")
    })
}

/// A `git` command that can never block on a prompt or an editor.
fn command() -> Command {
    let mut c = Command::new("git");
    c.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .stdin(Stdio::null());
    if let Some(ssh) = ssh_batch() {
        c.env("GIT_SSH_COMMAND", ssh);
    }
    c
}

/// Runs git in `repo`; returns `(success, stdout, stderr)`.
fn exec(repo: &Path, args: &[&str]) -> Result<(bool, String, String), String> {
    let out = command()
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
    ))
}

/// Runs git and returns stdout, or stderr as the error.
fn run(repo: &Path, args: &[&str]) -> Result<String, String> {
    let (ok, out, err) = exec(repo, args)?;
    if ok { Ok(out) } else { Err(err) }
}

/// Runs git and returns stdout plus stderr (git reports progress and results on stderr).
fn run_all(repo: &Path, args: &[&str]) -> Result<String, String> {
    let (ok, out, err) = exec(repo, args)?;
    let combined = format!("{}\n{}", out.trim(), err).trim().to_string();
    if ok { Ok(combined) } else { Err(combined) }
}

pub fn global_config(key: &str) -> Option<String> {
    let out = Command::new("git").args(["config", "--global", "--get", key]).output().ok()?;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !v.is_empty()).then_some(v)
}

/// URL of `origin`, or of the first remote when there is no `origin`.
pub fn remote_url(repo: &Path) -> Option<String> {
    let remotes = run(repo, &["remote"]).ok()?;
    let name = remotes.lines().find(|r| *r == "origin").or_else(|| remotes.lines().next())?;
    let url = run(repo, &["config", "--get", &format!("remote.{name}.url")]).ok()?;
    Some(url.trim().to_string()).filter(|u| !u.is_empty())
}

/// `<hash> <subject> (<relative date>)` of HEAD.
pub fn last_commit(repo: &Path) -> String {
    run(repo, &["log", "-1", "--format=%h %s (%cr)"])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "no commits yet".to_string())
}

pub fn status(repo: &Path) -> Result<Status, String> {
    let out = run(repo, &["status", "--porcelain=v2", "--branch", "-z"])?;
    let mut st = Status { branch: String::new(), upstream: None, ahead: 0, behind: 0, files: Vec::new() };

    // With -z every record is NUL-terminated and paths are not quoted.
    let mut records = out.split('\0');
    while let Some(rec) = records.next() {
        if let Some(h) = rec.strip_prefix("# ") {
            if let Some(b) = h.strip_prefix("branch.head ") {
                st.branch = b.to_string();
            } else if let Some(u) = h.strip_prefix("branch.upstream ") {
                st.upstream = Some(u.to_string());
            } else if let Some(ab) = h.strip_prefix("branch.ab ") {
                let mut it = ab.split(' ');
                st.ahead = it.next().and_then(|a| a.trim_start_matches('+').parse().ok()).unwrap_or(0);
                st.behind = it.next().and_then(|b| b.trim_start_matches('-').parse().ok()).unwrap_or(0);
            }
            continue;
        }
        // Ordinary (1), rename/copy (2), unmerged (u) and untracked (?) entries.
        let (xy, path) = match rec.as_bytes().first() {
            Some(b'1') => {
                let f: Vec<&str> = rec.splitn(9, ' ').collect();
                (f.get(1).copied().unwrap_or(".."), f.get(8).copied())
            }
            Some(b'2') => {
                let f: Vec<&str> = rec.splitn(10, ' ').collect();
                records.next(); // the original path of the rename follows as its own record
                (f.get(1).copied().unwrap_or(".."), f.get(9).copied())
            }
            Some(b'u') => {
                let f: Vec<&str> = rec.splitn(11, ' ').collect();
                (f.get(1).copied().unwrap_or("UU"), f.get(10).copied())
            }
            Some(b'?') => ("??", rec.get(2..)),
            _ => continue, // ignored files ('!') and empty trailing record
        };
        if let Some(path) = path {
            let mut c = xy.chars().map(|c| if c == '.' { ' ' } else { c });
            st.files.push(FileEntry {
                path: path.to_string(),
                x: c.next().unwrap_or(' '),
                y: c.next().unwrap_or(' '),
            });
        }
    }
    Ok(st)
}

/// Runs `op` in `repo`; the Ok/Err text is the command output for the log.
pub fn perform(repo: &Path, op: &Op) -> Result<String, String> {
    match op {
        Op::Fetch => run_all(repo, &["fetch", "--all", "--prune"]),
        // Fast-forward only: Oxide never creates surprise merge commits on pull.
        Op::Pull => run_all(repo, &["pull", "--ff-only"]),
        Op::Push => push(repo),
        Op::Merge(target) => {
            if target.starts_with('-') {
                return Err(format!("invalid merge target '{target}'"));
            }
            let target = if target.is_empty() { "@{u}" } else { target };
            run_all(repo, &["merge", "--no-edit", target])
        }
        Op::Stage(p) => run_all(repo, &["add", "--", p]),
        // `restore --staged` needs a HEAD; fall back to `rm --cached` in a fresh repository.
        Op::Unstage(p) => run_all(repo, &["restore", "--staged", "--", p])
            .or_else(|_| run_all(repo, &["rm", "--cached", "-r", "-q", "--", p])),
        Op::StageAll => run_all(repo, &["add", "-A"]),
        Op::UnstageAll => run_all(repo, &["reset", "-q"]),
        Op::Commit(msg) => run_all(repo, &["commit", "-m", msg]),
    }
}

/// Pushes the current branch, publishing it with `-u` when it has no upstream yet.
fn push(repo: &Path) -> Result<String, String> {
    if run(repo, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).is_ok() {
        return run_all(repo, &["push"]);
    }
    let remotes = run(repo, &["remote"])?;
    let remote = remotes
        .lines()
        .find(|r| *r == "origin")
        .or_else(|| remotes.lines().next())
        .ok_or("no remote configured")?;
    run_all(repo, &["push", "-u", remote, "HEAD"])
}
