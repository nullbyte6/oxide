//! Repository model: discovery on disk, status loading and owned/cloned classification.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    config::Config,
    git::{self, FileEntry},
};

/// Directories never searched for repositories (hidden directories are skipped as well).
const SKIP: &[&str] = &["node_modules", "target", "__pycache__", "venv", "snap"];

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
    pub owner: Owner,
    pub last_commit: String,
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
        walk(root, cfg.depth, &mut found);
    }
    found.sort();
    found.dedup();
    found
}

fn walk(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    // `.git` is a directory in normal clones and a file in worktrees and submodules.
    if dir.join(".git").exists() {
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
        if name.starts_with('.') || SKIP.contains(&&*name) {
            continue;
        }
        walk(&entry.path(), depth - 1, found);
    }
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
        remote,
        last_commit: String::new(),
        error: None,
    };
    match git::status(path) {
        Ok(st) => {
            repo.branch = st.branch;
            repo.upstream = st.upstream;
            repo.ahead = st.ahead;
            repo.behind = st.behind;
            repo.files = st.files;
            repo.last_commit = git::last_commit(path);
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
