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
    config::{self, Config},
    diff::{self, Diff},
    git::{Branch, Commit, DiffKind, Op},
    github::{self, Item, Kind, RepoItem, State},
    logger::{self, Level, Logger},
    marks::{Mark, Marks},
    palette,
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
    /// The yazi file manager inside `repo`, with `target` revealed. Files are then opened from yazi,
    /// with whatever program its rules choose (e.g. nvim). Without yazi, `$VISUAL` / `$EDITOR` opens `target`.
    Browse { repo: PathBuf, target: PathBuf },
    /// `$SHELL` inside `repo`.
    Shell { repo: PathBuf },
}

impl Launch {
    pub fn repo(&self) -> &Path {
        match self {
            Launch::Browse { repo, .. } | Launch::Shell { repo } => repo,
        }
    }

    /// Runs the program in the foreground and waits for it.
    pub fn run(&self) -> Result<(), String> {
        let (mut cmd, name) = match self {
            Launch::Browse { repo, target } => {
                return match Command::new("yazi").arg(target).current_dir(repo).status() {
                    Ok(_) => Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        run_editor(repo, target).map_err(|e| format!("yazi is not installed, and the editor failed: {e}"))
                    }
                    Err(e) => Err(format!("cannot run yazi: {e}")),
                };
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

/// Fallback when yazi is missing: `$VISUAL` / `$EDITOR` (default `vi`) on `target`.
fn run_editor(repo: &Path, target: &Path) -> Result<(), String> {
    let line = ["VISUAL", "EDITOR"].iter().find_map(|v| std::env::var(v).ok().filter(|s| !s.trim().is_empty()));
    let line = line.unwrap_or_else(|| "vi".to_string());
    let mut parts = line.split_whitespace().map(str::to_string);
    let program = parts.next().unwrap_or_else(|| "vi".to_string());
    Command::new(&program)
        .args(parts)
        .arg(target)
        .current_dir(repo)
        .status()
        .map(|_| ())
        .map_err(|e| format!("cannot run {program}: {e}"))
}

/// State of the command palette.
#[derive(Default)]
pub struct Palette {
    pub query: String,
    pub state: ListState,
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
    /// The mode to return to when the viewer closes (the dashboard, or the commit log).
    back: Mode,
    id: u64,
}

/// The commit log of one repository.
pub struct HistoryView {
    pub repo: PathBuf,
    pub name: String,
    pub branch: String,
    /// None while the log is loading.
    pub commits: Option<Vec<Commit>>,
    pub error: Option<String>,
    pub state: ListState,
    id: u64,
}

/// The branches of one repository.
pub struct BranchView {
    pub repo: PathBuf,
    pub name: String,
    pub branches: Option<Vec<Branch>>,
    pub error: Option<String>,
    pub state: ListState,
    id: u64,
}

impl BranchView {
    fn selected(&self) -> Option<&Branch> {
        self.branches.as_ref()?.get(self.state.selected()?)
    }
}

/// The issues or pull requests of one repository on GitHub.
pub struct GhView {
    pub repo: PathBuf,
    pub name: String,
    pub slug: String,
    pub kind: Kind,
    pub state: State,
    /// None while the list is loading.
    pub items: Option<Vec<Item>>,
    pub error: Option<String>,
    pub list: ListState,
    id: u64,
}

/// GitHub repositories found by a search, or your own, to clone.
pub struct FinderView {
    /// The search terms; empty for your own repositories.
    pub query: String,
    /// None while loading.
    pub items: Option<Vec<RepoItem>>,
    pub error: Option<String>,
    pub list: ListState,
    id: u64,
}

impl FinderView {
    fn selected(&self) -> Option<&RepoItem> {
        self.items.as_ref()?.get(self.list.selected()?)
    }
}

/// An issue or pull request as scrollable text, opened from the GitHub list.
pub struct TextView {
    pub title: String,
    /// None while loading.
    pub lines: Option<Vec<String>>,
    pub error: Option<String>,
    pub scroll: usize,
    /// Rows that fit on screen; set by the renderer.
    pub height: usize,
    /// Arguments for `gh` that open the same item in the browser.
    browse: Vec<String>,
    id: u64,
}

impl GhView {
    fn selected(&self) -> Option<&Item> {
        self.items.as_ref()?.get(self.list.selected()?)
    }
}

/// A destructive or bulk operation waiting for a yes.
pub struct Confirm {
    pub prompt: String,
    jobs: Vec<(PathBuf, Op)>,
    /// The mode to return to afterwards.
    pub back: Mode,
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
    /// Name of a new branch.
    Branch,
    /// Folder to run `git init` in.
    Init,
    /// Search terms for GitHub repositories (empty lists your own).
    GhSearch,
    /// Folder to clone the chosen GitHub repository into.
    CloneTo,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Normal,
    Input(InputKind),
    /// Waiting for `y` to run `App::confirm`.
    Confirm,
    /// The command palette is open.
    Palette,
    /// The diff viewer (`App::diff`) is open.
    Diff,
    /// The commit log (`App::history`) is open.
    History,
    /// The branch list (`App::branches`) is open.
    Branches,
    /// The GitHub issue or pull request list (`App::gh`) is open.
    GitHub,
    /// An issue or pull request opened as text (`App::text`).
    Text,
    /// GitHub repository search results (`App::finder`).
    Finder,
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
    /// The key bar at the bottom is visible (toggled with `h`).
    pub hints: bool,
    /// The command palette's query and selection.
    pub palette: Palette,
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
    pub history: Option<HistoryView>,
    pub branches: Option<BranchView>,
    pub gh: Option<GhView>,
    pub text: Option<TextView>,
    pub finder: Option<FinderView>,
    /// The repository chosen in the finder, waiting for a clone destination.
    clone_slug: Option<String>,
    /// Mode to return to when the input line closes (the dashboard, or the branch list).
    input_back: Mode,
    /// A repository just created with `git init`, selected as soon as it is loaded.
    pending_select: Option<PathBuf>,
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
        let (color, hints) = (cfg.color, cfg.hints);
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
            hints,
            palette: Palette::default(),
            search: String::new(),
            input: String::new(),
            log: Vec::new(),
            scan: None,
            busy: HashMap::new(),
            tick: 0,
            quit: false,
            confirm: None,
            diff: None,
            history: None,
            branches: None,
            gh: None,
            text: None,
            finder: None,
            clone_slug: None,
            input_back: Mode::Normal,
            pending_select: None,
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
                    let path = repo.path.clone();
                    match self.repos.iter_mut().find(|r| r.path == path) {
                        Some(slot) => *slot = repo,
                        None => self.repos.push(repo),
                    }
                    self.rebuild();
                    if self.pending_select.as_ref() == Some(&path) {
                        self.pending_select = None;
                        if let Some(i) = self.visible.iter().position(|&r| self.repos[r].path == path) {
                            self.repo_state.select(Some(i));
                            self.file_state.select(None);
                            self.clamp_files();
                        }
                    }
                }
                Msg::ScanDone(ms) => {
                    self.scan = None;
                    // Drop repositories that no longer exist on disk.
                    let keep = &self.scan_paths;
                    self.repos.retain(|r| keep.contains(&r.path));
                    self.rebuild();
                    self.log(Level::Ok, format!("scan finished: {} repositories in {ms} ms", self.repos.len()));
                }
                Msg::History { id, result } => {
                    if let Some(v) = self.history.as_mut().filter(|v| v.id == id) {
                        match result {
                            Ok(commits) => {
                                v.state.select(if commits.is_empty() { None } else { Some(0) });
                                v.commits = Some(commits);
                            }
                            Err(e) => v.error = Some(e),
                        }
                    }
                }
                Msg::GitHub { id, result } => {
                    if let Some(v) = self.gh.as_mut().filter(|v| v.id == id) {
                        match result {
                            Ok(items) => {
                                let keep = v.selected().map(|i| i.number);
                                let at = items.iter().position(|i| Some(i.number) == keep);
                                v.list.select(at.or(if items.is_empty() { None } else { Some(0) }));
                                v.items = Some(items);
                                v.error = None;
                            }
                            Err(e) => v.error = Some(e),
                        }
                    }
                }
                Msg::Repos { id, result } => {
                    if let Some(v) = self.finder.as_mut().filter(|v| v.id == id) {
                        match result {
                            Ok(items) => {
                                v.list.select(if items.is_empty() { None } else { Some(0) });
                                v.items = Some(items);
                            }
                            Err(e) => v.error = Some(e),
                        }
                    }
                }
                Msg::Text { id, result } => {
                    if let Some(v) = self.text.as_mut().filter(|v| v.id == id) {
                        match result {
                            // The terminal can show neither tabs nor control characters as they are.
                            Ok(text) => v.lines = Some(text.lines().map(|l| l.replace('\t', "    ").chars().filter(|c| !c.is_control()).collect()).collect()),
                            Err(e) => v.error = Some(e),
                        }
                    }
                }
                Msg::Branches { id, result } => {
                    if let Some(v) = self.branches.as_mut().filter(|v| v.id == id) {
                        match result {
                            Ok(list) => {
                                // Keep the cursor on the same branch after a reload; start on the current one.
                                let keep = v.selected().map(|b| b.name.clone());
                                let at = list.iter().position(|b| Some(&b.name) == keep.as_ref()).or_else(|| list.iter().position(|b| b.current));
                                v.state.select(at.or(if list.is_empty() { None } else { Some(0) }));
                                v.branches = Some(list);
                                v.error = None;
                            }
                            Err(e) => v.error = Some(e),
                        }
                    }
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
                    if self.branches.as_ref().is_some_and(|v| v.repo == path) {
                        self.load_branches(); // switching, creating or deleting changed the list
                    }
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
            Mode::Palette => self.on_palette_key(key),
            Mode::Diff => self.on_diff_key(key),
            Mode::History => self.on_history_key(key),
            Mode::Branches => self.on_branch_key(key),
            Mode::GitHub => self.on_gh_key(key),
            Mode::Text => self.on_text_key(key),
            Mode::Finder => self.on_finder_key(key),
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
            KeyCode::Right => self.focus = Focus::Files,
            KeyCode::Left => self.focus = Focus::Repos,
            KeyCode::Char('h') => self.hints = !self.hints,
            KeyCode::Char(':') => self.open_palette(),
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => self.open_palette(),
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
            KeyCode::Char('v') => self.open_history(),
            KeyCode::Char('b') => self.open_branches(),
            KeyCode::Char('I') => self.open_init(),
            KeyCode::Char('i') => self.open_gh(Kind::Issues),
            KeyCode::Char('y') => self.open_gh(Kind::Pulls),
            KeyCode::Char('B') => self.browse_repo(),
            KeyCode::Char('S') => self.open_input(InputKind::GhSearch),
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
            KeyCode::Char('e') => self.open_browser(),
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
        self.start_diff(repo, file, kind, both, Mode::Normal);
    }

    /// Opens the diff viewer on `target` (a file, or a commit hash for `DiffKind::Commit`).
    fn start_diff(&mut self, repo: PathBuf, target: String, kind: DiffKind, both: bool, back: Mode) {
        self.diff_id += 1;
        self.diff = Some(DiffView {
            repo: repo.clone(),
            file: target.clone(),
            kind,
            diff: None,
            error: None,
            scroll: 0,
            hscroll: 0,
            height: 20,
            both,
            back,
            id: self.diff_id,
        });
        self.mode = Mode::Diff;
        worker::spawn_diff(repo, target, kind, self.diff_id, self.tx.clone());
    }

    // ---- commit log ----

    fn open_history(&mut self) {
        let Some((repo, name, branch)) = self.selected().map(|r| (r.path.clone(), r.name.clone(), r.branch.clone())) else { return };
        self.diff_id += 1;
        worker::spawn_history(repo.clone(), self.diff_id, self.tx.clone());
        self.history = Some(HistoryView { repo, name, branch, commits: None, error: None, state: ListState::default(), id: self.diff_id });
        self.mode = Mode::History;
    }

    fn on_history_key(&mut self, key: KeyEvent) {
        let Some(v) = self.history.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        let len = v.commits.as_ref().map_or(0, Vec::len);
        let delta = match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'v') => {
                self.history = None;
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Down | KeyCode::Char('j') => 1,
            KeyCode::Up | KeyCode::Char('k') => -1,
            KeyCode::PageDown | KeyCode::Char(' ') => 15,
            KeyCode::PageUp | KeyCode::Char('b') => -15,
            KeyCode::Home | KeyCode::Char('g') => -(1 << 30),
            KeyCode::End | KeyCode::Char('G') => 1 << 30,
            KeyCode::Enter | KeyCode::Char('d') => {
                let hash = v.commits.as_ref().and_then(|c| c.get(v.state.selected()?)).map(|c| c.hash.clone());
                if let Some(hash) = hash {
                    let repo = v.repo.clone();
                    self.start_diff(repo, hash, DiffKind::Commit, false, Mode::History);
                }
                return;
            }
            _ => return,
        };
        v.state.select(step(v.state.selected(), len, delta));
    }

    // ---- branches ----

    fn open_branches(&mut self) {
        let Some((repo, name)) = self.selected().map(|r| (r.path.clone(), r.name.clone())) else { return };
        self.branches = Some(BranchView { repo, name, branches: None, error: None, state: ListState::default(), id: 0 });
        self.mode = Mode::Branches;
        self.load_branches();
    }

    fn load_branches(&mut self) {
        self.diff_id += 1;
        if let Some(v) = self.branches.as_mut() {
            v.id = self.diff_id;
            worker::spawn_branches(v.repo.clone(), v.id, self.tx.clone());
        }
    }

    fn on_branch_key(&mut self, key: KeyEvent) {
        let Some(v) = self.branches.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        let len = v.branches.as_ref().map_or(0, Vec::len);
        let repo = v.repo.clone();
        let picked = v.selected().map(|b| (b.name.clone(), b.remote, b.current, b.merged));
        let delta = match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'b') => {
                self.branches = None;
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Down | KeyCode::Char('j') => 1,
            KeyCode::Up | KeyCode::Char('k') => -1,
            KeyCode::PageDown | KeyCode::Char(' ') => 10,
            KeyCode::PageUp => -10,
            KeyCode::Home | KeyCode::Char('g') => -(1 << 30),
            KeyCode::End | KeyCode::Char('G') => 1 << 30,
            KeyCode::Char('n') => {
                self.open_input(InputKind::Branch);
                return;
            }
            _ => {
                let Some((name, remote, current, merged)) = picked else { return };
                match key.code {
                    KeyCode::Enter if current => self.log(Level::Info, format!("already on {name}")),
                    KeyCode::Enter => self.run_ops(vec![(repo, Op::Switch(name, remote))]),
                    KeyCode::Char('m') if current => self.log(Level::Warn, "cannot merge a branch into itself"),
                    KeyCode::Char('m') => self.ask(format!("merge {name} into the current branch?"), vec![(repo, Op::Merge(name))]),
                    KeyCode::Char(c @ ('d' | 'D')) => {
                        if remote {
                            self.log(Level::Warn, "remote branches are not deleted from here");
                        } else if current {
                            self.log(Level::Warn, "cannot delete the branch you are on; switch first");
                        } else if c == 'd' && !merged {
                            self.log(Level::Warn, format!("{name} is not merged; press D to delete it anyway"));
                        } else {
                            let how = if c == 'D' && !merged { "delete UNMERGED branch" } else { "delete branch" };
                            self.ask(format!("{how} {name}?"), vec![(repo, Op::DeleteBranch(name, c == 'D'))]);
                        }
                    }
                    _ => {}
                }
                return;
            }
        };
        v.state.select(step(v.state.selected(), len, delta));
    }

