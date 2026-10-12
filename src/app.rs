//! Application state and input handling for the dashboard.

use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;

use crate::{
    config::Config,
    diff::{self, Diff},
    git::{DiffKind, Op},
    logger::{self, Level, Logger},
    marks::{Mark, Marks},
    repo::{Health, Owner, Repo},
    worker::{self, Msg},
};

/// Number of log entries kept for the in-app log panel.
const LOG_LINES: usize = 500;

#[derive(Clone, Copy, PartialEq)]
pub enum Filter {
    All,
    Owned,
    Cloned,
    /// Repositories with local changes, errors or commits to push or pull.
    Attention,
}

impl Filter {
    pub const ALL: [Filter; 4] = [Filter::All, Filter::Owned, Filter::Cloned, Filter::Attention];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }

    pub fn name(self) -> &'static str {
        match self {
            Filter::All => "all",
            Filter::Owned => "owned",
            Filter::Cloned => "cloned",
            Filter::Attention => "attention",
        }
    }

    fn matches(self, r: &Repo) -> bool {
        match self {
            Filter::All => true,
            Filter::Owned => r.owner == Owner::Owned,
            Filter::Cloned => r.owner == Owner::Cloned,
            Filter::Attention => r.health() != Health::Clean || r.ahead > 0 || r.behind > 0,
        }
    }
}

/// Order of the repository list; pinned repositories always come first.
#[derive(Clone, Copy, PartialEq)]
pub enum Sort {
    Name,
    /// Most recent commit first.
    Recent,
    /// Conflicts, errors and the most changed files first.
    Changes,
    /// Most commits behind (then ahead) first.
    Behind,
}

impl Sort {
    const ALL: [Sort; 4] = [Sort::Name, Sort::Recent, Sort::Changes, Sort::Behind];

    pub fn name(self) -> &'static str {
        match self {
            Sort::Name => "name",
            Sort::Recent => "recent",
            Sort::Changes => "changes",
            Sort::Behind => "behind",
        }
    }
}

/// An external program that needs the terminal; the event loop suspends the dashboard to run it.
pub enum Launch {
    /// `$VISUAL` / `$EDITOR` on `target`, started inside `repo`.
    Editor { repo: PathBuf, target: PathBuf },
    /// `$SHELL` inside `repo`.
    Shell { repo: PathBuf },
}

impl Launch {
    pub fn repo(&self) -> &Path {
        match self {
            Launch::Editor { repo, .. } | Launch::Shell { repo } => repo,
        }
    }

    /// Runs the program in the foreground and waits for it.
    pub fn run(&self) -> Result<(), String> {
        let (mut cmd, name) = match self {
            Launch::Editor { target, .. } => {
                let line = ["VISUAL", "EDITOR"].iter().find_map(|v| std::env::var(v).ok().filter(|s| !s.trim().is_empty()));
                let line = line.unwrap_or_else(|| "vi".to_string());
                let mut parts = line.split_whitespace().map(str::to_string);
                let program = parts.next().unwrap_or_else(|| "vi".to_string());
                let mut cmd = Command::new(&program);
                cmd.args(parts).arg(target);
                (cmd, program)
            }
            Launch::Shell { .. } => {
                let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".to_string());
                eprintln!("oxide: shell in {} - exit it to return to the dashboard", self.repo().display());
                (Command::new(&shell), shell)
            }
        };
        cmd.current_dir(self.repo()).status().map(|_| ()).map_err(|e| format!("cannot run {name}: {e}"))
    }
}

/// The diff viewer: one file of one repository.
pub struct DiffView {
    pub repo: PathBuf,
    pub file: String,
    pub kind: DiffKind,
    /// None while the diff is loading.
    pub diff: Option<Diff>,
    pub error: Option<String>,
    /// First visible row, and how many columns the code is scrolled to the left.
    pub scroll: usize,
    pub hscroll: usize,
    /// Rows that fit on screen; set by the renderer, used for page jumps.
    pub height: usize,
    /// True when the file has both staged and unstaged changes, so `Tab` can switch between them.
    pub both: bool,
    id: u64,
}

