//! Thin wrapper around the `git` command line: status parsing and repository operations.

use crate::scaffold::Plan;

use std::{
    env,
    path::{Path, PathBuf},
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
    /// Number of stash entries (the `# stash` header, git 2.35 or newer).
    pub stashes: u32,
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
    /// Stash tracked and untracked changes.
    Stash,
    StashPop,
    /// `git init`, creating the folder first when it does not exist.
    Init,
    /// Switch to a branch; `true` creates a local tracking branch from a remote one.
    Switch(String, bool),
    /// Create a branch from HEAD and switch to it.
    NewBranch(String),
    /// Delete a local branch; `true` deletes it even when it is not merged.
    DeleteBranch(String, bool),
    /// Check out the branch of a GitHub pull request (through `gh`).
    CheckoutPr(u64),
    /// Clone a GitHub repository (`owner/repo`) into the operation's folder.
    Clone(String),
    /// Delete the repository's folder from disk.
    Delete,
    /// Amend the last commit with the staged changes; an empty message keeps the old one.
    Amend(String),
    /// `git reset --keep` to a commit: moves the branch, keeps uncommitted changes (refuses when they would be lost).
    Reset(String),
    /// Check out a branch or commit by name.
    Checkout(String),
    /// Create a worktree for a branch at `path`, creating the branch when it does not exist.
    AddWorktree { path: PathBuf, branch: String },
    /// Remove a linked worktree; `force` also discards its uncommitted changes.
    RemoveWorktree { path: PathBuf, force: bool },
    /// Forget worktrees whose folder is gone.
    PruneWorktrees,
    /// Initialise and update a submodule (an empty path means all of them, recursively).
    UpdateSubmodule(String),
    /// `git remote add`.
    RemoteAdd { name: String, url: String },
    /// Point an existing remote at another URL.
    RemoteSetUrl { name: String, url: String },
    RemoteRename { old: String, new: String },
    RemoteRemove(String),
    /// Fetch one named remote.
    RemoteFetch(String),
    /// Apply one hunk of a diff to the index: stage it, or with `reverse` unstage it.
    Hunk { patch: String, reverse: bool },
    /// Write a new workflow file; `publish` also commits and pushes it, and `dispatch` (repository, branch) then
    /// starts it for the first time.
    NewWorkflow { file: String, text: String, publish: bool, message: String, dispatch: Option<(String, String)> },
    /// Start a workflow by hand (`gh workflow run`) on a branch or tag, with its inputs.
    Dispatch { slug: String, file: String, git_ref: String, inputs: Vec<(String, String)> },
    /// Undo the last project setup recorded in the repository (see `setup_record`).
    UndoSetup,
    /// Create a tag on HEAD; a message makes it an annotated tag.
    Tag { name: String, message: String },
    /// Push one tag to the repository's remote.
    PushTag(String),
    /// Delete a tag; `remote` deletes it from the remote first.
    DeleteTag { name: String, remote: bool },
    /// Create a GitHub release for a tag that is already on the remote, with generated notes.
    Release { tag: String, slug: String, draft: bool },
    /// Set a new project up with a plan from the new-project menu; `commit` also commits the result.
    Setup { plan: Box<Plan>, commit: bool },
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
            Op::Stash => "stash",
            Op::StashPop => "stash pop",
            Op::Init => "init",
            Op::Switch(..) => "switch",
            Op::NewBranch(_) => "new branch",
            Op::DeleteBranch(..) => "delete branch",
            Op::CheckoutPr(_) => "pr checkout",
            Op::Clone(_) => "clone",
            Op::Delete => "delete",
            Op::AddWorktree { .. } => "worktree add",
            Op::RemoveWorktree { .. } => "worktree remove",
            Op::PruneWorktrees => "worktree prune",
            Op::UpdateSubmodule(_) => "submodule update",
            Op::RemoteAdd { .. } => "remote add",
            Op::RemoteSetUrl { .. } => "remote set-url",
            Op::RemoteRename { .. } => "remote rename",
            Op::RemoteRemove(_) => "remote remove",
            Op::RemoteFetch(_) => "remote fetch",
            Op::Amend(_) => "amend",
            Op::Reset(_) => "reset",
            Op::Checkout(_) => "checkout",
            Op::Hunk { reverse: false, .. } => "stage hunk",
            Op::Hunk { reverse: true, .. } => "unstage hunk",
            Op::Setup { .. } => "set up",
            Op::UndoSetup => "undo setup",
            Op::Dispatch { .. } => "workflow run",
            Op::NewWorkflow { .. } => "new workflow",
            Op::Tag { .. } => "tag",
            Op::PushTag(_) => "push tag",
            Op::DeleteTag { .. } => "delete tag",
            Op::Release { draft: false, .. } => "release",
            Op::Release { draft: true, .. } => "draft release",
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

/// Every file of `repo` that git tracks, or could add (untracked and not ignored), sorted. Deleted files are left out.
pub fn project_files(repo: &Path) -> Result<Vec<String>, String> {
    let out = run(repo, &["ls-files", "-z", "--cached", "--others", "--exclude-standard"])?;
    let mut files: Vec<String> = out.split('\0').filter(|p| !p.is_empty() && repo.join(p).is_file()).map(str::to_string).collect();
    files.sort();
    files.dedup();
    Ok(files)
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

/// Whether a path of the repository is in step with the last commit.
#[derive(Clone, Copy, PartialEq)]
pub enum PathState {
    Clean,
    /// Not tracked by git (a new file).
    Untracked,
    /// Tracked, with uncommitted changes.
    Changed,
}

/// The state of one path (relative to the repository) against the last commit.
pub fn path_state(repo: &Path, rel: &str) -> PathState {
    match run(repo, &["status", "--porcelain", "--", rel]) {
        Ok(out) if out.trim().is_empty() => PathState::Clean,
        Ok(out) if out.starts_with("??") => PathState::Untracked,
        _ => PathState::Changed,
    }
}

/// Name of the remote Oxide talks to: `origin`, else the first one.
pub fn default_remote(repo: &Path) -> Option<String> {
    let remotes = run(repo, &["remote"]).ok()?;
    remotes.lines().find(|r| *r == "origin").or_else(|| remotes.lines().next()).map(str::to_string)
}

/// A tag of a repository.
pub struct Tag {
    pub name: String,
    /// An annotated tag (with its own message); false for a lightweight one.
    pub annotated: bool,
    /// Short hash of the commit the tag points at.
    pub hash: String,
    /// When it was made, e.g. `3 days ago`.
    pub age: String,
    /// The tag message's first line, or the commit subject of a lightweight tag.
    pub subject: String,
}

/// The tags of a repository, newest first.
pub fn tags(repo: &Path) -> Result<Vec<Tag>, String> {
    let format = "%(refname:short)%09%(objecttype)%09%(objectname:short)%09%(*objectname:short)%09%(creatordate:relative)%09%(contents:subject)";
    let out = run(repo, &["for-each-ref", "--sort=-creatordate", &format!("--format={format}"), "refs/tags"])?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            let [name, kind, hash, peeled, age, subject] = f[..] else { return None };
            // For an annotated tag the commit is the peeled object; for a lightweight one the tag is the commit.
            let commit = if peeled.is_empty() { hash } else { peeled };
            Some(Tag { name: name.into(), annotated: kind == "tag", hash: commit.into(), age: age.into(), subject: subject.into() })
        })
        .collect())
}

