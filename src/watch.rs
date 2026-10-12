//! File watching through Linux inotify, without any extra crate: the few libc calls std already links are
//! declared here directly.
//!
//! Two kinds of places are watched. The `.git` directory of every repository (its `index`, `HEAD`,
//! `FETCH_HEAD`, branch refs ...) reveals commits, checkouts, fetches and pushes made outside Oxide.
//! The working tree of the *selected* repository is watched recursively as well, so edits made in an
//! editor show up in the Changes list at once. Watching every working tree on the machine would use up
//! the inotify watch limit, which is why only one tree is watched at a time.
//!
//! A reader thread turns events into `Msg::Changed(repo)`; the app debounces them and reloads.

use std::{
    collections::HashMap,
    ffi::{CString, c_char, c_int},
    fs::{self, File},
    io::{self, Read},
    os::{fd::FromRawFd, unix::ffi::OsStrExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc::Sender},
    thread,
};

use crate::{repo, worker::Msg};

unsafe extern "C" {
    fn inotify_init1(flags: c_int) -> c_int;
    fn inotify_add_watch(fd: c_int, path: *const c_char, mask: u32) -> c_int;
    fn inotify_rm_watch(fd: c_int, wd: c_int) -> c_int;
}

const IN_CLOSE_WRITE: u32 = 0x8;
const IN_MOVED_FROM: u32 = 0x40;
const IN_MOVED_TO: u32 = 0x80;
const IN_CREATE: u32 = 0x100;
const IN_DELETE: u32 = 0x200;
const IN_IGNORED: u32 = 0x8000;
const IN_ONLYDIR: u32 = 0x0100_0000;
const IN_ISDIR: u32 = 0x4000_0000;
/// `O_CLOEXEC`: editors and shells started from Oxide must not inherit the descriptor.
const IN_CLOEXEC: c_int = 0x80000;

/// Files inside `.git` whose change means the repository's state changed.
const GIT_FILES: &[&str] = &["index", "HEAD", "FETCH_HEAD", "ORIG_HEAD", "MERGE_HEAD", "CHERRY_PICK_HEAD", "REBASE_HEAD", "packed-refs", "COMMIT_EDITMSG"];

/// Directories that are never watched inside a working tree, besides `repo::SKIP` and `.git`.
const TREE_SKIP: &[&str] = &[".git", ".venv", ".cache", ".next", ".idea", ".mypy_cache", ".pytest_cache", ".tox", ".gradle"];

/// Upper bound of directories watched for one working tree, far below the usual inotify limit.
const MAX_TREE_DIRS: usize = 4096;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// The `.git` directory itself: only `GIT_FILES` count.
    Git,
    /// `refs/heads` and the remotes' ref folders: any branch change counts.
    Refs,
    /// A directory of the watched working tree.
    Tree,
}

struct Entry {
    repo: PathBuf,
    dir: PathBuf,
    kind: Kind,
}

#[derive(Default)]
struct Shared {
    by_wd: HashMap<c_int, Entry>,
    /// Descriptors of the watched working tree, to drop them when another repository is selected.
    tree: Vec<c_int>,
}

#[derive(Clone)]
pub struct Watcher {
    fd: c_int,
    shared: Arc<Mutex<Shared>>,
}

fn mask(kind: Kind) -> u32 {
    let base = IN_CLOSE_WRITE | IN_MOVED_TO | IN_CREATE | IN_DELETE | IN_ONLYDIR;
    if kind == Kind::Tree { base | IN_MOVED_FROM } else { base }
}

fn add(fd: c_int, dir: &Path, mask: u32) -> io::Result<c_int> {
    let path = CString::new(dir.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: `path` is a valid NUL-terminated string that outlives the call.
    let wd = unsafe { inotify_add_watch(fd, path.as_ptr(), mask) };
    if wd < 0 { Err(io::Error::last_os_error()) } else { Ok(wd) }
}

/// The directory that holds `index` and `HEAD`: `.git`, or the target of the `gitdir:` file of a worktree or submodule.
fn git_dir(repo: &Path) -> Option<PathBuf> {
    let dot_git = repo.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let text = fs::read_to_string(&dot_git).ok()?;
    Some(repo.join(text.trim().strip_prefix("gitdir:")?.trim()))
}

/// Watches `dir` and every directory below it (up to the budget), recording them in `shared`.
fn add_tree(fd: c_int, shared: &mut Shared, repo: &Path, dir: &Path) -> io::Result<()> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if shared.tree.len() >= MAX_TREE_DIRS {
            break;
        }
        let wd = add(fd, &dir, mask(Kind::Tree))?;
        shared.tree.push(wd);
        shared.by_wd.insert(wd, Entry { repo: repo.to_path_buf(), dir: dir.clone(), kind: Kind::Tree });
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // `file_type` does not follow symlinks, so linked directories are not entered.
            if entry.file_type().is_ok_and(|t| t.is_dir()) && !TREE_SKIP.contains(&&*name) && !repo::SKIP.contains(&&*name) {
                stack.push(entry.path());
            }
        }
    }
    Ok(())
}

