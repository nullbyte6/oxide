//! Application state and input handling for the dashboard.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;

use crate::{
    config::Config,
    git::Op,
    logger::{self, Level, Logger},
    repo::{Owner, Repo},
    worker::{self, Msg},
};

/// Number of log entries kept for the in-app log panel.
const LOG_LINES: usize = 500;

#[derive(Clone, Copy, PartialEq)]
pub enum Filter {
    All,
    Owned,
    Cloned,
}

impl Filter {
    pub const ALL: [Filter; 3] = [Filter::All, Filter::Owned, Filter::Cloned];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }
}

/// Which pane receives navigation keys.
#[derive(Clone, Copy, PartialEq)]
pub enum Focus {
    Repos,
    Files,
}

/// What the text input line is collecting.
#[derive(Clone, Copy, PartialEq)]
pub enum InputKind {
    Commit,
    Merge,
    Search,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Normal,
    Input(InputKind),
    Help,
}

pub struct LogLine {
    pub time: String,
    pub level: Level,
    pub text: String,
}

pub struct App {
    pub cfg: Arc<Config>,
    pub repos: Vec<Repo>,
    /// Indices into `repos` that pass the current filter and search, in display order.
    pub visible: Vec<usize>,
    pub repo_state: ListState,
    pub file_state: ListState,
    pub focus: Focus,
    pub mode: Mode,
    pub filter: Filter,
    pub search: String,
    /// Text typed into the input line while in `Mode::Input`.
    pub input: String,
    pub log: Vec<LogLine>,
    /// `(loaded, total)` while a scan is running.
    pub scan: Option<(usize, usize)>,
    /// Repositories with a running operation, mapped to the operation label.
    pub busy: HashMap<PathBuf, &'static str>,
    /// Animation frame counter, advanced once per event-loop iteration.
    pub tick: usize,
    pub quit: bool,
    logger: Logger,
    scan_paths: HashSet<PathBuf>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

/// Moves a selection by `delta` inside `0..len`, clamping at both ends.
fn step(cur: Option<usize>, len: usize, delta: isize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let target = cur.unwrap_or(0) as isize + delta;
    Some(target.clamp(0, len as isize - 1) as usize)
}

impl App {
    pub fn new(cfg: Config) -> App {
        let (tx, rx) = mpsc::channel();
        App {
            cfg: Arc::new(cfg),
            repos: Vec::new(),
            visible: Vec::new(),
            repo_state: ListState::default(),
            file_state: ListState::default(),
            focus: Focus::Repos,
            mode: Mode::Normal,
            filter: Filter::All,
            search: String::new(),
            input: String::new(),
            log: Vec::new(),
            scan: None,
            busy: HashMap::new(),
            tick: 0,
            quit: false,
            logger: Logger::open(),
            scan_paths: HashSet::new(),
            tx,
            rx,
        }
    }

    pub fn log(&mut self, level: Level, text: impl Into<String>) {
        let text = text.into();
        self.logger.write(level, &text);
        self.log.push(LogLine { time: logger::clock(), level, text });
        if self.log.len() > LOG_LINES {
            self.log.remove(0);
        }
    }

    pub fn selected(&self) -> Option<&Repo> {
        let i = self.repo_state.selected()?;
        self.visible.get(i).map(|&r| &self.repos[r])
    }

    pub fn count(&self, filter: Filter) -> usize {
        self.repos
            .iter()
            .filter(|r| match filter {
                Filter::All => true,
                Filter::Owned => r.owner == Owner::Owned,
                Filter::Cloned => r.owner == Owner::Cloned,
            })
            .count()
    }

    // ---- background work ----

    pub fn start_scan(&mut self) {
        if self.scan.is_some() {
            self.log(Level::Warn, "a scan is already running");
            return;
        }
        let roots: Vec<String> = self.cfg.roots.iter().take(4).map(|r| r.display().to_string()).collect();
        let more = if self.cfg.roots.len() > 4 { ", …" } else { "" };
        self.log(Level::Info, format!("scanning {} roots: {}{more} (depth {})", self.cfg.roots.len(), roots.join(", "), self.cfg.depth));
        self.scan = Some((0, 0));
        worker::spawn_scan(self.cfg.clone(), self.tx.clone());
    }

    /// Starts `jobs` in the background; repositories that are already busy are skipped.
    fn run_ops(&mut self, jobs: Vec<(PathBuf, Op)>) {
        let mut accepted = Vec::new();
        for (path, op) in jobs {
            if self.busy.contains_key(&path) {
                self.log(Level::Warn, format!("{}: busy with {}", dir_name(&path), self.busy[&path]));
                continue;
            }
            self.busy.insert(path.clone(), op.label());
            accepted.push((path, op));
        }
        if !accepted.is_empty() {
            worker::spawn_ops(accepted, self.cfg.clone(), self.tx.clone());
        }
    }