/// Names of the tags on the remote (asks the server, so it can take a while or fail offline).
pub fn remote_tags(repo: &Path) -> Result<std::collections::HashSet<String>, String> {
    let remote = default_remote(repo).ok_or("no remote")?;
    let out = run(repo, &["ls-remote", "--tags", "--refs", &remote])?;
    Ok(out.lines().filter_map(|l| l.split_once("\trefs/tags/").map(|(_, n)| n.to_string())).collect())
}

/// `<hash> <subject> (<relative date>)` of HEAD and its commit time as a Unix timestamp.
pub fn last_commit(repo: &Path) -> (String, i64) {
    // %x1f (unit separator) splits the text from the timestamp, since a subject may contain anything else.
    let out = run(repo, &["log", "-1", "--format=%h %s (%cr)%x1f%ct"]).unwrap_or_default();
    match out.trim().split_once('\x1f') {
        Some((text, time)) => (text.to_string(), time.trim().parse().unwrap_or(0)),
        None => ("no commits yet".to_string(), 0),
    }
}

/// `--no-optional-locks` keeps status from refreshing (rewriting) the index, which the file watcher would report as a change again.
pub fn status(repo: &Path) -> Result<Status, String> {
    let out = run(repo, &["--no-optional-locks", "status", "--porcelain=v2", "--branch", "--show-stash", "-z"])?;
    let mut st = Status { branch: String::new(), upstream: None, ahead: 0, behind: 0, stashes: 0, files: Vec::new() };

    // With -z every record is NUL-terminated and paths are not quoted.
    let mut records = out.split('\0');
    while let Some(rec) = records.next() {
        if let Some(h) = rec.strip_prefix("# ") {
            if let Some(b) = h.strip_prefix("branch.head ") {
                st.branch = b.to_string();
            } else if let Some(u) = h.strip_prefix("branch.upstream ") {
                st.upstream = Some(u.to_string());
            } else if let Some(n) = h.strip_prefix("stash ") {
                st.stashes = n.trim().parse().unwrap_or(0);
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

/// Which version of a file the diff viewer shows.
#[derive(Clone, Copy, PartialEq)]
pub enum DiffKind {
    /// Working tree against the index.
    Unstaged,
    /// Index against HEAD.
    Staged,
    /// A file git does not track yet: shown as entirely added.
    Untracked,
    /// A whole commit; the diff target is its hash.
    Commit,
    /// A GitHub pull request; the diff target is `owner/repo#number`.
    Pull,
}

impl DiffKind {
    pub fn label(self) -> &'static str {
        match self {
            DiffKind::Unstaged => "unstaged",
            DiffKind::Staged => "staged",
            DiffKind::Untracked => "untracked",
            DiffKind::Commit => "commit",
            DiffKind::Pull => "pull request",
        }
    }
}

/// Unified diff text of `file` in `repo` (no color, no external diff tool, no rename detection).
pub fn diff(repo: &Path, file: &str, kind: DiffKind) -> Result<String, String> {
    let base = ["-c", "core.quotepath=off", "diff", "--no-color", "--no-ext-diff", "--no-renames"];
    match kind {
        DiffKind::Unstaged => run(repo, &[&base[..], &["--", file]].concat()),
        DiffKind::Staged => run(repo, &[&base[..], &["--cached", "--", file]].concat()),
        DiffKind::Pull => {
            let (slug, number) = file.rsplit_once('#').ok_or("bad pull request reference")?;
            crate::github::pr_diff(slug, number.parse().map_err(|_| "bad pull request number")?)
        }
        DiffKind::Commit if file.starts_with('-') => Err(format!("invalid commit '{file}'")),
        DiffKind::Commit => {
            let format = "--format=commit %H%nAuthor: %an <%ae>%nDate:   %ad%n%n%w(0,4,4)%B";
            run(repo, &["-c", "core.quotepath=off", "show", "--no-color", "--no-ext-diff", "--no-renames", format, file])
        }
        DiffKind::Untracked if file.ends_with('/') => {
            // Git lists a wholly untracked directory as one entry; show what is inside it.
            let list = run(repo, &["ls-files", "--others", "--exclude-standard", "--", file])?;
            Ok(format!("untracked directory {file}\n{list}"))
        }
        // `--no-index` exits with 1 when the files differ, which is the normal case here.
        DiffKind::Untracked => {
            let (_, out, err) = exec(repo, &[&base[..], &["--no-index", "--", "/dev/null", file]].concat())?;
            if out.is_empty() && !err.is_empty() { Err(err) } else { Ok(out) }
        }
    }
}

/// One entry of the commit log.
pub struct Commit {
    pub hash: String,
    pub author: String,
    /// Relative date, e.g. `3 days ago`.
    pub date: String,
    /// Branches and tags pointing at the commit, e.g. `HEAD -> main, origin/main`.
    pub refs: String,
    pub subject: String,
}

/// The last `limit` commits reachable from HEAD, newest first.
pub fn log(repo: &Path, limit: usize) -> Result<Vec<Commit>, String> {
    let n = format!("-n{limit}");
    let out = run(repo, &["log", &n, "--format=%h%x1f%an%x1f%cr%x1f%D%x1f%s"])?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let mut f = l.split('\x1f');
            Some(Commit {
                hash: f.next()?.to_string(),
                author: f.next()?.to_string(),
                date: f.next()?.to_string(),
                refs: f.next()?.to_string(),
                subject: f.next().unwrap_or("").to_string(),
            })
        })
        .collect())
}

