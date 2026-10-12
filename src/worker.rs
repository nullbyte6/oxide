//! Background threads: repository scans and git operations.
//! Results reach the UI thread through an mpsc channel, so drawing never blocks on git.

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    thread,
    time::Instant,
};

use crate::{
    config::Config,
    git::{self, DiffKind, Op},
    github,
    repo::{self, Repo},
};

/// Upper bound of concurrent git processes.
const WORKERS: usize = 8;

pub enum Msg {
    /// Discovery finished; carries every repository path that will be loaded.
    ScanStarted(Vec<PathBuf>),
    /// A freshly loaded repository state (from a scan or after an operation).
    Repo(Repo),
    /// The scan loaded everything; carries the elapsed milliseconds.
    ScanDone(u128),
    /// The diff text (or error) requested with the given id.
    Diff { id: u64, result: Result<String, String> },
    /// The commit log (or error) requested with the given id.
    History { id: u64, result: Result<Vec<git::Commit>, String> },
    /// The reflog (or error) requested with the given id.
    Reflog { id: u64, result: Result<Vec<git::ReflogEntry>, String> },
    /// Worktrees and submodules (or the error) requested with the given id.
    Trees { id: u64, result: Result<Vec<git::TreeItem>, String> },
    /// The branch list (or error) requested with the given id.
    Branches { id: u64, result: Result<Vec<git::Branch>, String> },
    /// The repository's folder was deleted from disk; drop it from the list.
    Removed(PathBuf),
    /// Repositories found on GitHub (or the error) requested with the given id.
    Repos { id: u64, result: Result<Vec<github::RepoItem>, String> },
    /// Issues or pull requests (or the error) requested with the given id.
    GitHub { id: u64, result: Result<Vec<github::Item>, String> },
    /// The text of an issue or pull request (or the error) requested with the given id.
    Text { id: u64, result: Result<String, String> },
    /// Something changed on disk in the repository (reported by the file watcher).
    Changed(PathBuf),
    /// The CI state of a branch; None when it has no runs.
    Ci { path: PathBuf, branch: String, state: Option<github::Ci> },
    /// `gh` cannot give CI states at all (not installed or not logged in); the message says why.
    CiFailed(String),
    /// The CI batch finished.
    CiDone,
    /// An operation finished.
    Op { path: PathBuf, label: &'static str, result: Result<String, String> },
}

/// Runs `f` over `items` on up to `WORKERS` threads and returns when all are done.
fn pool<T: Send>(items: Vec<T>, f: impl Fn(T) + Sync) {
    let threads = WORKERS.min(items.len());
    let queue = Mutex::new(VecDeque::from(items));
    thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    // Take the lock only for the pop, never while `f` runs.
                    let item = queue.lock().unwrap().pop_front();
                    let Some(item) = item else { break };
                    f(item);
                }
            });
        }
    });
}

/// Discovers repositories and loads their state in the background.
pub fn spawn_scan(cfg: Arc<Config>, tx: Sender<Msg>) {
    thread::spawn(move || {
        let started = Instant::now();
        let paths = repo::discover(&cfg);
        let _ = tx.send(Msg::ScanStarted(paths.clone()));
        pool(paths, |p| {
            let _ = tx.send(Msg::Repo(repo::load(&p, &cfg)));
        });
        let _ = tx.send(Msg::ScanDone(started.elapsed().as_millis()));
    });
}

/// Runs the operations in the background and reloads each affected repository afterwards.
pub fn spawn_ops(jobs: Vec<(PathBuf, Op)>, cfg: Arc<Config>, tx: Sender<Msg>) {
    thread::spawn(move || {
        pool(jobs, |(path, op)| {
            let result = git::perform(&path, &op);
            let failed_init = matches!(op, Op::Init | Op::Clone(_)) && result.is_err();
            let deleted = matches!(op, Op::Delete) && result.is_ok();
            let worktree = match &op {
                Op::AddWorktree { path, .. } if result.is_ok() => Some((path.clone(), true)),
                Op::RemoveWorktree { path, .. } if result.is_ok() => Some((path.clone(), false)),
                _ => None,
            };
            let _ = tx.send(Msg::Op { path: path.clone(), label: op.label(), result });
            // A new worktree is a repository of its own that joins the list; a removed one leaves it.
            match worktree {
                Some((wt, true)) => {
                    let _ = tx.send(Msg::Repo(repo::load(&wt, &cfg)));
                }
                Some((wt, false)) => {
                    let _ = tx.send(Msg::Removed(wt));
                }
                None => {}
            }
            if deleted {
                let _ = tx.send(Msg::Removed(path));
                return; // nothing left to reload
            }
            // A failed init or clone leaves no repository, so there is nothing to load (it would show as a broken one).
            if !failed_init {
                let _ = tx.send(Msg::Repo(repo::load(&path, &cfg)));
            }
        });
    });
}

/// Loads the CI state of each `(repository, GitHub slug, branch)` in the background.
pub fn spawn_ci(items: Vec<(PathBuf, String, String)>, tx: Sender<Msg>) {
    thread::spawn(move || {
        let stop = AtomicBool::new(false);
        pool(items, |(path, slug, branch)| {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            match github::ci_status(&slug, &branch) {
                Ok(state) => {
                    let _ = tx.send(Msg::Ci { path, branch, state });
                }
                Err(e) if github::is_fatal(&e) => {
                    // Only the first thread to see the failure reports it.
                    if !stop.swap(true, Ordering::Relaxed) {
                        let _ = tx.send(Msg::CiFailed(e));
                    }
                }
                // Other errors (no access, Actions disabled) only mean this repository has no CI to show.
                Err(_) => {
                    let _ = tx.send(Msg::Ci { path, branch, state: None });
                }
            }
        });
        let _ = tx.send(Msg::CiDone);
    });
}

/// Loads the diff of one file in the background; the id lets the app ignore stale answers.
pub fn spawn_diff(repo: PathBuf, file: String, kind: DiffKind, id: u64, tx: Sender<Msg>) {
    thread::spawn(move || {
        let _ = tx.send(Msg::Diff { id, result: git::diff(&repo, &file, kind) });
    });
}

/// Loads the commit log in the background.
pub fn spawn_history(repo: PathBuf, id: u64, tx: Sender<Msg>) {
    thread::spawn(move || {
        let _ = tx.send(Msg::History { id, result: git::log(&repo, 500) });
    });
}

/// Loads the reflog in the background.
pub fn spawn_reflog(repo: PathBuf, id: u64, tx: Sender<Msg>) {
    thread::spawn(move || {
        let _ = tx.send(Msg::Reflog { id, result: git::reflog(&repo, 300) });
    });
}

/// Loads worktrees and submodules in the background.
pub fn spawn_trees(repo: PathBuf, id: u64, tx: Sender<Msg>) {
    thread::spawn(move || {
        let _ = tx.send(Msg::Trees { id, result: git::trees(&repo) });
    });
}

/// Loads the branch list in the background.
pub fn spawn_branches(repo: PathBuf, id: u64, tx: Sender<Msg>) {
    thread::spawn(move || {
        let _ = tx.send(Msg::Branches { id, result: git::branches(&repo) });
    });
}

/// Runs `f` on a background thread and delivers the message it returns.
pub fn spawn_msg(tx: Sender<Msg>, f: impl FnOnce() -> Msg + Send + 'static) {
    thread::spawn(move || {
        let _ = tx.send(f());
    });
}