    /// Applies every message the workers have sent since the last call.
    pub fn drain(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::ScanStarted(paths) => {
                    self.log(Level::Info, format!("found {} repositories", paths.len()));
                    self.scan = Some((0, paths.len()));
                    self.scan_paths = paths.into_iter().collect();
                }
                Msg::Repo(repo) => {
                    if let Some((done, _)) = &mut self.scan {
                        *done += 1;
                    }
                    match self.repos.iter_mut().find(|r| r.path == repo.path) {
                        Some(slot) => *slot = repo,
                        None => self.repos.push(repo),
                    }
                    self.rebuild();
                }
                Msg::ScanDone(ms) => {
                    self.scan = None;
                    // Drop repositories that no longer exist on disk.
                    let keep = &self.scan_paths;
                    self.repos.retain(|r| keep.contains(&r.path));
                    self.rebuild();
                    self.log(Level::Ok, format!("scan finished: {} repositories in {ms} ms", self.repos.len()));
                }
                Msg::Op { path, label, result } => {
                    self.busy.remove(&path);
                    let name = dir_name(&path);
                    match result {
                        Ok(out) => {
                            self.log(Level::Ok, format!("{name}: {label} ok"));
                            for line in out.lines().filter(|l| !l.trim().is_empty()).take(3) {
                                self.log(Level::Info, format!("  {}", line.trim()));
                            }
                        }
                        Err(err) => {
                            self.log(Level::Error, format!("{name}: {label} failed"));
                            for line in err.lines().filter(|l| !l.trim().is_empty()).take(6) {
                                self.log(Level::Error, format!("  {}", line.trim()));
                            }
                        }
                    }
                }
            }
        }
    }

    // ---- derived state ----

    /// Re-sorts the repositories and recomputes the visible list, keeping the selection on the same repository.
    fn rebuild(&mut self) {
        let previous = self.repo_state.selected().and_then(|i| self.visible.get(i)).map(|&r| self.repos[r].path.clone());
        self.repos.sort_by(|a, b| {
            a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.path.cmp(&b.path))
        });

        let query = self.search.to_lowercase();
        let visible: Vec<usize> = self
            .repos
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                let owner_ok = match self.filter {
                    Filter::All => true,
                    Filter::Owned => r.owner == Owner::Owned,
                    Filter::Cloned => r.owner == Owner::Cloned,
                };
                owner_ok
                    && (query.is_empty()
                        || r.name.to_lowercase().contains(&query)
                        || r.branch.to_lowercase().contains(&query)
                        || r.path.to_string_lossy().to_lowercase().contains(&query))
            })
            .map(|(i, _)| i)
            .collect();
        self.visible = visible;

        let same = previous.and_then(|p| self.visible.iter().position(|&i| self.repos[i].path == p));
        let fallback = step(self.repo_state.selected(), self.visible.len(), 0);
        self.repo_state.select(same.or(fallback));
        self.clamp_files();
    }

    /// Keeps the file selection inside the selected repository's file list.
    fn clamp_files(&mut self) {
        let len = self.selected().map_or(0, |r| r.files.len());
        self.file_state.select(step(self.file_state.selected(), len, 0));
    }

    // ---- input ----

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        match self.mode {
            Mode::Help => self.mode = Mode::Normal,
            Mode::Input(kind) => self.on_input_key(kind, key),
            Mode::Normal => self.on_normal_key(key),
        }
    }

    fn on_normal_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Esc => {
                if self.focus == Focus::Files {
                    self.focus = Focus::Repos;
                } else if !self.search.is_empty() {
                    self.search.clear();
                    self.rebuild();
                }
            }
            KeyCode::Tab | KeyCode::BackTab => self.toggle_focus(),
            KeyCode::Right | KeyCode::Char('l') => self.focus = Focus::Files,
            KeyCode::Left | KeyCode::Char('h') => self.focus = Focus::Repos,
            KeyCode::Down | KeyCode::Char('j') => self.move_sel(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_sel(-1),
            KeyCode::PageDown => self.move_sel(10),
            KeyCode::PageUp => self.move_sel(-10),
            KeyCode::Home | KeyCode::Char('g') => self.move_sel(-(1 << 30)),
            KeyCode::End | KeyCode::Char('G') => self.move_sel(1 << 30),
            KeyCode::Char('f') => self.set_filter(Filter::ALL[(self.filter.index() + 1) % Filter::ALL.len()]),
            KeyCode::Char('1') => self.set_filter(Filter::All),
            KeyCode::Char('2') => self.set_filter(Filter::Owned),
            KeyCode::Char('3') => self.set_filter(Filter::Cloned),
            KeyCode::Char('/') => self.open_input(InputKind::Search),
            KeyCode::Char('R') => self.start_scan(),
            KeyCode::Char('r') => self.refresh_selected(),
            KeyCode::Char('F') => self.op_selected(Op::Fetch),
            KeyCode::Char('A') => self.fetch_visible(),
            KeyCode::Char('p') => self.op_selected(Op::Pull),
            KeyCode::Char('P') => self.op_selected(Op::Push),
            KeyCode::Char('a') => self.op_selected(Op::StageAll),
            KeyCode::Char('u') => self.op_selected(Op::UnstageAll),
            KeyCode::Char('m') => {
                if self.selected().is_some() {
                    self.open_input(InputKind::Merge);
                }
            }
            KeyCode::Char('c') => self.open_commit(),
            KeyCode::Char(' ') => self.toggle_stage(),
            KeyCode::Enter => {
                if self.focus == Focus::Repos {
                    self.focus = Focus::Files;
                } else {
                    self.toggle_stage();
                }
            }
            _ => {}
        }
    }

    fn on_input_key(&mut self, kind: InputKind, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.input.clear();
                self.mode = Mode::Normal;
                if kind == InputKind::Search {
                    self.search.clear();
                    self.rebuild();
                }
            }
            KeyCode::Enter => self.submit(kind),
            KeyCode::Backspace => {
                self.input.pop();
                self.sync_search(kind);
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.clear();
                self.sync_search(kind);
            }
            KeyCode::Char(c) => {
                self.input.push(c);
                self.sync_search(kind);
            }
            _ => {}
        }
    }

    /// The search filter is applied live while typing.
    fn sync_search(&mut self, kind: InputKind) {
        if kind == InputKind::Search {
            self.search = self.input.clone();
            self.rebuild();
        }
    }

    fn open_input(&mut self, kind: InputKind) {
        self.input = if kind == InputKind::Search { self.search.clone() } else { String::new() };
        self.mode = Mode::Input(kind);
    }

    fn open_commit(&mut self) {
        match self.selected().map(|r| r.staged()) {
            None => {}
            Some(0) => self.log(Level::Warn, "nothing staged to commit (stage files with Space or a)"),
            Some(_) => self.open_input(InputKind::Commit),
        }
    }

    fn submit(&mut self, kind: InputKind) {
        self.mode = Mode::Normal;
        let text = std::mem::take(&mut self.input).trim().to_string();
        match kind {
            InputKind::Search => {} // already applied while typing
            InputKind::Commit if text.is_empty() => self.log(Level::Warn, "empty commit message, commit cancelled"),
            InputKind::Commit => self.op_selected(Op::Commit(text)),
            InputKind::Merge => self.op_selected(Op::Merge(text)),
        }
    }

    fn set_filter(&mut self, filter: Filter) {
        self.filter = filter;
        self.rebuild();
    }

    fn toggle_focus(&mut self) {
        self.focus = if self.focus == Focus::Repos { Focus::Files } else { Focus::Repos };
    }

    fn move_sel(&mut self, delta: isize) {
        match self.focus {
            Focus::Repos => {
                let next = step(self.repo_state.selected(), self.visible.len(), delta);
                if next != self.repo_state.selected() {
                    self.repo_state.select(next);
                    self.file_state.select(None); // a different repository: start at its first file
                    self.clamp_files();
                }
            }
            Focus::Files => {
                let len = self.selected().map_or(0, |r| r.files.len());
                self.file_state.select(step(self.file_state.selected(), len, delta));
            }
        }
    }

    // ---- actions on the selected repository ----

    fn op_selected(&mut self, op: Op) {
        if let Some(path) = self.selected().map(|r| r.path.clone()) {
            self.run_ops(vec![(path, op)]);
        }
    }

    /// Fetches every repository that passes the current filter.
    fn fetch_visible(&mut self) {
        let jobs: Vec<(PathBuf, Op)> = self.visible.iter().map(|&i| (self.repos[i].path.clone(), Op::Fetch)).collect();
        self.log(Level::Info, format!("fetching {} repositories", jobs.len()));
        self.run_ops(jobs);
    }

    /// Reloads only the selected repository.
    fn refresh_selected(&mut self) {
        if let Some(path) = self.selected().map(|r| r.path.clone()) {
            // A plain reload needs no git operation, so it bypasses `run_ops` and the busy marker.
            let tx = self.tx.clone();
            let cfg = self.cfg.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Msg::Repo(crate::repo::load(&path, &cfg)));
            });
        }
    }

    /// Stages the highlighted file if it has unstaged or untracked changes, otherwise unstages it.
    fn toggle_stage(&mut self) {
        let job = self.selected().and_then(|r| {
            let f = r.files.get(self.file_state.selected()?)?;
            let op = if f.unstaged() || f.untracked() { Op::Stage(f.path.clone()) } else { Op::Unstage(f.path.clone()) };
            Some((r.path.clone(), op))
        });
        if let Some(job) = job {
            self.run_ops(vec![job]);
        }
    }
}

fn dir_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}