/// One line of `git reflog`: a place HEAD has been.
pub struct ReflogEntry {
    pub hash: String,
    pub short: String,
    /// What moved HEAD there, e.g. `commit: Fix typo` or `checkout: moving from main to dev`.
    pub subject: String,
    pub date: String,
}

/// The last `limit` positions of HEAD, newest first.
pub fn reflog(repo: &Path, limit: usize) -> Result<Vec<ReflogEntry>, String> {
    let n = format!("-n{limit}");
    let out = run(repo, &["reflog", "show", "HEAD", &n, "--format=%H%x1f%h%x1f%gs%x1f%cr"])?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let mut f = l.split('\x1f');
            Some(ReflogEntry { hash: f.next()?.to_string(), short: f.next()?.to_string(), subject: f.next()?.to_string(), date: f.next()?.to_string() })
        })
        .collect())
}

/// The value git would use for `key` in `repo` (local settings first, then global).
pub fn config_get(repo: &Path, key: &str) -> Option<String> {
    let v = run(repo, &["config", "--get", key]).ok()?.trim().to_string();
    (!v.is_empty()).then_some(v)
}

/// Every file committed at HEAD as `(path, size in bytes)`, from one `git ls-tree` (no checkout needed).
/// Symlinks and submodules are left out.
pub fn tree_blobs(repo: &Path) -> Result<Vec<(String, u64)>, String> {
    let out = run(repo, &["ls-tree", "-r", "-l", "-z", "HEAD"])?;
    // Each record is `<mode> <type> <hash> <size>\t<path>`; the size column is padded with spaces.
    Ok(out
        .split('\0')
        .filter_map(|rec| {
            let (meta, path) = rec.split_once('\t')?;
            let mut f = meta.split_whitespace();
            let (mode, kind, _hash, size) = (f.next()?, f.next()?, f.next()?, f.next()?);
            if kind != "blob" || mode == "120000" {
                return None;
            }
            Some((path.to_string(), size.parse().ok()?))
        })
        .collect())
}