    /// Asks for a yes before running `jobs`, then returns to the current mode.
    fn ask(&mut self, prompt: String, jobs: Vec<(PathBuf, Op)>) {
        self.confirm = Some(Confirm { prompt, jobs, back: self.mode });
        self.mode = Mode::Confirm;
    }

    // ---- GitHub ----

    /// The selected repository's name and GitHub slug; logs a warning when it is not on GitHub.
    fn github_target(&mut self) -> Option<(PathBuf, String, String)> {
        let r = self.selected()?;
        let found = r.slug.clone().map(|s| (r.path.clone(), r.name.clone(), s));
        if found.is_none() {
            let msg = format!("{}: no github.com remote", r.name);
            self.log(Level::Warn, msg);
        }
        found
    }

    fn open_gh(&mut self, kind: Kind) {
        let Some((repo, name, slug)) = self.github_target() else { return };
        self.gh = Some(GhView { repo, name, slug, kind, state: State::Open, items: None, error: None, list: ListState::default(), id: 0 });
        self.mode = Mode::GitHub;
        self.load_gh();
    }

    fn load_gh(&mut self) {
        self.diff_id += 1;
        let Some(v) = self.gh.as_mut() else { return };
        v.id = self.diff_id;
        v.items = None;
        v.error = None;
        let (id, slug, kind, state) = (v.id, v.slug.clone(), v.kind, v.state);
        worker::spawn_msg(self.tx.clone(), move || Msg::GitHub { id, result: github::list(&slug, kind, state) });
    }

