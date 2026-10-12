//! GitHub integration through the `gh` CLI (https://cli.github.com).
//!
//! Oxide never handles tokens itself: `gh` is already logged in, speaks to the API and follows the
//! user's protocol settings. Lists are requested with `--json` plus a `--jq` filter that prints one
//! tab-separated row per item, so no JSON parser is needed.

use std::{
    io,
    path::Path,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

/// What a list shows.
#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Issues,
    Pulls,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Issues => "Issues",
            Kind::Pulls => "Pull requests",
        }
    }

    pub fn other(self) -> Kind {
        match self {
            Kind::Issues => Kind::Pulls,
            Kind::Pulls => Kind::Issues,
        }
    }

    /// The `gh` subcommand.
    fn command(self) -> &'static str {
        match self {
            Kind::Issues => "issue",
            Kind::Pulls => "pr",
        }
    }
}

/// Which items a list shows.
#[derive(Clone, Copy, PartialEq)]
pub enum State {
    Open,
    Closed,
    /// Pull requests only.
    Merged,
    All,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Open => "open",
            State::Closed => "closed",
            State::Merged => "merged",
            State::All => "all",
        }
    }

    /// The next state in the cycle; issues have no merged state.
    pub fn next(self, kind: Kind) -> State {
        match (self, kind) {
            (State::Open, _) => State::Closed,
            (State::Closed, Kind::Pulls) => State::Merged,
            (State::Closed | State::Merged, _) => State::All,
            (State::All, _) => State::Open,
        }
    }
}

/// One issue or pull request in a list.
pub struct Item {
    pub number: u64,
    pub title: String,
    pub author: String,
    /// Comma-separated label names (issues).
    pub labels: String,
    /// Time since the last update, e.g. `3d`.
    pub age: String,
    pub draft: bool,
    /// Source branch (pull requests).
    pub branch: String,
    /// `APPROVED`, `CHANGES_REQUESTED`, `REVIEW_REQUIRED` or empty (pull requests).
    pub review: String,
}

/// Runs `gh`; returns `(stdout, stderr)`, or its error message when it fails. Prompts are disabled
/// and colors are off, so the output is plain text and a prompt can never take over the terminal.
/// `tty_width` makes gh format its output as for a terminal of that width (needed for `view`).
fn exec(cwd: Option<&Path>, args: &[&str], tty_width: Option<u16>) -> Result<(String, String), String> {
    let mut cmd = Command::new("gh");
    cmd.args(args).env("GH_PROMPT_DISABLED", "1").env("NO_COLOR", "1").stdin(Stdio::null());
    if let Some(w) = tty_width {
        cmd.env("GH_FORCE_TTY", w.to_string());
    }
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let out = cmd.output().map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => "gh is not installed (https://cli.github.com)".to_string(),
        _ => format!("cannot run gh: {e}"),
    })?;
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).trim().to_string());
    if out.status.success() {
        Ok((stdout, stderr))
    } else {
        Err(if stderr.is_empty() { "gh failed".to_string() } else { stderr })
    }
}

/// Runs `gh` and returns stdout.
pub fn gh(cwd: Option<&Path>, args: &[&str]) -> Result<String, String> {
    exec(cwd, args, None).map(|(out, _)| out)
}

/// Runs `gh` and returns stdout plus stderr, where it reports what it did (for the log).
pub fn gh_all(cwd: Option<&Path>, args: &[&str]) -> Result<String, String> {
    exec(cwd, args, None).map(|(out, err)| format!("{}\n{err}", out.trim()).trim().to_string())
}

/// `owner/repo` of a github.com remote URL (ssh, https or git protocol), or None for any other host.
pub fn slug(remote: &str) -> Option<String> {
    let host = remote.find("github.com")?;
    // The host must start at the beginning of the authority: `@github.com`, `//github.com` ...
    let before = remote[..host].chars().last();
    if !(host == 0 || matches!(before, Some('@' | '/'))) {
        return None;
    }
    let rest = remote[host + "github.com".len()..].strip_prefix([':', '/'])?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, repo) = rest.split_once('/')?;
    (!owner.is_empty() && !repo.is_empty() && !repo.contains('/')).then(|| format!("{owner}/{repo}"))
}