/// A bulk operation waiting for a yes.
pub struct Confirm {
    pub prompt: String,
    jobs: Vec<(PathBuf, Op)>,
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
    /// Waiting for `y` to run `App::confirm`.
    Confirm,
    /// The diff viewer (`App::diff`) is open.
    Diff,
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
    pub sort: Sort,
    /// Show ignored repositories too (marked in the list).
    pub show_hidden: bool,
    /// Accent colors on top of the monochrome design (toggled with `C`).
    pub color: bool,
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
    pub confirm: Option<Confirm>,
    pub diff: Option<DiffView>,
    diff_id: u64,
    /// A terminal program requested by the user; taken and run by the event loop.
    pub launch: Option<Launch>,
    marks: Marks,
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
        let color = cfg.color;
        App {
            marks: Marks::load(&cfg),
            cfg: Arc::new(cfg),
            repos: Vec::new(),
            visible: Vec::new(),
            repo_state: ListState::default(),
            file_state: ListState::default(),
            focus: Focus::Repos,
            mode: Mode::Normal,
            filter: Filter::All,
            sort: Sort::Name,
            show_hidden: false,
            color,
            search: String::new(),
            input: String::new(),
            log: Vec::new(),
            scan: None,
            busy: HashMap::new(),
            tick: 0,
            quit: false,
            confirm: None,
            diff: None,
            diff_id: 0,
            launch: None,
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

    /// Repositories that count towards the tabs and totals: all of them, minus ignored ones unless shown.
    pub fn listed(&self) -> impl Iterator<Item = &Repo> {
        self.repos.iter().filter(|r| self.show_hidden || !r.ignored)
    }

    pub fn count(&self, filter: Filter) -> usize {
        self.listed().filter(|r| filter.matches(r)).count()
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
                Msg::Diff { id, result } => {
                    if let Some(view) = self.diff.as_mut().filter(|v| v.id == id) {
                        match result {
                            Ok(text) => view.diff = Some(diff::parse(&text, &view.file)),
                            Err(e) => view.error = Some(e),
                        }
                    }
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
        for r in &mut self.repos {
            r.pinned = self.marks.has(Mark::Pin, &r.path);
            r.ignored = self.marks.has(Mark::Ignore, &r.path);
        }
        let sort = self.sort;
        self.repos.sort_by(|a, b| compare(sort, a, b));

        let query = self.search.to_lowercase();
        let visible: Vec<usize> = self
            .repos
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                (self.show_hidden || !r.ignored)
                    && self.filter.matches(r)
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
            Mode::Confirm => self.on_confirm_key(key),
            Mode::Diff => self.on_diff_key(key),
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
            KeyCode::Char('4') => self.set_filter(Filter::Attention),
            KeyCode::Char('d') => self.open_diff(),
            KeyCode::Char('s') => self.cycle_sort(),
            KeyCode::Char('C') => {
                self.color = !self.color;
                self.log(Level::Info, if self.color { "colors on" } else { "colors off" });
            }
            KeyCode::Char('H') => {
                self.show_hidden = !self.show_hidden;
                self.rebuild();
            }
            KeyCode::Char('*') => self.toggle_mark(Mark::Pin),
            KeyCode::Char('x') => self.toggle_mark(Mark::Ignore),
            KeyCode::Char('z') => self.op_selected(Op::Stash),
            KeyCode::Char('Z') => self.op_selected(Op::StashPop),
            KeyCode::Char('L') => self.pull_visible(),
            KeyCode::Char('U') => self.push_visible(),
            KeyCode::Char('e') => self.open_editor(),
            KeyCode::Char('t') => self.open_shell(),
            KeyCode::Char('o') => self.open_files(),
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

    /// Opens the diff viewer on the highlighted file.
    fn open_diff(&mut self) {
        let Some(r) = self.selected() else { return };
        let Some(f) = self.file_state.selected().and_then(|i| r.files.get(i)) else {
            self.log(Level::Warn, "no changed file selected to diff");
            return;
        };
        let kind = if f.untracked() {
            DiffKind::Untracked
        } else if f.unstaged() {
            DiffKind::Unstaged
        } else {
            DiffKind::Staged
        };
        let both = f.staged() && f.unstaged() && !f.conflict();
        let (repo, file) = (r.path.clone(), f.path.clone());
        self.diff_id += 1;
        self.diff = Some(DiffView {
            repo: repo.clone(),
            file: file.clone(),
            kind,
            diff: None,
            error: None,
            scroll: 0,
            hscroll: 0,
            height: 20,
            both,
            id: self.diff_id,
        });
        self.mode = Mode::Diff;
        worker::spawn_diff(repo, file, kind, self.diff_id, self.tx.clone());
    }

    fn on_diff_key(&mut self, key: KeyEvent) {
        let Some(v) = self.diff.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        let page = v.height.saturating_sub(1).max(1);
        let max = v.diff.as_ref().map_or(0, |d| d.rows.len().saturating_sub(v.height));
        match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'd') => {
                self.diff = None;
                self.mode = Mode::Normal;
            }
            KeyCode::Down | KeyCode::Char('j') => v.scroll = (v.scroll + 1).min(max),
            KeyCode::Up | KeyCode::Char('k') => v.scroll = v.scroll.saturating_sub(1),
            KeyCode::PageDown | KeyCode::Char(' ') => v.scroll = (v.scroll + page).min(max),
            KeyCode::PageUp | KeyCode::Char('b') => v.scroll = v.scroll.saturating_sub(page),
            KeyCode::Home | KeyCode::Char('g') => v.scroll = 0,
            KeyCode::End | KeyCode::Char('G') => v.scroll = max,
            KeyCode::Right | KeyCode::Char('l') => v.hscroll += 8,
            KeyCode::Left | KeyCode::Char('h') => v.hscroll = v.hscroll.saturating_sub(8),
            KeyCode::Char('0') => v.hscroll = 0,
            KeyCode::Char(']' | 'n') => {
                if let Some(d) = &v.diff {
                    v.scroll = d.hunks.iter().copied().find(|&h| h > v.scroll).unwrap_or(v.scroll).min(max);
                }
            }
            KeyCode::Char('[' | 'N') => {
                if let Some(d) = &v.diff {
                    v.scroll = d.hunks.iter().copied().rev().find(|&h| h < v.scroll).unwrap_or(0);
                }
            }
            KeyCode::Tab | KeyCode::BackTab => {
                if v.both {
                    v.kind = if v.kind == DiffKind::Staged { DiffKind::Unstaged } else { DiffKind::Staged };
                    self.diff_id += 1;
                    v.id = self.diff_id;
                    (v.diff, v.error, v.scroll, v.hscroll) = (None, None, 0, 0);
                    worker::spawn_diff(v.repo.clone(), v.file.clone(), v.kind, v.id, self.tx.clone());
                }
            }
            _ => {}
        }
    }

    fn on_confirm_key(&mut self, key: KeyEvent) {
        self.mode = Mode::Normal;
        let Some(confirm) = self.confirm.take() else { return };
        if key.code == KeyCode::Char('y') {
            self.run_ops(confirm.jobs);
        } else {
            self.log(Level::Info, "cancelled");
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

    /// Pulls (fast-forward only) every repository in the current view that is behind its upstream.
    fn pull_visible(&mut self) {
        let jobs: Vec<(PathBuf, Op)> = self
            .visible
            .iter()
            .map(|&i| &self.repos[i])
            .filter(|r| r.behind > 0 && r.upstream.is_some() && r.error.is_none())
            .map(|r| (r.path.clone(), Op::Pull))
            .collect();
        if jobs.is_empty() {
            self.log(Level::Info, "no repository in this view is behind its upstream");
            return;
        }
        self.log(Level::Info, format!("pulling {} repositories", jobs.len()));
        self.run_ops(jobs);
    }

    /// Asks for confirmation, then pushes every repository in the current view that is ahead of its upstream.
    fn push_visible(&mut self) {
        let jobs: Vec<(PathBuf, Op)> = self
            .visible
            .iter()
            .map(|&i| &self.repos[i])
            .filter(|r| r.ahead > 0 && r.upstream.is_some() && r.error.is_none())
            .map(|r| (r.path.clone(), Op::Push))
            .collect();
        if jobs.is_empty() {
            self.log(Level::Info, "no repository in this view is ahead of its upstream");
            return;
        }
        let n = jobs.len();
        let prompt = format!("push {n} repositor{}?", if n == 1 { "y" } else { "ies" });
        self.confirm = Some(Confirm { prompt, jobs });
        self.mode = Mode::Confirm;
    }

    fn cycle_sort(&mut self) {
        let next = Sort::ALL[(Sort::ALL.iter().position(|s| *s == self.sort).unwrap_or(0) + 1) % Sort::ALL.len()];
        self.sort = next;
        self.rebuild();
    }

    /// Pins or ignores the selected repository (saved in the state file).
    fn toggle_mark(&mut self, mark: Mark) {
        let Some((path, name)) = self.selected().map(|r| (r.path.clone(), r.name.clone())) else { return };
        match self.marks.toggle(mark, &path) {
            Ok(on) => {
                let (word, undo) = match mark {
                    Mark::Pin => ("pinned", "unpinned"),
                    Mark::Ignore => ("ignored", "no longer ignored"),
                };
                self.log(Level::Info, format!("{name}: {}", if on { word } else { undo }));
                self.rebuild();
            }
            Err(e) => self.log(Level::Warn, format!("{name}: {e}")),
        }
    }

    /// Opens the highlighted file (or the repository when no file pane is focused) in the editor.
    fn open_editor(&mut self) {
        let Some(r) = self.selected() else { return };
        let file = if self.focus == Focus::Files { self.file_state.selected().and_then(|i| r.files.get(i)) } else { None };
        let target = file.map_or_else(|| r.path.clone(), |f| r.path.join(&f.path));
        self.launch = Some(Launch::Editor { repo: r.path.clone(), target });
    }

    fn open_shell(&mut self) {
        if let Some(r) = self.selected() {
            self.launch = Some(Launch::Shell { repo: r.path.clone() });
        }
    }

    /// Opens the repository folder in the desktop file manager, without leaving the dashboard.
    fn open_files(&mut self) {
        let Some((path, name)) = self.selected().map(|r| (r.path.clone(), r.name.clone())) else { return };
        let child = Command::new("xdg-open").arg(&path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        match child {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait(); // reap the process
                });
                self.log(Level::Info, format!("{name}: opened in the file manager"));
            }
            Err(e) => self.log(Level::Error, format!("{name}: cannot run xdg-open: {e}")),
        }
    }

    /// Called by the event loop after a `Launch` program exited.
    pub fn launch_done(&mut self, repo: &Path, result: Result<(), String>) {
        if let Err(e) = result {
            self.log(Level::Error, e);
        }
        self.reload(repo.to_path_buf()); // the program may have changed the working tree
    }

    /// Reloads only the selected repository.
    fn refresh_selected(&mut self) {
        if let Some(path) = self.selected().map(|r| r.path.clone()) {
            self.reload(path);
        }
    }

    fn reload(&self, path: PathBuf) {
        // A plain reload needs no git operation, so it bypasses `run_ops` and the busy marker.
        let tx = self.tx.clone();
        let cfg = self.cfg.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Repo(crate::repo::load(&path, &cfg)));
        });
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

/// Pinned repositories first, then the chosen sort order, then by name and path.
fn compare(sort: Sort, a: &Repo, b: &Repo) -> Ordering {
    b.pinned
        .cmp(&a.pinned)
        .then_with(|| match sort {
            Sort::Name => Ordering::Equal,
            Sort::Recent => b.commit_time.cmp(&a.commit_time),
            Sort::Changes => b.activity().cmp(&a.activity()),
            Sort::Behind => b.behind.cmp(&a.behind).then(b.ahead.cmp(&a.ahead)),
        })
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        .then_with(|| a.path.cmp(&b.path))
}

fn dir_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}