    fn on_gh_key(&mut self, key: KeyEvent) {
        let Some(v) = self.gh.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        let len = v.items.as_ref().map_or(0, Vec::len);
        let delta = match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'i' | 'y') => {
                self.gh = None;
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                v.kind = v.kind.other();
                v.state = State::Open;
                self.load_gh();
                return;
            }
            KeyCode::Char('s') => {
                v.state = v.state.next(v.kind);
                self.load_gh();
                return;
            }
            KeyCode::Char('r') => {
                self.load_gh();
                return;
            }
            KeyCode::Char('o') => {
                let args = v.selected().map(|i| github::browse_args(&v.slug, v.kind, Some(i.number)));
                if let Some(args) = args {
                    self.spawn_detached("gh", &args, "opened in the browser");
                }
                return;
            }
            KeyCode::Enter => {
                if let Some(i) = v.selected() {
                    let (number, title) = (i.number, i.title.clone());
                    self.open_text(number, title);
                }
                return;
            }
            KeyCode::Char('d') if v.kind == Kind::Pulls => {
                if let Some(i) = v.selected() {
                    let target = format!("{}#{}", v.slug, i.number);
                    let repo = v.repo.clone();
                    self.start_diff(repo, target, DiffKind::Pull, false, Mode::GitHub);
                }
                return;
            }
            KeyCode::Char('c') if v.kind == Kind::Pulls => {
                if let Some(i) = v.selected() {
                    let (repo, n) = (v.repo.clone(), i.number);
                    let prompt = format!("check out PR #{n} ({}) in {}?", i.branch, v.name);
                    self.ask(prompt, vec![(repo, Op::CheckoutPr(n))]);
                }
                return;
            }
            KeyCode::Char('B') => {
                let args = github::browse_args(&v.slug, v.kind, None);
                self.spawn_detached("gh", &args, "repository opened in the browser");
                return;
            }
            KeyCode::Down | KeyCode::Char('j') => 1,
            KeyCode::Up | KeyCode::Char('k') => -1,
            KeyCode::PageDown | KeyCode::Char(' ') => 15,
            KeyCode::PageUp | KeyCode::Char('b') => -15,
            KeyCode::Home | KeyCode::Char('g') => -(1 << 30),
            KeyCode::End | KeyCode::Char('G') => 1 << 30,
            _ => return,
        };
        v.list.select(step(v.list.selected(), len, delta));
    }

    /// Opens issue or pull request `number` of the current GitHub list as text.
    fn open_text(&mut self, number: u64, title: String) {
        let Some(v) = self.gh.as_ref() else { return };
        let (slug, kind) = (v.slug.clone(), v.kind);
        let sigil = if kind == Kind::Pulls { "PR" } else { "Issue" };
        self.diff_id += 1;
        let id = self.diff_id;
        let browse = github::browse_args(&slug, kind, Some(number));
        self.text = Some(TextView { title: format!("{sigil} #{number} · {title}"), lines: None, error: None, scroll: 0, height: 20, browse, id });
        self.mode = Mode::Text;
        // gh formats for a terminal of the given width; leave room for the panel border and padding.
        let width = crossterm::terminal::size().map_or(100, |(w, _)| w.saturating_sub(6).clamp(40, 140));
        worker::spawn_msg(self.tx.clone(), move || Msg::Text { id, result: github::view(&slug, kind, number, width) });
    }

    fn on_text_key(&mut self, key: KeyEvent) {
        let Some(v) = self.text.as_mut() else {
            self.mode = Mode::GitHub;
            return;
        };
        let max = v.lines.as_ref().map_or(0, |l| l.len().saturating_sub(v.height));
        let page = v.height.saturating_sub(1).max(1);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.text = None;
                self.mode = Mode::GitHub;
            }
            KeyCode::Down | KeyCode::Char('j') => v.scroll = (v.scroll + 1).min(max),
            KeyCode::Up | KeyCode::Char('k') => v.scroll = v.scroll.saturating_sub(1),
            KeyCode::PageDown | KeyCode::Char(' ') => v.scroll = (v.scroll + page).min(max),
            KeyCode::PageUp | KeyCode::Char('b') => v.scroll = v.scroll.saturating_sub(page),
            KeyCode::Home | KeyCode::Char('g') => v.scroll = 0,
            KeyCode::End | KeyCode::Char('G') => v.scroll = max,
            KeyCode::Char('o') => {
                let args = v.browse.clone();
                self.spawn_detached("gh", &args, "opened in the browser");
            }
            _ => {}
        }
    }

    // ---- finding and cloning repositories ----

    fn load_finder(&mut self) {
        self.diff_id += 1;
        let Some(v) = self.finder.as_mut() else { return };
        v.id = self.diff_id;
        v.items = None;
        v.error = None;
        let (id, query) = (v.id, v.query.clone());
        worker::spawn_msg(self.tx.clone(), move || Msg::Repos { id, result: github::find(&query) });
    }

    fn on_finder_key(&mut self, key: KeyEvent) {
        let Some(v) = self.finder.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        let len = v.items.as_ref().map_or(0, Vec::len);
        let delta = match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.finder = None;
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Char('/' | 'S') => {
                let query = v.query.clone();
                self.open_input(InputKind::GhSearch);
                self.input = query;
                return;
            }
            KeyCode::Char('r') => {
                self.load_finder();
                return;
            }
            KeyCode::Char('o') => {
                let args = v.selected().map(|i| vec!["browse".to_string(), "-R".into(), i.slug.clone()]);
                if let Some(args) = args {
                    self.spawn_detached("gh", &args, "opened in the browser");
                }
                return;
            }
            KeyCode::Enter => {
                if let Some(slug) = v.selected().map(|i| i.slug.clone()) {
                    self.choose_repo(slug);
                }
                return;
            }
            KeyCode::Down | KeyCode::Char('j') => 1,
            KeyCode::Up | KeyCode::Char('k') => -1,
            KeyCode::PageDown | KeyCode::Char(' ') => 15,
            KeyCode::PageUp | KeyCode::Char('b') => -15,
            KeyCode::Home | KeyCode::Char('g') => -(1 << 30),
            KeyCode::End | KeyCode::Char('G') => 1 << 30,
            _ => return,
        };
        v.list.select(step(v.list.selected(), len, delta));
    }

    /// Enter on a found repository: jump to it if it is already cloned, else ask where to clone it.
    fn choose_repo(&mut self, slug: String) {
        let local = self.repos.iter().find(|r| r.slug.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(&slug))).map(|r| r.path.clone());
        if let Some(path) = local {
            self.log(Level::Info, format!("{slug} is already cloned at {}", path.display()));
            self.finder = None;
            self.mode = Mode::Normal;
            self.filter = Filter::All;
            self.search.clear();
            self.show_hidden = true;
            self.rebuild();
            if let Some(i) = self.visible.iter().position(|&r| self.repos[r].path == path) {
                self.repo_state.select(Some(i));
                self.file_state.select(None);
                self.clamp_files();
            }
            return;
        }
        let name = slug.rsplit('/').next().unwrap_or(&slug).to_string();
        self.clone_slug = Some(slug);
        self.open_input(InputKind::CloneTo);
        self.input = self.cfg.clone_dir.join(name).display().to_string();
    }

    /// Opens the selected repository's GitHub page in the browser.
    fn browse_repo(&mut self) {
        let Some((_, _, slug)) = self.github_target() else { return };
        self.spawn_detached("gh", &github::browse_args(&slug, Kind::Issues, None), "repository opened in the browser");
    }

    /// Starts a program without waiting for it (browsers, file managers) and logs the outcome.
    fn spawn_detached(&mut self, program: &str, args: &[String], done: &str) {
        let child = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        match child {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait(); // reap the process
                });
                self.log(Level::Info, done.to_string());
            }
            Err(e) => self.log(Level::Error, format!("cannot run {program}: {e}")),
        }
    }

    // ---- git init ----

    /// Prompts for a folder; the input starts with the current directory so a new folder name can be appended.
    fn open_init(&mut self) {
        self.open_input(InputKind::Init);
        if let Ok(dir) = std::env::current_dir() {
            self.input = format!("{}/", dir.display());
        }
    }

    /// Clones the repository chosen in the finder into `text` (relative paths start from `clone_dir`).
    fn clone_repo(&mut self, text: &str) {
        let Some(slug) = self.clone_slug.take() else { return };
        if text.is_empty() {
            self.log(Level::Warn, "no folder given, clone cancelled");
            return;
        }
        let mut path = config::expand(text);
        if path.is_relative() {
            path = self.cfg.clone_dir.join(path);
        }
        self.log(Level::Info, format!("cloning {slug} into {}", path.display()));
        self.pending_select = Some(path.clone());
        self.run_ops(vec![(path, Op::Clone(slug))]);
    }

    fn init_repo(&mut self, text: &str) {
        if text.is_empty() {
            self.log(Level::Warn, "no folder given, git init cancelled");
            return;
        }
        let mut path = config::expand(text);
        if path.is_relative() {
            path = std::env::current_dir().unwrap_or_default().join(path);
        }
        if path.join(".git").exists() {
            self.log(Level::Warn, format!("{} is already a git repository", path.display()));
            return;
        }
        self.pending_select = Some(path.clone());
        self.run_ops(vec![(path, Op::Init)]);
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
                self.mode = v.back;
                self.diff = None;
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

    fn open_palette(&mut self) {
        self.palette = Palette::default();
        self.mode = Mode::Palette;
    }

    fn on_palette_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let found = palette::matches(&self.palette.query);
        let delta = match key.code {
            // First Esc clears what was typed, the next one closes the palette.
            KeyCode::Esc if !self.palette.query.is_empty() => {
                self.palette.query.clear();
                self.palette.state.select(Some(0));
                return;
            }
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Enter => {
                let pick = self.palette.state.selected().and_then(|i| found.get(i));
                self.mode = Mode::Normal;
                if let Some(&(entry, _)) = pick {
                    // Run the entry exactly as if its key had been pressed on the dashboard.
                    let code = KeyCode::Char(palette::ENTRIES[entry].code);
                    self.on_normal_key(KeyEvent::new(code, KeyModifiers::NONE));
                }
                return;
            }
            KeyCode::Down => 1,
            KeyCode::Up => -1,
            KeyCode::Char('n') if ctrl => 1,
            KeyCode::Char('p') if ctrl => -1,
            KeyCode::PageDown => 8,
            KeyCode::PageUp => -8,
            KeyCode::Backspace => {
                self.palette.query.pop();
                self.palette.state.select(Some(0));
                return;
            }
            KeyCode::Char('u') if ctrl => {
                self.palette.query.clear();
                self.palette.state.select(Some(0));
                return;
            }
            KeyCode::Char(c) if !ctrl => {
                self.palette.query.push(c);
                self.palette.state.select(Some(0));
                return;
            }
            _ => return,
        };
        self.palette.state.select(step(self.palette.state.selected(), found.len(), delta));
    }

    fn on_confirm_key(&mut self, key: KeyEvent) {
        let Some(confirm) = self.confirm.take() else {
            self.mode = Mode::Normal;
            return;
        };
        self.mode = confirm.back;
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
                self.mode = self.input_back;
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
        self.input_back = if matches!(self.mode, Mode::Branches | Mode::Finder) { self.mode } else { Mode::Normal };
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
        self.mode = self.input_back;
        let text = std::mem::take(&mut self.input).trim().to_string();
        match kind {
            InputKind::Init => self.init_repo(&text),
            InputKind::GhSearch => {
                self.finder = Some(FinderView { query: text, items: None, error: None, list: ListState::default(), id: 0 });
                self.mode = Mode::Finder;
                self.load_finder();
            }
            InputKind::CloneTo => self.clone_repo(&text),
            InputKind::Branch if text.is_empty() => self.log(Level::Warn, "empty branch name, nothing created"),
            InputKind::Branch => {
                if let Some(repo) = self.branches.as_ref().map(|v| v.repo.clone()) {
                    self.run_ops(vec![(repo, Op::NewBranch(text))]);
                }
            }
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
        self.ask(prompt, jobs);
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

    /// Opens yazi in the repository, revealing the highlighted file when the changes pane has focus.
    fn open_browser(&mut self) {
        let Some(r) = self.selected() else { return };
        let file = if self.focus == Focus::Files { self.file_state.selected().and_then(|i| r.files.get(i)) } else { None };
        // A deleted file no longer exists to reveal; fall back to the repository folder.
        let target = file.map(|f| r.path.join(&f.path)).filter(|p| p.exists()).unwrap_or_else(|| r.path.clone());
        self.launch = Some(Launch::Browse { repo: r.path.clone(), target });
    }

    fn open_shell(&mut self) {
        if let Some(r) = self.selected() {
            self.launch = Some(Launch::Shell { repo: r.path.clone() });
        }
    }

    /// Opens the repository folder in the desktop file manager, without leaving the dashboard.
    fn open_files(&mut self) {
        let Some((path, name)) = self.selected().map(|r| (r.path.clone(), r.name.clone())) else { return };
        self.spawn_detached("xdg-open", &[path.display().to_string()], &format!("{name}: opened in the file manager"));
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