impl Watcher {
    /// Starts the reader thread; fails when inotify is not available.
    pub fn start(tx: Sender<Msg>) -> io::Result<Watcher> {
        // SAFETY: plain syscall wrapper without pointers.
        let fd = unsafe { inotify_init1(IN_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let shared = Arc::new(Mutex::new(Shared::default()));
        // SAFETY: `fd` is a freshly created descriptor that nothing else owns; the reader thread keeps it for good.
        let reader = unsafe { File::from_raw_fd(fd) };
        let sh = shared.clone();
        thread::spawn(move || read_loop(reader, fd, sh, tx));
        Ok(Watcher { fd, shared })
    }

    /// Watches the git directory and the branch refs of `repo`.
    pub fn watch_git(&self, repo: &Path) -> io::Result<()> {
        let Some(gitdir) = git_dir(repo) else { return Ok(()) };
        let mut dirs = vec![(gitdir.clone(), Kind::Git), (gitdir.join("refs/heads"), Kind::Refs)];
        if let Ok(remotes) = fs::read_dir(gitdir.join("refs/remotes")) {
            dirs.extend(remotes.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| (e.path(), Kind::Refs)));
        }
        let mut shared = self.shared.lock().unwrap();
        for (dir, kind) in dirs {
            match add(self.fd, &dir, mask(kind)) {
                Ok(wd) => {
                    shared.by_wd.insert(wd, Entry { repo: repo.to_path_buf(), dir, kind });
                }
                // A worktree's git directory has no refs of its own; only the main folder is required.
                Err(_) if kind == Kind::Refs => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Moves the working-tree watch to `repo`, dropping the previous one. Returns how many directories are watched.
    pub fn watch_tree(&self, repo: &Path) -> io::Result<usize> {
        let mut shared = self.shared.lock().unwrap();
        for wd in std::mem::take(&mut shared.tree) {
            // SAFETY: plain syscall wrapper; an already removed descriptor just makes it fail.
            unsafe { inotify_rm_watch(self.fd, wd) };
            shared.by_wd.remove(&wd);
        }
        add_tree(self.fd, &mut shared, repo, repo)?;
        Ok(shared.tree.len())
    }
}

/// Reads events forever and reports the repositories they belong to.
fn read_loop(mut file: File, fd: c_int, shared: Arc<Mutex<Shared>>, tx: Sender<Msg>) {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match file.read(&mut buf) {
            Ok(n) if n > 0 => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            _ => return,
        };
        let (mut changed, mut new_dirs): (Vec<PathBuf>, Vec<(PathBuf, PathBuf)>) = (Vec::new(), Vec::new());
        let mut at = 0;
        // Each event is a 16-byte header (wd, mask, cookie, name length) followed by the NUL-padded name.
        while at + 16 <= n {
            let word = |i: usize| u32::from_ne_bytes(buf[at + i..at + i + 4].try_into().unwrap_or_default());
            let (wd, ev, len) = (word(0) as c_int, word(4), word(12) as usize);
            let name = buf.get(at + 16..(at + 16 + len).min(n)).unwrap_or_default();
            let name = String::from_utf8_lossy(name.split(|&b| b == 0).next().unwrap_or_default()).into_owned();
            at += 16 + len;

            let mut shared = shared.lock().unwrap();
            if ev & IN_IGNORED != 0 {
                shared.by_wd.remove(&wd); // the directory was deleted or unmounted
                continue;
            }
            let Some(entry) = shared.by_wd.get(&wd) else { continue };
            let relevant = match entry.kind {
                Kind::Git => GIT_FILES.contains(&name.as_str()),
                Kind::Refs => !name.ends_with(".lock"),
                Kind::Tree => !(ev & IN_ISDIR != 0 && (TREE_SKIP.contains(&name.as_str()) || repo::SKIP.contains(&name.as_str()))),
            };
            if !relevant {
                continue;
            }
            if entry.kind == Kind::Tree && ev & IN_ISDIR != 0 && ev & (IN_CREATE | IN_MOVED_TO) != 0 {
                new_dirs.push((entry.repo.clone(), entry.dir.join(&name)));
            }
            if !changed.contains(&entry.repo) {
                changed.push(entry.repo.clone());
            }
        }
        // A directory created inside the watched tree needs its own watches.
        if !new_dirs.is_empty() {
            let mut shared = shared.lock().unwrap();
            for (repo, dir) in new_dirs {
                let _ = add_tree(fd, &mut shared, &repo, &dir);
            }
        }
        for repo in changed {
            if tx.send(Msg::Changed(repo)).is_err() {
                return;
            }
        }
    }
}