/// A configured remote.
pub struct Remote {
    pub name: String,
    pub url: String,
    /// The push URL when it differs from the fetch URL.
    pub push_url: Option<String>,
}

/// Every remote with its URLs, from `git remote -v`.
pub fn remotes(repo: &Path) -> Result<Vec<Remote>, String> {
    let out = run(repo, &["remote", "-v"])?;
    let mut list: Vec<Remote> = Vec::new();
    // Lines look like `origin<TAB>git@github.com:me/repo.git (fetch)`, then the same with `(push)`.
    for line in out.lines() {
        let Some((name, rest)) = line.split_once('\t') else { continue };
        let Some((url, kind)) = rest.rsplit_once(' ') else { continue };
        match kind {
            "(fetch)" if !list.iter().any(|r| r.name == name) => list.push(Remote { name: name.to_string(), url: url.to_string(), push_url: None }),
            "(push)" => {
                if let Some(r) = list.iter_mut().find(|r| r.name == name && r.url != url) {
                    r.push_url = Some(url.to_string());
                }
            }
            _ => {}
        }
    }
    Ok(list)
}

/// A linked worktree or a submodule of a repository.
pub struct TreeItem {
    pub kind: TreeKind,
    /// Absolute folder of a worktree; folder of a submodule relative to the repository.
    pub path: String,
    /// Branch of a worktree (`(detached)` without one); `git describe` of a submodule.
    pub label: String,
    /// Abbreviated commit that is checked out.
    pub head: String,
}

pub enum TreeKind {
    /// The first worktree: the repository's own folder.
    Main,
    Linked { locked: bool, prunable: bool },
    /// `state` is git's status character: `' '` in sync, `-` not initialised, `+` other commit, `U` conflicts.
    Submodule { state: char },
}

/// The repository's worktrees, then its submodules.
pub fn trees(repo: &Path) -> Result<Vec<TreeItem>, String> {
    let mut items = Vec::new();
    let out = run(repo, &["worktree", "list", "--porcelain"])?;
    // Blocks of `key value` lines separated by blank lines; the first block is the main worktree.
    for (n, block) in out.split("\n\n").filter(|b| !b.trim().is_empty()).enumerate() {
        let (mut path, mut head, mut branch) = (String::new(), String::new(), String::new());
        let (mut locked, mut prunable, mut bare) = (false, false, false);
        for line in block.lines() {
            let (key, val) = line.split_once(' ').unwrap_or((line, ""));
            match key {
                "worktree" => path = val.to_string(),
                "HEAD" => head = val.chars().take(7).collect(),
                "branch" => branch = val.strip_prefix("refs/heads/").unwrap_or(val).to_string(),
                "locked" => locked = true,
                "prunable" => prunable = true,
                "bare" => bare = true,
                _ => {}
            }
        }
        if bare {
            continue;
        }
        let kind = if n == 0 { TreeKind::Main } else { TreeKind::Linked { locked, prunable } };
        let label = if branch.is_empty() { "(detached)".to_string() } else { branch };
        items.push(TreeItem { kind, path, label, head });
    }
    // `git submodule status`: `<state char><sha> <path> (<describe>)`; the describe part is missing when not initialised.
    for line in run(repo, &["submodule", "status"]).unwrap_or_default().lines() {
        let state = line.chars().next().unwrap_or(' ');
        let rest = line.get(1..).unwrap_or("");
        let Some((sha, rest)) = rest.split_once(' ') else { continue };
        let (path, label) = match rest.rsplit_once(" (") {
            Some((path, describe)) => (path, describe.trim_end_matches(')')),
            None => (rest, ""),
        };
        items.push(TreeItem { kind: TreeKind::Submodule { state }, path: path.to_string(), label: label.to_string(), head: sha.chars().take(7).collect() });
    }
    Ok(items)
}

/// A local or remote-tracking branch.
pub struct Branch {
    /// Short name: `main` or `origin/main`.
    pub name: String,
    pub remote: bool,
    pub current: bool,
    /// Already merged into HEAD, so it can be deleted safely.
    pub merged: bool,
    /// No commit for more than 90 days.
    pub stale: bool,
    /// Upstream and sync state, e.g. `origin/main [ahead 1]`.
    pub upstream: String,
    pub date: String,
}

