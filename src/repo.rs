//! Repository model: discovery on disk, status loading and owned/cloned classification.

use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use crate::{
    config::Config,
    git::{self, FileEntry},
    github,
};

/// Directories never searched for repositories (hidden directories are skipped as well).
const SKIP: &[&str] = &["node_modules", "target", "__pycache__", "venv", "snap", "$RECYCLE.BIN", "System Volume Information"];

/// System directories skipped only directly under `/` (a user's own `dev` or `usr` folder is still scanned).
const ROOT_SKIP: &[&str] = &["proc", "sys", "dev", "run", "boot", "usr", "lost+found"];

#[derive(Clone, Copy, PartialEq)]
pub enum Owner {
    Owned,
    Cloned,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Health {
    Clean,
    Dirty,
    Conflict,
    Error,
}

#[derive(Clone)]
pub struct Repo {
    pub path: PathBuf,
    pub name: String,
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<FileEntry>,
    pub remote: Option<String>,
    /// `owner/repo` when the remote lives on github.com.
    pub slug: Option<String>,
    pub owner: Owner,
    pub last_commit: String,
    /// Unix time of the last commit; 0 when there is none.
    pub commit_time: i64,
    /// Number of entries in `git stash list`.
    pub stashes: u32,
    /// Set by the app from the pin marks; pinned repositories are listed first.
    pub pinned: bool,
    /// Set by the app from the ignore marks; ignored repositories are hidden unless asked for.
    pub ignored: bool,
    /// Set when `git status` failed; the repository is then shown as broken.
    pub error: Option<String>,
}

impl Repo {
    pub fn staged(&self) -> usize {
        self.files.iter().filter(|f| f.staged()).count()
    }
    pub fn unstaged(&self) -> usize {
        self.files.iter().filter(|f| f.unstaged() && !f.conflict()).count()
    }
    pub fn untracked(&self) -> usize {
        self.files.iter().filter(|f| f.untracked()).count()
    }
    pub fn conflicts(&self) -> usize {
        self.files.iter().filter(|f| f.conflict()).count()
    }

    /// Sort weight for "most changed first": conflicts, then errors, then the number of changed files.
    pub fn activity(&self) -> usize {
        let rank = match self.health() {
            Health::Conflict => 3,
            Health::Error => 2,
            Health::Dirty => 1,
            Health::Clean => 0,
        };
        rank * 1_000_000 + self.files.len()
    }

    pub fn health(&self) -> Health {
        if self.error.is_some() {
            Health::Error
        } else if self.conflicts() > 0 {
            Health::Conflict
        } else if self.files.is_empty() {
            Health::Clean
        } else {
            Health::Dirty
        }
    }
}

/// Finds every repository below the configured roots (a directory containing `.git`).
pub fn discover(cfg: &Config) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in &cfg.roots {
        // Remember the root's filesystem so other mounted drives (their own roots) are not entered twice.
        let dev = if cfg.cross_devices { None } else { fs::metadata(root).ok().map(|m| m.dev()) };
        walk(root, cfg.depth, dev, &mut found);
    }
    found.sort();
    found.dedup();
    found
}

fn walk(dir: &Path, depth: usize, dev: Option<u64>, found: &mut Vec<PathBuf>) {
    if has_git_dir(dir) {
        found.push(dir.to_path_buf());
        return; // nested repositories are not searched
    }
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        // `file_type` does not follow symlinks, so symlinked directories are skipped.
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || SKIP.contains(&&*name) || (dir == Path::new("/") && ROOT_SKIP.contains(&&*name)) {
            continue;
        }
        if dev.is_some_and(|d| entry.metadata().is_ok_and(|m| m.dev() != d)) {
            continue; // a different filesystem: it is scanned as its own root
        }
        walk(&entry.path(), depth - 1, dev, found);
    }
}

/// True when `dir/.git` is a usable repository. It is a directory holding `HEAD` in normal clones,
/// and a `gitdir: <path>` file in worktrees and submodules; the target of that file must exist.
/// Leftovers such as an empty `.git` folder or a submodule whose gitdir was removed are rejected.
fn has_git_dir(dir: &Path) -> bool {
    let dot_git = dir.join(".git");
    if dot_git.is_dir() {
        return dot_git.join("HEAD").exists();
    }
    let Ok(text) = fs::read_to_string(&dot_git) else { return false };
    text.trim().strip_prefix("gitdir:").is_some_and(|target| dir.join(target.trim()).exists())
}

/// Loads the full state of the repository at `path`.
pub fn load(path: &Path, cfg: &Config) -> Repo {
    let remote = git::remote_url(path);
    let mut repo = Repo {
        path: path.to_path_buf(),
        name: path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned()),
        branch: String::new(),
        upstream: None,
        ahead: 0,
        behind: 0,
        files: Vec::new(),
        owner: classify(remote.as_deref(), cfg),
        slug: remote.as_deref().and_then(github::slug),
        remote,
        last_commit: String::new(),
        commit_time: 0,
        stashes: 0,
        pinned: false,
        ignored: false,
        error: None,
    };
    match git::status(path) {
        Ok(st) => {
            repo.branch = st.branch;
            repo.upstream = st.upstream;
            repo.ahead = st.ahead;
            repo.behind = st.behind;
            repo.files = st.files;
            repo.stashes = st.stashes;
            (repo.last_commit, repo.commit_time) = git::last_commit(path);
        }
        Err(e) => repo.error = Some(e),
    }
    repo
}

/// Owned = no remote at all, a local-path remote, or a remote whose account is one of the owners.
fn classify(remote: Option<&str>, cfg: &Config) -> Owner {
    let Some(url) = remote else { return Owner::Owned };
    // The path part of the URL: `scheme://host/<path>` or scp-style `host:<path>`.
    let path = if url.starts_with("file://") || url.starts_with(['/', '.', '~']) {
        return Owner::Owned;
    } else if let Some((_, rest)) = url.split_once("://") {
        rest.split_once('/').map(|(_, p)| p)
    } else {
        url.split_once(':').map(|(_, p)| p)
    };
    let account = path.and_then(|p| p.trim_start_matches('/').split('/').next()).map(str::to_lowercase);
    match account {
        Some(a) if cfg.owners.contains(&a) => Owner::Owned,
        _ => Owner::Cloned,
    }
}