/// Replaces the escapes `@tsv` writes inside a field with plain characters.
fn untsv(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t' | 'n') => out.push(' '),
            Some('r') => {}
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// Days since 1970-01-01 of a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// How long ago an ISO timestamp (`2026-10-12T05:55:01Z`) was, in one short unit: `5m`, `3h`, `2d`, `4mo`, `1y`.
pub fn ago(iso: &str) -> String {
    let num = |range: std::ops::Range<usize>| iso.get(range).and_then(|s| s.parse::<i64>().ok());
    let (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(s)) = (num(0..4), num(5..7), num(8..10), num(11..13), num(14..16), num(17..19)) else {
        return String::new();
    };
    let then = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |t| t.as_secs() as i64);
    match (now - then).max(0) {
        s if s < 3600 => format!("{}m", (s / 60).max(1)),
        s if s < 86_400 => format!("{}h", s / 3600),
        s if s < 30 * 86_400 => format!("{}d", s / 86_400),
        s if s < 365 * 86_400 => format!("{}mo", s / (30 * 86_400)),
        s => format!("{}y", s / (365 * 86_400)),
    }
}

/// The newest 100 issues or pull requests of `slug` in the given state.
pub fn list(slug: &str, kind: Kind, state: State) -> Result<Vec<Item>, String> {
    // Every row has the same eight columns, whichever kind it is.
    let (fields, jq) = match kind {
        Kind::Issues => (
            "number,title,author,labels,updatedAt",
            r#".[]|[.number,.title,(.author.login//""),([.labels[].name]|join(",")),.updatedAt,false,"",""]|@tsv"#,
        ),
        Kind::Pulls => (
            "number,title,author,isDraft,headRefName,reviewDecision,updatedAt",
            r#".[]|[.number,.title,(.author.login//""),"",.updatedAt,.isDraft,.headRefName,(.reviewDecision//"")]|@tsv"#,
        ),
    };
    let out = gh(None, &[kind.command(), "list", "-R", slug, "--state", state.name(), "--limit", "100", "--json", fields, "--jq", jq])?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let f: Vec<String> = l.split('\t').map(untsv).collect();
            let [number, title, author, labels, updated, draft, branch, review] = &f[..] else { return None };
            Some(Item {
                number: number.parse().ok()?,
                title: title.clone(),
                author: author.clone(),
                labels: labels.clone(),
                age: ago(updated),
                draft: draft == "true",
                branch: branch.clone(),
                review: review.clone(),
            })
        })
        .collect())
}

/// Arguments for `gh` that open an item (or, with `number` None, the repository) in the web browser.
pub fn browse_args(slug: &str, kind: Kind, number: Option<u64>) -> Vec<String> {
    match number {
        Some(n) => vec![kind.command().into(), "view".into(), n.to_string(), "-R".into(), slug.into(), "--web".into()],
        None => vec!["browse".into(), "-R".into(), slug.into()],
    }
}

/// An issue or pull request as text: header, description and comments, formatted for `width` columns.
pub fn view(slug: &str, kind: Kind, number: u64, width: u16) -> Result<String, String> {
    let n = number.to_string();
    exec(None, &[kind.command(), "view", &n, "-R", slug, "--comments"], Some(width)).map(|(out, _)| out)
}

/// The unified diff of a pull request.
pub fn pr_diff(slug: &str, number: u64) -> Result<String, String> {
    gh(None, &["pr", "diff", &number.to_string(), "-R", slug, "--color", "never"])
}

/// Checks out a pull request's branch in the repository at `repo`.
pub fn checkout(repo: &Path, number: u64) -> Result<String, String> {
    gh_all(Some(repo), &["pr", "checkout", &number.to_string()])
}