/// Local branches first, then remote-tracking ones.
pub fn branches(repo: &Path) -> Result<Vec<Branch>, String> {
    let fmt = "--format=%(HEAD)%1f%(refname)%1f%(refname:short)%1f%(upstream:short)%1f%(upstream:track)%1f%(committerdate:unix)%1f%(committerdate:relative)";
    let out = run(repo, &["for-each-ref", fmt, "refs/heads", "refs/remotes"])?;
    // Merged-ness needs a commit to compare with; a repository without commits has no branches anyway.
    let merged = run(repo, &["for-each-ref", "--merged=HEAD", "--format=%(refname)", "refs/heads", "refs/remotes"]).unwrap_or_default();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    Ok(out
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\x1f').collect();
            let [head, full, name, upstream, track, ts, date] = f[..] else { return None };
            if full.ends_with("/HEAD") {
                return None; // the `origin/HEAD` pointer is not a branch
            }
            let ts: i64 = ts.parse().unwrap_or(now);
            let upstream = match (upstream.is_empty(), track.is_empty()) {
                (true, _) => String::new(),
                (false, true) => upstream.to_string(),
                (false, false) => format!("{upstream} {track}"),
            };
            Some(Branch {
                name: name.to_string(),
                remote: full.starts_with("refs/remotes/"),
                current: head == "*",
                merged: merged.lines().any(|m| m == full),
                stale: now - ts > 90 * 24 * 3600,
                upstream,
                date: date.to_string(),
            })
        })
        .collect())
}

/// Message of the commit that can follow a project setup.
const SETUP_COMMIT: &str = "Add initial commit and projects' initial files";

/// Does the plan in `repo`, then commits everything when asked. The text is one line per finished step,
/// and on failure ends with the reason.
fn setup(repo: &Path, plan: &Plan, commit: bool) -> Result<String, String> {
    // Everything in the folder besides `.git` is then the setup's, which is what undoing it removes.
    if !crate::scaffold::is_empty(repo) {
        return Err("the repository is not empty any more: nothing was done".to_string());
    }
    let mut lines = Vec::new();
    let applied = plan.apply(repo, |l| lines.push(l));
    write_record(repo, None); // also after a failure, so what was made can be undone
    if let Err(e) = applied {
        lines.push(e);
        lines.push("X undoes what was made".to_string());
        return Err(lines.join("\n"));
    }
    if commit {
        let made = run_all(repo, &["add", "-A"]).and_then(|_| run_all(repo, &["commit", "-m", SETUP_COMMIT]));
        match made {
            Ok(_) => {
                lines.push(format!("committed: {SETUP_COMMIT}"));
                write_record(repo, run(repo, &["rev-parse", "HEAD"]).ok().map(|h| h.trim().to_string()).as_deref());
            }
            Err(e) => {
                lines.push(format!("the files were made, but the commit failed: {e}"));
                return Err(lines.join("\n"));
            }
        }
    }
    lines.push("X undoes this setup".to_string());
    Ok(lines.join("\n"))
}

/// Writes `.github/workflows/<file>`. With `publish` it then commits only that file, pushes the branch and, with
/// `dispatch`, starts the workflow; GitHub needs a moment to know a workflow it was just pushed, so starting is retried.
fn new_workflow(repo: &Path, file: &str, text: &str, publish: bool, message: &str, dispatch: Option<&(String, String)>) -> Result<String, String> {
    if file.is_empty() || file.contains('/') || file.starts_with('.') {
        return Err(format!("invalid workflow file name '{file}'"));
    }
    let rel = format!("{}/{file}", crate::workflow::DIR);
    let dir = repo.join(crate::workflow::DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    std::fs::write(repo.join(&rel), text).map_err(|e| format!("cannot write {rel}: {e}"))?;
    let mut lines = vec![format!("wrote {rel}")];
    if !publish {
        lines.push("N then Enter runs it once it is pushed".to_string());
        return Ok(lines.join("\n"));
    }
    let remote = default_remote(repo).ok_or("no remote to push to")?;
    let steps = run_all(repo, &["add", "--", &rel])
        .and_then(|_| run_all(repo, &["commit", "-m", message, "--", &rel]))
        .and_then(|_| run_all(repo, &["push", &remote, "HEAD"]));
    if let Err(e) = steps {
        lines.push(format!("committing or pushing failed: {e}"));
        return Err(lines.join("\n"));
    }
    lines.push(format!("committed and pushed to {remote}"));
    if let Some((slug, git_ref)) = dispatch {
        let mut last = String::new();
        for attempt in 0..6 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_secs(4));
            }
            match crate::github::gh_all(Some(repo), &["workflow", "run", file, "-R", slug, "--ref", git_ref]) {
                Ok(out) => {
                    lines.push(format!("started: {}", out.lines().next().unwrap_or("")));
                    return Ok(lines.join("\n"));
                }
                Err(e) => last = e,
            }
        }
        lines.push(format!("pushed, but GitHub would not start it yet ({}): press N, then Enter on it", last.lines().next().unwrap_or("")));
    } else {
        lines.push("pushed: it runs on its triggers".to_string());
    }
    Ok(lines.join("\n"))
}

// ---- undoing a setup ----

/// What the last setup made: the top-level entries of the folder (the repository was empty before) and, if it
/// made one, the commit. It lives in `oxide-setup` inside the git directory, so it goes with the repository.
pub struct SetupRecord {
    pub entries: Vec<String>,
    pub commit: Option<String>,
}

fn record_path(repo: &Path) -> Option<PathBuf> {
    let p = PathBuf::from(run(repo, &["rev-parse", "--git-path", "oxide-setup"]).ok()?.trim());
    Some(if p.is_absolute() { p } else { repo.join(p) })
}

/// Records the folder's current top-level entries (and `commit`) as the last setup.
fn write_record(repo: &Path, commit: Option<&str>) {
    let Some(path) = record_path(repo) else { return };
    let mut names: Vec<String> =
        std::fs::read_dir(repo).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n != ".git").collect();
    names.sort();
    let mut text = String::from("oxide-setup 1\n");
    if let Some(c) = commit {
        text.push_str(&format!("commit {c}\n"));
    }
    for n in names {
        text.push_str(&format!("entry {n}\n"));
    }
    let _ = std::fs::write(path, text);
}

/// The record of the last setup, if there is one. Entry names that could point outside the folder are dropped.
pub fn setup_record(repo: &Path) -> Option<SetupRecord> {
    let text = std::fs::read_to_string(record_path(repo)?).ok()?;
    let mut rec = SetupRecord { entries: Vec::new(), commit: None };
    for line in text.lines() {
        if let Some(c) = line.strip_prefix("commit ") {
            rec.commit = Some(c.to_string());
        } else if let Some(n) = line.strip_prefix("entry ")
            && !n.is_empty()
            && !n.contains('/')
            && !matches!(n, "." | ".." | ".git")
        {
            rec.entries.push(n.to_string());
        }
    }
    Some(rec)
}

/// Files below `path` (itself when it is a file); links are counted, not followed.
fn count_files(path: &Path) -> usize {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::read_dir(path).into_iter().flatten().flatten().map(|e| count_files(&e.path())).sum(),
        Ok(_) => 1,
        Err(_) => 0,
    }
}

/// Whether the setup in `rec` can still be undone: the setup commit has to be the only commit, so nothing built
/// on it is lost. Returns the entries that are still there.
fn undo_guard(repo: &Path, rec: &SetupRecord) -> Result<Vec<String>, String> {
    if let Some(commit) = &rec.commit {
        let head = run(repo, &["rev-parse", "HEAD"]).map(|h| h.trim().to_string()).unwrap_or_default();
        let count = run(repo, &["rev-list", "--count", "HEAD"]).ok().and_then(|c| c.trim().parse::<u64>().ok()).unwrap_or(0);
        if &head != commit || count != 1 {
            return Err("commits were made after the setup, so it cannot be undone from here".to_string());
        }
    }
    let there: Vec<String> = rec.entries.iter().filter(|n| std::fs::symlink_metadata(repo.join(n)).is_ok()).cloned().collect();
    if there.is_empty() {
        return Err("nothing left to undo: the files of the setup are already gone".to_string());
    }
    Ok(there)
}

/// The question to ask before undoing the last setup, or why it cannot be undone.
pub fn undo_question(repo: &Path, rec: &SetupRecord) -> Result<String, String> {
    let there = undo_guard(repo, rec)?;
    let files: usize = there.iter().map(|n| count_files(&repo.join(n))).sum();
    let shown: Vec<&str> = there.iter().take(4).map(String::as_str).collect();
    let more = if there.len() > shown.len() { ", …" } else { "" };
    let mut q = format!("undo the setup: delete {} files ({}{more})", files, shown.join(", "));
    if rec.commit.is_some() {
        q.push_str(" and the setup commit (its hash is logged, so it can be brought back)");
        let changed = run(repo, &["status", "--porcelain"]).map_or(0, |s| s.lines().count());
        if changed > 0 {
            q.push_str(&format!(", and {changed} change(s) made since"));
        }
    } else {
        q.push_str("; edits made to them since are lost too");
    }
    Ok(q)
}

/// Removes what the last setup made, leaving the repository empty as it was.
fn undo_setup(repo: &Path) -> Result<String, String> {
    let rec = setup_record(repo).ok_or("no setup was recorded for this repository")?;
    let there = undo_guard(repo, &rec)?;
    let mut lines = Vec::new();
    let mut recovery = None;
    if let Some(commit) = &rec.commit {
        run_all(repo, &["update-ref", "-d", "HEAD"])?;
        // The commit object stays in the repository until git cleans up, so it can be brought back.
        let short = &commit[..commit.len().min(10)];
        recovery = Some(format!("removed the setup commit {short} (git branch rescue {short} brings it back until git gc)"));
    }
    // Drop the files from the index first; untracked ones are simply not there.
    let mut args = vec!["rm", "-r", "--cached", "--ignore-unmatch", "-q", "--"];
    args.extend(there.iter().map(String::as_str));
    let _ = run_all(repo, &args);
    for name in &there {
        let path = repo.join(name);
        let removed = match std::fs::symlink_metadata(&path) {
            Ok(m) if m.is_dir() => std::fs::remove_dir_all(&path),
            Ok(_) => std::fs::remove_file(&path),
            Err(_) => Ok(()),
        };
        removed.map_err(|e| format!("cannot remove {name}: {e}"))?;
        lines.push(format!("removed {name}"));
    }
    if let Some(p) = record_path(repo) {
        let _ = std::fs::remove_file(p);
    }
    lines.extend(recovery); // last, so it is what the log panel still shows
    Ok(lines.join("\n"))
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
        Op::Setup { plan, commit } => setup(repo, plan, *commit),
        Op::UndoSetup => undo_setup(repo),
        Op::NewWorkflow { file, text, publish, message, dispatch } => new_workflow(repo, file, text, *publish, message, dispatch.as_ref()),
        Op::Dispatch { file, git_ref, .. } if file.is_empty() || file.starts_with('-') || git_ref.starts_with('-') => {
            Err(format!("invalid workflow '{file}' or ref '{git_ref}'"))
        }
        Op::Dispatch { slug, file, git_ref, inputs } => {
            let pairs: Vec<String> = inputs.iter().map(|(k, v)| format!("{k}={v}")).collect();
            let mut args = vec!["workflow", "run", file.as_str(), "-R", slug.as_str()];
            if !git_ref.is_empty() {
                args.extend(["--ref", git_ref.as_str()]);
            }
            for p in &pairs {
                args.extend(["-f", p.as_str()]);
            }
            crate::github::gh_all(Some(repo), &args)
        }
        Op::Tag { name, .. } | Op::PushTag(name) | Op::DeleteTag { name, .. } | Op::Release { tag: name, .. } if name.is_empty() || name.starts_with('-') || name.contains(char::is_whitespace) => {
            Err(format!("invalid tag name '{name}'"))
        }
        Op::Tag { name, message } if message.is_empty() => run_all(repo, &["tag", name]),
        Op::Tag { name, message } => run_all(repo, &["tag", "-a", name, "-m", message]),
        Op::PushTag(name) => {
            let remote = default_remote(repo).ok_or("no remote to push to")?;
            run_all(repo, &["push", &remote, &format!("refs/tags/{name}")])
        }
        Op::DeleteTag { name, remote } => {
            let mut out = String::new();
            if *remote {
                let r = default_remote(repo).ok_or("no remote to delete from")?;
                out = run_all(repo, &["push", &r, &format!(":refs/tags/{name}")])?;
            }
            let local = run_all(repo, &["tag", "-d", name]);
            // A tag that only exists on the remote is not an error when it was deleted there.
            match (local, *remote) {
                (Ok(l), _) => Ok(format!("{out}\n{l}").trim().to_string()),
                (Err(_), true) => Ok(out),
                (Err(e), false) => Err(e),
            }
        }
        // `--verify-tag` stops gh from creating the tag on GitHub's default branch when it was never pushed.
        Op::Release { tag, slug, draft } => {
            let mut args = vec!["release", "create", tag, "-R", slug, "--generate-notes", "--verify-tag"];
            if *draft {
                args.push("--draft");
            }
            crate::github::gh_all(Some(repo), &args)
        }
        Op::Stash => run_all(repo, &["stash", "push", "--include-untracked", "-m", "oxide stash"]),
        Op::StashPop => run_all(repo, &["stash", "pop"]),
        Op::Init => {
            std::fs::create_dir_all(repo).map_err(|e| format!("cannot create {}: {e}", repo.display()))?;
            run_all(repo, &["init"])
        }
        Op::Switch(name, _) | Op::NewBranch(name) | Op::DeleteBranch(name, _) if name.is_empty() || name.starts_with('-') => {
            Err(format!("invalid branch name '{name}'"))
        }
        Op::Switch(name, true) => run_all(repo, &["switch", "--track", name]),
        Op::Switch(name, false) => run_all(repo, &["switch", name]),
        Op::NewBranch(name) => run_all(repo, &["switch", "-c", name]),
        Op::CheckoutPr(n) => crate::github::checkout(repo, *n),
        Op::Clone(slug) if slug.starts_with('-') => Err(format!("invalid repository '{slug}'")),
        Op::Clone(_) if repo.read_dir().is_ok_and(|mut d| d.next().is_some()) => Err(format!("{} already exists and is not empty", repo.display())),
        Op::Clone(slug) => crate::github::clone(slug, repo),
        Op::Delete => delete_repo(repo),
        Op::AddWorktree { branch, .. } if branch.is_empty() || branch.starts_with('-') => Err(format!("invalid branch name '{branch}'")),
        Op::AddWorktree { path, branch } => {
            let path = path.display().to_string();
            let exists = run(repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).is_ok();
            // An existing branch is checked out as it is; a new name creates the branch from HEAD.
            if exists { run_all(repo, &["worktree", "add", &path, branch]) } else { run_all(repo, &["worktree", "add", "-b", branch, &path]) }
        }
        Op::RemoveWorktree { path, force } => {
            let path = path.display().to_string();
            if *force { run_all(repo, &["worktree", "remove", "--force", &path]) } else { run_all(repo, &["worktree", "remove", &path]) }
        }
        Op::PruneWorktrees => run_all(repo, &["worktree", "prune", "--verbose"]),
        Op::UpdateSubmodule(p) if p.is_empty() => run_all(repo, &["submodule", "update", "--init", "--recursive"]),
        Op::UpdateSubmodule(p) => run_all(repo, &["submodule", "update", "--init", "--recursive", "--", p]),
        // Names and URLs that git could read as options are refused before they reach it.
        Op::RemoteAdd { name: a, url: b } | Op::RemoteSetUrl { name: a, url: b } | Op::RemoteRename { old: a, new: b }
            if [a, b].iter().any(|v| v.is_empty() || v.starts_with('-') || (v == &a && v.contains(char::is_whitespace))) =>
        {
            Err(format!("invalid remote name or URL ('{a}', '{b}')"))
        }
        Op::RemoteRemove(n) | Op::RemoteFetch(n) if n.is_empty() || n.starts_with('-') => Err(format!("invalid remote name '{n}'")),
        Op::RemoteAdd { name, url } => run_all(repo, &["remote", "add", name, url]),
        Op::RemoteSetUrl { name, url } => run_all(repo, &["remote", "set-url", name, url]),
        Op::RemoteRename { old, new } => run_all(repo, &["remote", "rename", old, new]),
        Op::RemoteRemove(name) => run_all(repo, &["remote", "remove", name]),
        Op::RemoteFetch(name) => run_all(repo, &["fetch", "--prune", name]),
        Op::Amend(msg) if msg.is_empty() => run_all(repo, &["commit", "--amend", "--no-edit"]),
        Op::Amend(msg) => run_all(repo, &["commit", "--amend", "-m", msg]),
        Op::Reset(rev) | Op::Checkout(rev) if rev.is_empty() || rev.starts_with('-') => Err(format!("invalid target '{rev}'")),
        Op::Reset(rev) => run_all(repo, &["reset", "--keep", rev]),
        Op::Checkout(rev) => run_all(repo, &["checkout", rev, "--"]),
        Op::Hunk { patch, reverse } => apply_cached(repo, patch, *reverse),
        Op::DeleteBranch(name, force) => run_all(repo, &["branch", if *force { "-D" } else { "-d" }, "--", name]),
    }
}

/// Applies `patch` (a single-file unified diff, fed through stdin) to the index only. `--recount` lets git
/// trust the hunk body over its header counts; `reverse` undoes a patch that is already in the index.
fn apply_cached(repo: &Path, patch: &str, reverse: bool) -> Result<String, String> {
    use std::io::Write;
    let mut args = vec!["apply", "--cached", "--recount", "--whitespace=nowarn"];
    if reverse {
        args.push("-R");
    }
    args.push("-");
    let mut child = command()
        .arg("-C")
        .arg(repo)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run git: {e}"))?;
    // Dropping the handle closes stdin, which tells git the patch is complete.
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(patch.as_bytes()).map_err(|e| format!("cannot send the patch to git: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("cannot run git: {e}"))?;
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if out.status.success() { Ok(err) } else { Err(err) }
}

/// Removes a repository's folder from disk. Besides the prompt in the UI, this refuses anything that is
/// not a plain, standalone repository folder: symlinks (which could lead anywhere), worktrees and
/// submodules (whose `.git` is a file), and any folder that is, or contains, the home directory.
fn delete_repo(path: &Path) -> Result<String, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if meta.file_type().is_symlink() {
        return Err(format!("{} is a symlink; delete it by hand", path.display()));
    }
    if !path.join(".git").is_dir() {
        return Err(format!("{} is not a standalone repository (worktree or submodule); delete it by hand", path.display()));
    }
    let real = path.canonicalize().map_err(|e| format!("cannot resolve {}: {e}", path.display()))?;
    let home = crate::config::home().canonicalize().unwrap_or_default();
    if real.parent().is_none() || home.starts_with(&real) {
        return Err(format!("refusing to delete {}: it is, or contains, your home directory", real.display()));
    }
    std::fs::remove_dir_all(&real).map_err(|e| format!("cannot delete {}: {e}", real.display()))?;
    Ok(format!("deleted {}", real.display()))
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
