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
    time::{Duration, Instant},
};

mod flows;
mod view;

pub use flows::WStep;
pub use view::{CodeView, PickerView, Prompt};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;

use crate::{
    config::{self, Config},
    diff::{self, Diff},
    git::{self, Branch, Commit, DiffKind, Op, ReflogEntry, Remote, Tag, TreeItem, TreeKind},
    langs::Breakdown,
    github::{self, Ci, Hits, Item, Kind, Release, Scope, State, WorkflowRun},
    license::{LICENSES, License},
    logger::{self, Level, Logger},
    marks::{Mark, Marks},
    palette,
    repo::{Health, Owner, Repo},
    scaffold::{self, PackageKind, Plan, langs},
    term::Term,
    watch::Watcher,
    worker::{self, Msg},
};

/// How many event-loop iterations (about 100 ms each) a message stays up while the log panel is hidden.
const TOAST_TICKS: usize = 40;

/// Quiet time after the last file event before a repository is reloaded, so a burst of writes reloads once.
const RELOAD_DELAY: Duration = Duration::from_millis(400);

/// How long another repository must stay selected before its working tree is watched.
const TREE_DELAY: Duration = Duration::from_millis(300);

/// Most repositories asked for their CI state in one batch.
const CI_BATCH: usize = 100;

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
    /// The editor `editor` (a command line, see `App::editor_line`) on `file` of `repo`, at `line` (1-based).
    Edit { repo: PathBuf, file: String, line: usize, editor: String },
}

impl Launch {
    pub fn repo(&self) -> &Path {
        match self {
            Launch::Browse { repo, .. } | Launch::Edit { repo, .. } => repo,
        }
    }

    /// Runs the program in the foreground and waits for it.
    pub fn run(&self) -> Result<(), String> {
        let (repo, target) = match self {
            Launch::Browse { repo, target } => (repo, target),
            Launch::Edit { repo, file, line, editor } => return edit(repo, &repo.join(file), *line, editor),
        };
        match Command::new("yazi").arg(target).current_dir(repo).status() {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                run_editor(repo, target).map_err(|e| format!("yazi is not installed, and the editor failed: {e}"))
            }
            Err(e) => Err(format!("cannot run yazi: {e}")),
        }
    }
}

/// Runs the editor command line `spec` on `path`, opened at `line`, and waits for it. `{file}` and `{line}` in the
/// arguments are replaced; without them the arguments the editor needs for a line are added from its name
/// (`+12 file` for vi, nano, emacs ...; `file:12` for helix, zed, sublime; `-g file:12` for VS Code).
fn edit(repo: &Path, path: &Path, line: usize, spec: &str) -> Result<(), String> {
    let mut parts = spec.split_whitespace().map(str::to_string);
    let program = parts.next().unwrap_or_else(|| "vi".to_string());
    let mut args: Vec<String> = parts.collect();
    let file = path.display().to_string();
    if args.iter().any(|a| a.contains("{file}")) {
        args = args.iter().map(|a| a.replace("{file}", &file).replace("{line}", &line.to_string())).collect();
    } else {
        match Path::new(&program).file_name().and_then(|n| n.to_str()).unwrap_or(&program) {
            "vi" | "vim" | "nvim" | "view" | "nano" | "pico" | "micro" | "emacs" | "emacsclient" | "kak" | "gedit" | "joe" => {
                args.extend([format!("+{line}"), file]);
            }
            "hx" | "helix" | "zed" | "subl" | "sublime_text" => args.push(format!("{file}:{line}")),
            "code" | "codium" | "code-insiders" | "cursor" => args.extend(["-g".to_string(), format!("{file}:{line}")]),
            _ => args.push(file),
        }
    }
    Command::new(&program)
        .args(&args)
        .current_dir(repo)
        .status()
        .map(|_| ())
        .map_err(|e| format!("cannot run {program}: {e}"))
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
    /// The diff text as git printed it, kept to cut single hunks out of it for staging.
    raw: String,
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

impl DiffView {
    /// Index of the hunk the top row belongs to (the first one above the first hunk header).
    pub fn hunk(&self) -> Option<usize> {
        let hunks = &self.diff.as_ref()?.hunks;
        (!hunks.is_empty()).then(|| hunks.iter().rposition(|&h| h <= self.scroll).unwrap_or(0))
    }

    /// True for the file diffs whose hunks can be staged or unstaged.
    pub fn stageable(&self) -> bool {
        matches!(self.kind, DiffKind::Staged | DiffKind::Unstaged)
    }
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

/// The reflog of one repository: every place HEAD has been, for undoing mistakes.
pub struct ReflogView {
    pub repo: PathBuf,
    pub name: String,
    /// None while the reflog is loading.
    pub entries: Option<Vec<ReflogEntry>>,
    pub error: Option<String>,
    pub state: ListState,
    id: u64,
}

/// The remotes of one repository.
pub struct RemotesView {
    pub repo: PathBuf,
    pub name: String,
    /// None while loading.
    pub items: Option<Vec<Remote>>,
    pub error: Option<String>,
    pub state: ListState,
    id: u64,
}

impl RemotesView {
    fn selected(&self) -> Option<&Remote> {
        self.items.as_ref()?.get(self.state.selected()?)
    }
}

/// The tags of one repository, with what is known about them on the remote and on GitHub.
pub struct TagsView {
    pub repo: PathBuf,
    pub name: String,
    /// `owner/repo` of the GitHub remote; releases are only offered with one.
    pub slug: Option<String>,
    /// None while loading.
    pub items: Option<Vec<Tag>>,
    pub error: Option<String>,
    pub state: ListState,
    /// The tags on the remote; None while the server is asked, Err when it could not be (offline, no remote).
    pub remote: Option<Result<HashSet<String>, String>>,
    /// The GitHub releases by tag; None while loading, Err without `gh` or a GitHub remote.
    pub releases: Option<Result<HashMap<String, Release>, String>>,
    id: u64,
}

impl TagsView {
    pub fn selected(&self) -> Option<&Tag> {
        self.items.as_ref()?.get(self.state.selected()?)
    }
}

/// The worktrees and submodules of one repository.
pub struct TreesView {
    pub repo: PathBuf,
    pub name: String,
    /// None while loading.
    pub items: Option<Vec<TreeItem>>,
    pub error: Option<String>,
    pub state: ListState,
    id: u64,
}

impl TreesView {
    fn selected(&self) -> Option<&TreeItem> {
        self.items.as_ref()?.get(self.state.selected()?)
    }

    /// The repository's own folder: where worktree commands run from, whichever worktree is open.
    fn main(&self) -> PathBuf {
        self.items.as_ref().and_then(|i| i.first()).map_or_else(|| self.repo.clone(), |m| PathBuf::from(&m.path))
    }
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
    /// What was typed (shown in the title); empty for your own repositories.
    pub query: String,
    pub scope: Scope,
    /// The query without its `user:` / `repo:` / `file:` prefix.
    pub terms: String,
    /// Current page, from 1.
    pub page: usize,
    /// True when a next page exists.
    pub more: bool,
    /// Matches in all, when GitHub reports them.
    pub total: Option<u64>,
    /// None while loading.
    pub hits: Option<Hits>,
    pub error: Option<String>,
    pub list: ListState,
    id: u64,
}

impl FinderView {
    fn new(query: String, scope: Scope, terms: String) -> FinderView {
        FinderView { query, scope, terms, page: 1, more: false, total: None, hits: None, error: None, list: ListState::default(), id: 0 }
    }

    /// The repository of the highlighted row (None on a user).
    fn selected_slug(&self) -> Option<String> {
        self.hits.as_ref()?.slug(self.list.selected()?).map(String::from)
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

/// The pages of the new-project menu.
#[derive(Clone, Copy, PartialEq)]
pub enum ProjectStep {
    Lang,
    /// Package names, for languages that keep their code in packages.
    Package,
    Variant,
    License,
    Confirm,
}

/// A row of the language page.
#[derive(Clone, Copy, PartialEq)]
pub enum Choice {
    /// An index into `scaffold::langs()`.
    Lang(usize),
    /// Skip the menu and set the project up by hand in the embedded terminal.
    Other,
}

/// The new-project menu (`K`) for an empty repository: language, variant, license, then a plan to confirm.
pub struct ProjectView {
    pub repo: PathBuf,
    pub step: ProjectStep,
    /// Text typed on the language page; it narrows the list.
    pub query: String,
    pub lang_state: ListState,
    /// The chosen language, an index into `scaffold::langs()`.
    pub lang: usize,
    pub variant_state: ListState,
    /// Selection on the license page; `LICENSES.len()` is "no license".
    pub license_state: ListState,
    /// Package names being typed on the package page (comma-separated).
    pub package: String,
    /// Why `package` was refused.
    pub package_error: Option<String>,
    /// The packages accepted on the package page; empty for a language without them.
    pub packages: Vec<String>,
    /// Copyright holder of the LICENSE, edited on the license page.
    pub holder: String,
    /// The holder is being typed instead of the list being moved.
    pub editing: bool,
    pub year: i64,
    /// Make a first commit with the new files (set on the confirmation page).
    pub commit: bool,
    /// What will be done, built when the confirmation page opens.
    pub plan: Option<Plan>,
}

impl ProjectView {
    /// The rows of the language page that match the query, "Other" last.
    pub fn choices(&self) -> Vec<Choice> {
        let q = self.query.to_lowercase();
        let mut out: Vec<Choice> = (0..langs().len()).filter(|&i| langs()[i].name.to_lowercase().contains(&q)).map(Choice::Lang).collect();
        if q.is_empty() || ["other", "manual", "terminal"].iter().any(|w| w.contains(&q)) {
            out.push(Choice::Other);
        }
        out
    }

    /// The page after the language page: package names if the language has packages, else the variants (when
    /// there is a choice) or the license.
    fn after_lang(&self) -> ProjectStep {
        let lang = &langs()[self.lang];
        if lang.package.is_some() { ProjectStep::Package } else { self.after_package() }
    }

    fn after_package(&self) -> ProjectStep {
        if langs()[self.lang].variants.len() > 1 { ProjectStep::Variant } else { ProjectStep::License }
    }

    /// The page `Esc` returns to from the license page.
    fn before_license(&self) -> ProjectStep {
        let lang = &langs()[self.lang];
        if lang.variants.len() > 1 {
            ProjectStep::Variant
        } else if lang.package.is_some() {
            ProjectStep::Package
        } else {
            ProjectStep::Lang
        }
    }

    /// The chosen license; None for "no license".
    pub fn license(&self) -> Option<&'static License> {
        self.license_state.selected().and_then(|i| LICENSES.get(i))
    }
}

/// Which pane receives navigation keys.
#[derive(Clone, Copy, PartialEq)]
pub enum Focus {
    Repos,
    /// The languages pane of the details panel.
    Langs,
    Files,
}

/// What the text input line is collecting.
#[derive(Clone, Copy, PartialEq)]
pub enum InputKind {
    Commit,
    /// Name of a new tag, optionally followed by a message (which makes it an annotated tag).
    Tag,
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
    /// The repository's name, typed to confirm deleting it from disk.
    DeleteRepo,
    /// Message for amending the last commit (empty keeps the old one).
    Amend,
    /// Branch for a new worktree.
    Worktree,
    /// `<name> <url>` of a remote to add.
    RemoteAdd,
    /// The new URL of the remote in `App::remote_target`.
    RemoteUrl,
    /// The new name of the remote in `App::remote_target`.
    RemoteRename,
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
    /// The reflog (`App::reflog`) is open.
    Reflog,
    /// The worktree and submodule list (`App::trees`) is open.
    Trees,
    /// The remote list (`App::remotes`) is open.
    Remotes,
    /// The GitHub issue or pull request list (`App::gh`) is open.
    GitHub,
    /// An issue or pull request opened as text (`App::text`).
    Text,
    /// GitHub repository search results (`App::finder`).
    Finder,
    /// The file finder (`App::picker`) is open.
    Picker,
    /// The code viewer (`App::code`) is open.
    Code,
    /// The embedded terminal (`App::terms[App::term]`) is open and gets every key.
    Terminal,
    /// The new-project menu (`App::project`) is open.
    Project,
    /// The tags and releases (`App::tags`) are open.
    Tags,
    /// The workflows (`App::workflows`) are open.
    Workflows,
    /// The form that starts a workflow (`App::run_form`) is open.
    WorkflowRun,
    /// The wizard that writes a new workflow (`App::wizard`) is open.
    WorkflowNew,
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
    /// Selection in the languages pane (it only scrolls).
    pub lang_state: ListState,
    /// Language breakdown per repository, with the commit it was computed for.
    pub langs: HashMap<PathBuf, (String, Result<Breakdown, String>)>,
    /// Repositories whose languages are being counted.
    lang_busy: HashSet<PathBuf>,
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
    /// The log panel is visible (toggled with `l`).
    pub logs: bool,
    /// The `tick` at which the newest log line was added, for the short message shown while logs are hidden.
    log_tick: usize,
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
    pub reflog: Option<ReflogView>,
    pub trees: Option<TreesView>,
    pub remotes: Option<RemotesView>,
    pub tags: Option<TagsView>,
    pub workflows: Option<flows::WorkflowsView>,
    pub picker: Option<PickerView>,
    pub code: Option<CodeView>,
    pub run_form: Option<flows::RunForm>,
    pub wizard: Option<flows::Wizard>,
    /// The remote an open input line is about (change URL, rename).
    remote_target: Option<String>,
    /// Why the next commit may be made with the wrong identity (shown next to the commit prompt).
    pub commit_warn: Option<String>,
    /// The message of the commit being made, and of the last one that failed (e.g. rejected by a hook).
    pending_msg: Option<(PathBuf, String)>,
    retry_msg: Option<(PathBuf, String)>,
    pub gh: Option<GhView>,
    pub text: Option<TextView>,
    pub finder: Option<FinderView>,
    pub project: Option<ProjectView>,
    /// The repository chosen in the finder, waiting for a clone destination.
    clone_slug: Option<String>,
    /// Mode to return to when the input line closes (the dashboard, or the branch list).
    input_back: Mode,
    /// A repository just created with `git init`, selected as soon as it is loaded.
    pending_select: Option<PathBuf>,
    diff_id: u64,
    /// The file watcher; None where inotify is unavailable.
    watch: Option<Watcher>,
    /// Repositories with file events waiting for their quiet time, with the time of the last event.
    changed: HashMap<PathBuf, Instant>,
    /// The repository whose working tree is (or is about to be) watched, and since when it has been selected.
    tree: Option<(PathBuf, Instant)>,
    /// The repository whose working tree the watcher was last pointed at.
    tree_done: Option<PathBuf>,
    /// Repositories being fetched by the timer; their results are only logged when something fails.
    quiet: HashSet<PathBuf>,
    /// An automatic fetch already reported a failure in this round.
    auto_failed: bool,
    /// When the timer last started an automatic fetch (or when Oxide started).
    auto_at: Instant,
    /// Newest CI state per repository, with the branch it belongs to.
    pub ci: HashMap<PathBuf, (String, Ci)>,
    /// A CI batch is running.
    ci_busy: bool,
    /// Automatic CI refreshes are on (turned off when `gh` is unusable, until `w` is pressed).
    ci_auto: bool,
    /// A terminal program requested by the user; taken and run by the event loop.
    pub launch: Option<Launch>,
    /// Embedded shells, one per repository. They keep running while hidden.
    pub terms: Vec<Term>,
    /// The shell in `terms` that is shown while in `Mode::Terminal`.
    pub term: Option<usize>,
    marks: Marks,
    logger: Logger,
    scan_paths: HashSet<PathBuf>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

/// Conventional-commit types offered by Tab in the commit prompt.
const COMMIT_TYPES: [&str; 11] = ["feat", "fix", "docs", "style", "refactor", "perf", "test", "build", "ci", "chore", "revert"];

/// Puts the next (or previous) conventional-commit type in front of the message, replacing the one it
/// already starts with; `type(scope)!: subject` keeps its scope and bang.
fn cycle_type(input: &mut String, forward: bool) {
    let n = COMMIT_TYPES.len();
    let parsed = input.split_once(": ").and_then(|(head, subject)| {
        let (kind, tail) = head.split_at(head.find(['(', '!']).unwrap_or(head.len()));
        let i = COMMIT_TYPES.iter().position(|t| *t == kind)?;
        Some((i, tail.to_string(), subject.to_string()))
    });
    let (next, tail, subject) = match parsed {
        Some((i, tail, subject)) => (if forward { (i + 1) % n } else { (i + n - 1) % n }, tail, subject),
        None => (if forward { 0 } else { n - 1 }, String::new(), input.clone()),
    };
    *input = format!("{}{tail}: {subject}", COMMIT_TYPES[next]);
}

/// The first line of the repository's `commit.template` that is not empty or a comment.
fn commit_template(repo: &Path) -> Option<String> {
    let path = config::expand(&git::config_get(repo, "commit.template")?);
    let text = std::fs::read_to_string(if path.is_relative() { repo.join(path) } else { path }).ok()?;
    text.lines().find(|l| !l.trim().is_empty() && !l.starts_with('#')).map(str::to_string)
}

/// What the reflog's `u` does for entry `i`: the confirmation text and the operation that undoes it.
/// A checkout goes back to the branch it came from; anything else moves the branch to where HEAD was before.
fn undo_of(entries: &[ReflogEntry], i: usize) -> Option<(String, Op)> {
    let e = entries.get(i)?;
    if let Some(from) = e.subject.strip_prefix("checkout: moving from ").and_then(|r| r.split(" to ").next()) {
        return Some((format!("undo \"{}\": check out {from} again?", e.subject), Op::Checkout(from.to_string())));
    }
    let before = entries.get(i + 1)?;
    let prompt = format!("undo \"{}\": move the branch back to {}? uncommitted changes are kept", e.subject, before.short);
    Some((prompt, Op::Reset(before.hash.clone())))
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
        let (color, hints, logs, cfg_ci) = (cfg.color, cfg.hints, cfg.logs, cfg.ci);
        let watch = Watcher::start(tx.clone());
        let mut app = App {
            marks: Marks::load(&cfg),
            cfg: Arc::new(cfg),
            repos: Vec::new(),
            visible: Vec::new(),
            repo_state: ListState::default(),
            file_state: ListState::default(),
            lang_state: ListState::default(),
            langs: HashMap::new(),
            lang_busy: HashSet::new(),
            focus: Focus::Repos,
            mode: Mode::Normal,
            filter: Filter::All,
            sort: Sort::Name,
            show_hidden: false,
            color,
            hints,
            logs,
            log_tick: 0,
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
            reflog: None,
            trees: None,
            remotes: None,
            tags: None,
            workflows: None,
            picker: None,
            code: None,
            run_form: None,
            wizard: None,
            remote_target: None,
            commit_warn: None,
            pending_msg: None,
            retry_msg: None,
            gh: None,
            text: None,
            finder: None,
            project: None,
            clone_slug: None,
            input_back: Mode::Normal,
            pending_select: None,
            diff_id: 0,
            watch: watch.as_ref().ok().cloned(),
            changed: HashMap::new(),
            tree: None,
            tree_done: None,
            quiet: HashSet::new(),
            auto_failed: false,
            auto_at: Instant::now(),
            ci: HashMap::new(),
            ci_busy: false,
            ci_auto: cfg_ci,
            launch: None,
            terms: Vec::new(),
            term: None,
            logger: Logger::open(),
            scan_paths: HashSet::new(),
            tx,
            rx,
        };
        if let Some(e) = app.cfg.error.clone() {
            app.log(Level::Error, format!("config.lua ignored: {e}"));
        }
        if let Err(e) = watch {
            app.log(Level::Warn, format!("file watching is off: {e}"));
        }
        app
    }

    pub fn log(&mut self, level: Level, text: impl Into<String>) {
        let text = text.into();
        self.logger.write(level, &text);
        self.log.push(LogLine { time: logger::clock(), level, text });
        self.log_tick = self.tick;
        if self.log.len() > LOG_LINES {
            self.log.remove(0);
        }
    }

    /// The newest log line while it is fresh and the log panel is hidden, so results are not missed.
    pub fn toast(&self) -> Option<&LogLine> {
        if self.logs || self.tick.wrapping_sub(self.log_tick) >= TOAST_TICKS {
            return None;
        }
        self.log.last()
    }

    /// The newest log line while it is fresh, whether or not the log panel is shown.
    pub fn fresh_log(&self) -> Option<&LogLine> {
        if self.tick.wrapping_sub(self.log_tick) >= TOAST_TICKS { None } else { self.log.last() }
    }

    /// The name of the remote an open input line is about, for its prompt.
    pub fn remote_name(&self) -> &str {
        self.remote_target.as_deref().unwrap_or("")
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
                    self.follow_diff(&repo);
                    match self.repos.iter_mut().find(|r| r.path == path) {
                        Some(slot) => *slot = repo,
                        None => {
                            // A repository that appeared after the scan (clone, init) is watched from now on.
                            if self.scan.is_none() {
                                self.watch_repo(&path);
                            }
                            self.repos.push(repo);
                        }
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
                    self.watch_repos();
                    if self.ci_auto {
                        self.start_ci(false);
                    }
                }
                Msg::Changed(path) => {
                    // An operation of Oxide's own reloads the repository when it ends.
                    if !self.busy.contains_key(&path) {
                        self.changed.insert(path, Instant::now());
                    }
                }
                Msg::Ci { path, branch, state } => match state {
                    Some(state) => {
                        self.ci.insert(path, (branch, state));
                    }
                    None => {
                        self.ci.remove(&path);
                    }
                },
                Msg::CiFailed(e) => {
                    self.ci_auto = false;
                    let first = e.lines().next().unwrap_or("").to_string();
                    self.log(Level::Warn, format!("CI status unavailable: {first}"));
                }
                Msg::CiDone => self.ci_busy = false,
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
                Msg::Reflog { id, result } => {
                    if let Some(v) = self.reflog.as_mut().filter(|v| v.id == id) {
                        match result {
                            Ok(entries) => {
                                // Keep the cursor on the same row after a reload (the newest entries shift it down).
                                let keep = v.state.selected().unwrap_or(0).min(entries.len().saturating_sub(1));
                                v.state.select(if entries.is_empty() { None } else { Some(keep) });
                                v.entries = Some(entries);
                                v.error = None;
                            }
                            Err(e) => v.error = Some(e),
                        }
                    }
                }
                Msg::Langs { path, key, result } => {
                    self.lang_busy.remove(&path);
                    self.langs.insert(path, (key, result));
                }
                Msg::Remotes { id, result } => {
                    if let Some(v) = self.remotes.as_mut().filter(|v| v.id == id) {
                        match result {
                            Ok(items) => {
                                let keep = v.state.selected().unwrap_or(0).min(items.len().saturating_sub(1));
                                v.state.select(if items.is_empty() { None } else { Some(keep) });
                                v.items = Some(items);
                                v.error = None;
                            }
                            Err(e) => v.error = Some(e),
                        }
                    }
                }
                Msg::Tags { id, result } => {
                    if let Some(v) = self.tags.as_mut().filter(|v| v.id == id) {
                        match result {
                            Ok(items) => {
                                let keep = v.state.selected().unwrap_or(0).min(items.len().saturating_sub(1));
                                v.state.select(if items.is_empty() { None } else { Some(keep) });
                                v.items = Some(items);
                                v.error = None;
                            }
                            Err(e) => v.error = Some(e),
                        }
                    }
                }
                Msg::WorkflowRuns { id, result } => {
                    if let Some(v) = self.workflows.as_mut().filter(|v| v.id == id) {
                        v.runs = Some(result);
                    }
                }
                Msg::TagRemote { id, result } => {
                    if let Some(v) = self.tags.as_mut().filter(|v| v.id == id) {
                        v.remote = Some(result);
                    }
                }
                Msg::Releases { id, result } => {
                    if let Some(v) = self.tags.as_mut().filter(|v| v.id == id) {
                        v.releases = Some(result);
                    }
                }
                Msg::Trees { id, result } => {
                    if let Some(v) = self.trees.as_mut().filter(|v| v.id == id) {
                        match result {
                            Ok(items) => {
                                let keep = v.state.selected().unwrap_or(0).min(items.len().saturating_sub(1));
                                v.state.select(if items.is_empty() { None } else { Some(keep) });
                                v.items = Some(items);
                                v.error = None;
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
                Msg::Removed(path) => {
                    self.repos.retain(|r| r.path != path);
                    self.rebuild();
                }
                Msg::Repos { id, result } => {
                    if let Some(v) = self.finder.as_mut().filter(|v| v.id == id) {
                        match result {
                            Ok(page) => {
                                v.list.select(if page.hits.len() == 0 { None } else { Some(0) });
                                (v.more, v.total, v.hits) = (page.more, page.total, Some(page.hits));
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
                Msg::Files { id, result } => self.files_loaded(id, result),
                Msg::Code { id, result } => self.code_loaded(id, result),
                Msg::Diff { id, result } => {
                    if let Some(view) = self.diff.as_mut().filter(|v| v.id == id) {
                        match result {
                            Ok(text) => {
                                view.diff = Some(diff::parse(&text, &view.file));
                                view.raw = text;
                            }
                            Err(e) => view.error = Some(e),
                        }
                    }
                }
                Msg::Op { path, label, result } => {
                    self.busy.remove(&path);
                    if self.branches.as_ref().is_some_and(|v| v.repo == path) {
                        self.load_branches(); // switching, creating or deleting changed the list
                    }
                    if result.is_ok() && label.ends_with("hunk") {
                        self.reload_diff(&path);
                    }
                    if self.reflog.as_ref().is_some_and(|v| v.repo == path) {
                        self.load_reflog(); // the operation added a line to the reflog
                    }
                    if self.remotes.as_ref().is_some_and(|v| v.repo == path) {
                        self.load_remotes();
                    }
                    if self.trees.as_ref().is_some_and(|v| v.main() == path || v.repo == path) {
                        self.load_trees();
                    }
                    if self.workflows.as_ref().is_some_and(|v| v.repo == path) {
                        self.load_workflows(); // a run was started or a workflow written
                    }
                    if self.tags.as_ref().is_some_and(|v| v.repo == path) {
                        self.load_tags(); // a tag was made, pushed or deleted, or a release made
                    }
                    if label == "commit" {
                        // A commit that failed (a hook said no) keeps its message for the next try.
                        let pending = self.pending_msg.take().filter(|(p, _)| *p == path);
                        self.retry_msg = if result.is_err() { pending } else { None };
                    }
                    let name = dir_name(&path);
                    // A fetch started by the timer only speaks up when it fails, and only once per round.
                    if self.quiet.remove(&path) {
                        if let Err(err) = result
                            && !self.auto_failed
                        {
                            self.auto_failed = true;
                            let first = err.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
                            self.log(Level::Warn, format!("auto-fetch: {name} failed ({first}); other failures this round are not logged"));
                        }
                        continue;
                    }
                    match result {
                        Ok(out) => {
                            self.log(Level::Ok, format!("{name}: {label} ok"));
                            // Hook output (pre-commit, commit-msg ...) is part of a commit's result; show more of it.
                            let shown = match label {
                                "commit" | "amend" => 12,
                                "set up" | "undo setup" | "new workflow" => 30, // one line per step of the plan
                                _ => 3,
                            };
                            for line in out.lines().filter(|l| !l.trim().is_empty()).take(shown) {
                                self.log(Level::Info, format!("  {}", line.trim()));
                            }
                        }
                        Err(err) => {
                            self.log(Level::Error, format!("{name}: {label} failed"));
                            let shown = match label {
                                "commit" | "amend" | "set up" => 12,
                                _ => 6,
                            };
                            for line in err.lines().filter(|l| !l.trim().is_empty()).take(shown) {
                                self.log(Level::Error, format!("  {}", line.trim()));
                            }
                        }
                    }
                }
            }
        }
    }

    // ---- file watching, automatic fetch and CI ----

    /// Starts watching the git directory of one repository, logging the failure (once) when the limit is hit.
    fn watch_repo(&mut self, path: &Path) {
        let Some(w) = self.watch.clone() else { return };
        if let Err(e) = w.watch_git(path) {
            self.log(Level::Warn, format!("cannot watch {}: {e} (raise fs.inotify.max_user_watches)", path.display()));
            self.watch = None; // every further repository would fail the same way
        }
    }

    fn watch_repos(&mut self) {
        let paths: Vec<PathBuf> = self.repos.iter().map(|r| r.path.clone()).collect();
        for path in paths {
            self.watch_repo(&path);
        }
    }

    /// Housekeeping run once per event-loop iteration: reloads repositories whose files changed,
    /// moves the working-tree watch to the selected repository and starts the timed fetch.
    pub fn background(&mut self) {
        self.reap_terms();
        let due: Vec<PathBuf> = self.changed.iter().filter(|(_, t)| t.elapsed() >= RELOAD_DELAY).map(|(p, _)| p.clone()).collect();
        for path in due {
            self.changed.remove(&path);
            if !self.busy.contains_key(&path) {
                self.reload(path);
            }
        }

        // Move the working-tree watch to the selected repository, but only once the selection has rested
        // for a moment, so scrolling through the list does not walk every tree on the way.
        if let (Some(w), Some(sel)) = (self.watch.clone(), self.selected().map(|r| r.path.clone()))
            && self.tree_done.as_ref() != Some(&sel)
        {
            let since = match &self.tree {
                Some((p, t)) if *p == sel => *t,
                _ => {
                    self.tree = Some((sel.clone(), Instant::now()));
                    Instant::now()
                }
            };
            if since.elapsed() >= TREE_DELAY {
                self.tree_done = Some(sel.clone());
                // A tree that cannot be watched (limit reached) just falls back to manual reloads with Ctrl+R.
                std::thread::spawn(move || {
                    let _ = w.watch_tree(&sel);
                });
            }
        }

        // Count the selected repository's languages once per commit; the result is cached until HEAD moves.
        if let Some(r) = self.selected()
            && let Some(key) = r.last_commit.split_whitespace().next().filter(|_| r.commit_time > 0).map(str::to_string)
            && self.langs.get(&r.path).is_none_or(|(k, _)| *k != key)
            && !self.lang_busy.contains(&r.path)
        {
            let path = r.path.clone();
            self.lang_busy.insert(path.clone());
            worker::spawn_langs(path, key, self.tx.clone());
        }

        let every = self.cfg.autofetch;
        if every > 0 && self.scan.is_none() && self.auto_at.elapsed() >= Duration::from_secs(every) {
            self.auto_at = Instant::now();
            self.auto_fetch();
        }
    }

    /// Fetches every listed repository that has a remote, without logging successes.
    fn auto_fetch(&mut self) {
        let jobs: Vec<(PathBuf, Op)> =
            self.listed().filter(|r| r.remote.is_some() && r.error.is_none()).filter(|r| !self.busy.contains_key(&r.path)).map(|r| (r.path.clone(), Op::Fetch)).collect();
        self.auto_failed = false;
        self.quiet.extend(jobs.iter().map(|(p, _)| p.clone()));
        self.run_ops(jobs);
        if self.ci_auto {
            self.start_ci(false);
        }
    }

    /// Asks GitHub for the CI state of the repositories in view that live on github.com.
    /// `manual` also reports when there is nothing to ask and turns automatic refreshes back on.
    fn start_ci(&mut self, manual: bool) {
        if manual {
            self.ci_auto = true;
        }
        if self.ci_busy {
            if manual {
                self.log(Level::Info, "CI states are already loading");
            }
            return;
        }
        let items: Vec<(PathBuf, String, String)> = self
            .visible
            .iter()
            .map(|&i| &self.repos[i])
            .filter(|r| r.error.is_none() && !r.ignored)
            .filter_map(|r| Some((r.path.clone(), r.slug.clone()?, r.branch.clone())))
            .take(CI_BATCH)
            .collect();
        if items.is_empty() {
            if manual {
                self.log(Level::Info, "no repository in this view has a github.com remote");
            }
            return;
        }
        if manual {
            self.log(Level::Info, format!("loading CI status of {} repositories", items.len()));
        }
        self.ci_busy = true;
        worker::spawn_ci(items, self.tx.clone());
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
        // The terminal gets every key, Ctrl+C included, except Ctrl+T which hides it.
        if self.mode == Mode::Terminal {
            return self.on_term_key(key);
        }
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
            Mode::Picker => self.on_picker_key(key),
            Mode::Code => self.on_code_key(key),
            Mode::History => self.on_history_key(key),
            Mode::Branches => self.on_branch_key(key),
            Mode::Reflog => self.on_reflog_key(key),
            Mode::Trees => self.on_trees_key(key),
            Mode::Remotes => self.on_remotes_key(key),
            Mode::GitHub => self.on_gh_key(key),
            Mode::Text => self.on_text_key(key),
            Mode::Finder => self.on_finder_key(key),
            Mode::Project => self.on_project_key(key),
            Mode::Tags => self.on_tags_key(key),
            Mode::Workflows => self.on_workflows_key(key),
            Mode::WorkflowRun => self.on_run_key(key),
            Mode::WorkflowNew => self.on_wizard_key(key),
            Mode::Terminal => {}
            Mode::Normal => self.on_normal_key(key),
        }
    }

    fn on_normal_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Esc => {
                if self.focus != Focus::Repos {
                    self.focus = Focus::Repos;
                } else if !self.search.is_empty() {
                    self.search.clear();
                    self.rebuild();
                }
            }
            KeyCode::Tab | KeyCode::Right => self.move_focus(true),
            KeyCode::BackTab | KeyCode::Left => self.move_focus(false),
            KeyCode::Char('h') => self.hints = !self.hints,
            KeyCode::Char('l') => self.logs = !self.logs,
            KeyCode::Char(':') => self.open_palette(),
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => self.open_palette(),
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
            KeyCode::Char('O') => self.open_reflog(),
            KeyCode::Char('W') => self.open_trees(),
            KeyCode::Char('V') => self.open_tags(),
            KeyCode::Char('N') => self.open_workflows(),
            KeyCode::Char('E') => self.open_amend(),
            KeyCode::Char('I') => self.open_init(),
            KeyCode::Char('i') => self.open_gh(Kind::Issues),
            KeyCode::Char('y') => self.open_gh(Kind::Pulls),
            KeyCode::Char('B') => self.browse_repo(),
            KeyCode::Char('S') => self.open_input(InputKind::GhSearch),
            KeyCode::Char('D') => {
                if self.selected().is_some() {
                    self.open_input(InputKind::DeleteRepo);
                }
            }
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
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => self.edit_highlighted(),
            KeyCode::Char('e') => self.open_browser(),
            KeyCode::Char('t' | 'T') => self.open_terminal(),
            KeyCode::Char('K') => self.open_project(),
            KeyCode::Char('X') => self.undo_setup(),
            KeyCode::Char('o') => self.open_files(),
            KeyCode::Char('/') => self.open_input(InputKind::Search),
            KeyCode::Char('R') => self.start_scan(),
            KeyCode::Char('w') => self.start_ci(true),
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => self.refresh_selected(),
            KeyCode::F(5) => self.refresh_selected(),
            KeyCode::Char('r') => self.open_remotes(),
            KeyCode::Char('F') => self.op_selected(Op::Fetch),
            KeyCode::Char('A') => self.fetch_visible(),
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => self.open_picker(),
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
                match self.focus {
                    Focus::Repos => self.focus = Focus::Files,
                    Focus::Files => self.toggle_stage(),
                    Focus::Langs => {}
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
            raw: String::new(),
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

    // ---- reflog ----

    fn open_reflog(&mut self) {
        let Some((repo, name)) = self.selected().map(|r| (r.path.clone(), r.name.clone())) else { return };
        self.reflog = Some(ReflogView { repo, name, entries: None, error: None, state: ListState::default(), id: 0 });
        self.mode = Mode::Reflog;
        self.load_reflog();
    }

    fn load_reflog(&mut self) {
        self.diff_id += 1;
        if let Some(v) = self.reflog.as_mut() {
            v.id = self.diff_id;
            worker::spawn_reflog(v.repo.clone(), v.id, self.tx.clone());
        }
    }

    fn on_reflog_key(&mut self, key: KeyEvent) {
        let Some(v) = self.reflog.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        let len = v.entries.as_ref().map_or(0, Vec::len);
        let repo = v.repo.clone();
        let picked = v.state.selected().and_then(|i| Some((i, v.entries.as_ref()?.get(i)?)));
        let delta = match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'O') => {
                self.reflog = None;
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Down | KeyCode::Char('j') => 1,
            KeyCode::Up | KeyCode::Char('k') => -1,
            KeyCode::PageDown | KeyCode::Char(' ') => 15,
            KeyCode::PageUp | KeyCode::Char('b') => -15,
            KeyCode::Home | KeyCode::Char('g') => -(1 << 30),
            KeyCode::End | KeyCode::Char('G') => 1 << 30,
            // Undo: go back to how things were before the highlighted entry happened.
            KeyCode::Char('u') => {
                let undo = picked.and_then(|(i, _)| undo_of(v.entries.as_deref()?, i));
                match undo {
                    Some((prompt, op)) => self.ask(prompt, vec![(repo, op)]),
                    None => self.log(Level::Warn, "nothing earlier in the reflog to go back to"),
                }
                return;
            }
            // Restore: move the branch to the highlighted entry's commit.
            KeyCode::Enter => {
                if let Some((_, e)) = picked {
                    let prompt = format!("move the current branch to {} ({})? uncommitted changes are kept", e.short, e.subject);
                    let hash = e.hash.clone();
                    self.ask(prompt, vec![(repo, Op::Reset(hash))]);
                }
                return;
            }
            KeyCode::Char('d') => {
                if let Some((_, e)) = picked {
                    let hash = e.hash.clone();
                    self.start_diff(repo, hash, DiffKind::Commit, false, Mode::Reflog);
                }
                return;
            }
            _ => return,
        };
        v.state.select(step(v.state.selected(), len, delta));
    }

    // ---- remotes ----

    fn open_remotes(&mut self) {
        let Some((repo, name)) = self.selected().map(|r| (r.path.clone(), r.name.clone())) else { return };
        self.remotes = Some(RemotesView { repo, name, items: None, error: None, state: ListState::default(), id: 0 });
        self.mode = Mode::Remotes;
        self.load_remotes();
    }

    // ---- tags and releases ----

    fn open_tags(&mut self) {
        let Some((repo, name, slug)) = self.selected().map(|r| (r.path.clone(), r.name.clone(), r.slug.clone())) else { return };
        self.tags = Some(TagsView { repo, name, slug, items: None, error: None, state: ListState::default(), remote: None, releases: None, id: 0 });
        self.mode = Mode::Tags;
        self.load_tags();
    }

    /// Loads the tags, then (separately, as they need the network) the remote's tags and the GitHub releases.
    fn load_tags(&mut self) {
        self.diff_id += 1;
        if let Some(v) = self.tags.as_mut() {
            v.id = self.diff_id;
            v.remote = None;
            v.releases = if v.slug.is_none() { Some(Err("no github.com remote".to_string())) } else { None };
            worker::spawn_tags(v.repo.clone(), v.slug.clone(), v.id, self.tx.clone());
        }
    }

    fn on_tags_key(&mut self, key: KeyEvent) {
        let Some(v) = self.tags.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        let len = v.items.as_ref().map_or(0, Vec::len);
        let repo = v.repo.clone();
        let slug = v.slug.clone();
        let picked = v.selected().map(|t| (t.name.clone(), t.hash.clone()));
        // What is known about the highlighted tag: on the remote, and its GitHub release.
        let on_remote = picked.as_ref().and_then(|(n, _)| v.remote.as_ref()?.as_ref().ok().map(|set| set.contains(n)));
        let released = picked.as_ref().and_then(|(n, _)| v.releases.as_ref()?.as_ref().ok()?.get(n)).is_some();
        let delta = match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'V') => {
                self.tags = None;
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Down | KeyCode::Char('j') => 1,
            KeyCode::Up | KeyCode::Char('k') => -1,
            KeyCode::Home | KeyCode::Char('g') => -(1 << 30),
            KeyCode::End | KeyCode::Char('G') => 1 << 30,
            KeyCode::Char('n') => {
                let suggestion = next_tag(v.items.as_deref().unwrap_or(&[]));
                self.open_input(InputKind::Tag);
                self.input = suggestion;
                return;
            }
            KeyCode::Char('f') | KeyCode::F(5) => return self.load_tags(),
            _ => {
                let Some((name, hash)) = picked else {
                    self.log(Level::Info, "no tag highlighted (press n to make one)");
                    return;
                };
                match key.code {
                    KeyCode::Enter => self.start_diff(repo, hash, DiffKind::Commit, false, Mode::Tags),
                    KeyCode::Char('p') => {
                        if on_remote == Some(true) {
                            self.log(Level::Info, format!("{name} is already on the remote"));
                        } else {
                            self.run_ops(vec![(repo, Op::PushTag(name))]);
                        }
                    }
                    KeyCode::Char('d') => {
                        let prompt = format!("delete tag {name} from this repository (it stays on the remote)?");
                        self.ask(prompt, vec![(repo, Op::DeleteTag { name, remote: false })]);
                    }
                    KeyCode::Char('D') => {
                        let prompt = format!("delete tag {name} here AND on the remote? others may already have it");
                        self.ask(prompt, vec![(repo, Op::DeleteTag { name, remote: true })]);
                    }
                    KeyCode::Char(c @ ('r' | 'R')) => {
                        let draft = c == 'R';
                        let kind = if draft { "draft release" } else { "release" };
                        match (slug, on_remote) {
                            (None, _) => self.log(Level::Warn, "releases need a github.com remote"),
                            (_, Some(false)) => self.log(Level::Warn, format!("{name} is not on the remote yet: push it first (p)")),
                            (_, None) => self.log(Level::Warn, "still asking the remote which tags it has: try again in a moment"),
                            _ if released => self.log(Level::Info, format!("{name} already has a release (o opens it)")),
                            (Some(slug), Some(true)) => {
                                let prompt = format!("create a GitHub {kind} for {name} with generated notes?");
                                self.ask(prompt, vec![(repo, Op::Release { tag: name, slug, draft })]);
                            }
                        }
                    }
                    KeyCode::Char('o') => match (slug, released) {
                        (Some(slug), true) => {
                            let args = vec!["release".to_string(), "view".into(), name.clone(), "-R".into(), slug, "--web".into()];
                            self.spawn_detached("gh", &args, &format!("release {name} opened in the browser"));
                        }
                        _ => self.log(Level::Info, format!("{name} has no GitHub release")),
                    },
                    _ => {}
                }
                return;
            }
        };
        v.state.select(step(v.state.selected(), len, delta));
    }

    /// Makes a tag on HEAD from `name` or `name message` (a message makes it an annotated tag).
    fn add_tag(&mut self, text: &str) {
        let Some(repo) = self.tags.as_ref().map(|v| v.repo.clone()) else { return };
        let text = text.trim();
        let (name, message) = text.split_once(char::is_whitespace).map_or((text, ""), |(n, m)| (n, m.trim()));
        if name.is_empty() {
            return;
        }
        self.run_ops(vec![(repo, Op::Tag { name: name.to_string(), message: message.to_string() })]);
    }

    fn load_remotes(&mut self) {
        self.diff_id += 1;
        if let Some(v) = self.remotes.as_mut() {
            v.id = self.diff_id;
            worker::spawn_remotes(v.repo.clone(), v.id, self.tx.clone());
        }
    }

    fn on_remotes_key(&mut self, key: KeyEvent) {
        let Some(v) = self.remotes.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        let len = v.items.as_ref().map_or(0, Vec::len);
        let repo = v.repo.clone();
        let picked = v.selected().map(|r| (r.name.clone(), r.url.clone()));
        let delta = match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'r') => {
                self.remotes = None;
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Down | KeyCode::Char('j') => 1,
            KeyCode::Up | KeyCode::Char('k') => -1,
            KeyCode::Home | KeyCode::Char('g') => -(1 << 30),
            KeyCode::End | KeyCode::Char('G') => 1 << 30,
            KeyCode::Char('a') => {
                self.open_input(InputKind::RemoteAdd);
                return;
            }
            _ => {
                let Some((name, url)) = picked else {
                    self.log(Level::Info, "no remote highlighted (press a to add one)");
                    return;
                };
                match key.code {
                    // Change the URL; the prompt starts with the current one.
                    KeyCode::Char('e' | 'u') => {
                        self.remote_target = Some(name);
                        self.open_input(InputKind::RemoteUrl);
                        self.input = url;
                    }
                    KeyCode::Char('n') | KeyCode::F(2) => {
                        self.remote_target = Some(name.clone());
                        self.open_input(InputKind::RemoteRename);
                        self.input = name;
                    }
                    KeyCode::Char('d') => {
                        let prompt = format!("remove remote {name} ({url})? its remote-tracking branches are deleted too");
                        self.ask(prompt, vec![(repo, Op::RemoteRemove(name))]);
                    }
                    KeyCode::Enter | KeyCode::Char('f') => self.run_ops(vec![(repo, Op::RemoteFetch(name))]),
                    _ => {}
                }
                return;
            }
        };
        v.state.select(step(v.state.selected(), len, delta));
    }

    /// Runs `op` on the repository whose remotes are open.
    fn remote_op(&mut self, op: Op) {
        if let Some(repo) = self.remotes.as_ref().map(|v| v.repo.clone()) {
            self.run_ops(vec![(repo, op)]);
        }
    }

    /// `text` is `<name> <url>`; a lone URL names the repository's first remote `origin`.
    fn add_remote(&mut self, text: &str) {
        let mut words = text.split_whitespace();
        let (first, second) = (words.next(), words.next());
        let have_any = self.remotes.as_ref().and_then(|v| v.items.as_ref()).is_some_and(|i| !i.is_empty());
        match (first, second) {
            (Some(name), Some(url)) => self.remote_op(Op::RemoteAdd { name: name.to_string(), url: url.to_string() }),
            (Some(url), None) if !have_any => self.remote_op(Op::RemoteAdd { name: "origin".to_string(), url: url.to_string() }),
            (Some(_), None) => self.log(Level::Warn, "give a name and a URL: <name> <url>"),
            _ => self.log(Level::Warn, "nothing typed, no remote added"),
        }
    }

    // ---- worktrees and submodules ----

    fn open_trees(&mut self) {
        let Some((repo, name)) = self.selected().map(|r| (r.path.clone(), r.name.clone())) else { return };
        self.trees = Some(TreesView { repo, name, items: None, error: None, state: ListState::default(), id: 0 });
        self.mode = Mode::Trees;
        self.load_trees();
    }

    fn load_trees(&mut self) {
        self.diff_id += 1;
        if let Some(v) = self.trees.as_mut() {
            v.id = self.diff_id;
            worker::spawn_trees(v.repo.clone(), v.id, self.tx.clone());
        }
    }

    fn on_trees_key(&mut self, key: KeyEvent) {
        let Some(v) = self.trees.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        let len = v.items.as_ref().map_or(0, Vec::len);
        let main = v.main();
        // The highlighted worktree as (folder, locked) and submodule as its folder, for the actions below.
        let picked = v.selected().map(|i| (i.path.clone(), matches!(i.kind, TreeKind::Linked { locked: true, .. }), matches!(i.kind, TreeKind::Main), matches!(i.kind, TreeKind::Submodule { .. })));
        let delta = match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'W') => {
                self.trees = None;
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Down | KeyCode::Char('j') => 1,
            KeyCode::Up | KeyCode::Char('k') => -1,
            KeyCode::PageDown | KeyCode::Char(' ') => 10,
            KeyCode::PageUp | KeyCode::Char('b') => -10,
            KeyCode::Home | KeyCode::Char('g') => -(1 << 30),
            KeyCode::End | KeyCode::Char('G') => 1 << 30,
            KeyCode::Char('n') => {
                self.open_input(InputKind::Worktree);
                return;
            }
            KeyCode::Char('p') => {
                self.run_ops(vec![(main, Op::PruneWorktrees)]);
                return;
            }
            KeyCode::Char('U') => {
                self.ask("initialise and update every submodule?".to_string(), vec![(main, Op::UpdateSubmodule(String::new()))]);
                return;
            }
            KeyCode::Enter => {
                match picked {
                    Some((path, _, _, false)) => self.jump_to_repo(&PathBuf::from(path)),
                    Some(_) => self.log(Level::Info, "submodules are not listed on the dashboard; press u to update one"),
                    None => {}
                }
                return;
            }
            KeyCode::Char('u') => {
                match picked {
                    Some((path, _, _, true)) => self.run_ops(vec![(main, Op::UpdateSubmodule(path))]),
                    Some(_) => self.log(Level::Warn, "u updates a submodule; the highlighted row is a worktree"),
                    None => {}
                }
                return;
            }
            KeyCode::Char(c @ ('d' | 'D')) => {
                match picked {
                    Some((path, locked, false, false)) => {
                        if locked {
                            self.log(Level::Warn, "that worktree is locked; unlock it with git worktree unlock first");
                        } else {
                            let force = c == 'D';
                            let how = if force { "REMOVE worktree (and its uncommitted changes)" } else { "remove worktree" };
                            self.ask(format!("{how} {path}?"), vec![(main, Op::RemoveWorktree { path: PathBuf::from(path), force })]);
                        }
                    }
                    Some((_, _, true, _)) => self.log(Level::Warn, "the main worktree is the repository itself and cannot be removed"),
                    Some(_) => self.log(Level::Warn, "submodules are not removed from here"),
                    None => {}
                }
                return;
            }
            _ => return,
        };
        v.state.select(step(v.state.selected(), len, delta));
    }

    /// Closes the open lists and selects `path` on the dashboard, when it is a listed repository.
    fn jump_to_repo(&mut self, path: &Path) {
        let known = self.repos.iter().any(|r| r.path == path);
        if !known {
            self.log(Level::Warn, format!("{} is not in the repository list (rescan with R, or add its folder as a root)", path.display()));
            return;
        }
        self.trees = None;
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
    }

    /// Creates a worktree for `branch` in a folder next to the repository, named `<repo>-<branch>`.
    fn add_worktree(&mut self, branch: &str) {
        let Some(v) = self.trees.as_ref() else { return };
        if branch.is_empty() {
            self.log(Level::Warn, "empty branch name, no worktree created");
            return;
        }
        let main = v.main();
        let base = main.file_name().map_or_else(|| "repo".to_string(), |n| n.to_string_lossy().into_owned());
        let path = main.parent().unwrap_or(Path::new(".")).join(format!("{base}-{}", branch.replace('/', "-")));
        if path.exists() {
            self.log(Level::Warn, format!("{} already exists", path.display()));
            return;
        }
        self.log(Level::Info, format!("adding worktree {}", path.display()));
        self.run_ops(vec![(main, Op::AddWorktree { path, branch: branch.to_string() })]);
    }

    // ---- commit helpers ----

    /// Prompts for the new message of the last commit; staged changes are folded into it.
    fn open_amend(&mut self) {
        let Some((has_commit, pushed)) = self.selected().map(|r| (r.commit_time > 0, r.upstream.is_some() && r.ahead == 0)) else { return };
        if !has_commit {
            self.log(Level::Warn, "no commit to amend");
            return;
        }
        if pushed {
            self.log(Level::Warn, "the last commit is already pushed: after amending it, publishing needs a force push");
        }
        self.open_input(InputKind::Amend);
        self.commit_warn = self.identity_warning();
    }

    /// Prefills the commit prompt with the message of a commit that just failed, else the first line of the
    /// repository's `commit.template`, and checks the identity the commit will be made with.
    fn prefill_commit(&mut self) {
        let Some(path) = self.selected().map(|r| r.path.clone()) else { return };
        if let Some((_, msg)) = self.retry_msg.take().filter(|(p, _)| *p == path) {
            self.input = msg;
            self.log(Level::Info, "the message of the failed commit is restored");
        } else if let Some(line) = commit_template(&path) {
            self.input = line;
        }
        self.commit_warn = self.identity_warning();
    }

    /// Explains why a commit made now may carry the wrong e-mail address, or None when it looks right.
    /// The expected addresses are the `email` entries of the config, else the global `user.email`.
    fn identity_warning(&self) -> Option<String> {
        let r = self.selected()?;
        let expected: Vec<String> = if self.cfg.emails.is_empty() {
            git::global_config("user.email").map(|e| e.to_lowercase()).into_iter().collect()
        } else {
            self.cfg.emails.clone()
        };
        match git::config_get(&r.path, "user.email") {
            None => Some("no user.email is set".to_string()),
            Some(e) if !expected.is_empty() && !expected.contains(&e.to_lowercase()) => Some(format!("committing as {e}, expected {}", expected.join(" or "))),
            Some(_) => None,
        }
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
        v.hits = None;
        v.error = None;
        let (id, scope, terms, page) = (v.id, v.scope, v.terms.clone(), v.page);
        worker::spawn_msg(self.tx.clone(), move || Msg::Repos { id, result: github::find(scope, &terms, page) });
    }

    /// Shows another page of the current search.
    fn finder_page(&mut self, page: usize) {
        if let Some(v) = self.finder.as_mut() {
            v.page = page;
        }
        self.load_finder();
    }

    fn on_finder_key(&mut self, key: KeyEvent) {
        let Some(v) = self.finder.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        let len = v.hits.as_ref().map_or(0, Hits::len);
        let delta = match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.finder = None;
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Char('/' | 'S') => {
                let query = if v.scope == Scope::Owner { String::new() } else { v.query.clone() };
                self.open_input(InputKind::GhSearch);
                self.input = query;
                return;
            }
            KeyCode::Char('n' | ']') | KeyCode::Right => {
                if v.more {
                    let next = v.page + 1;
                    self.finder_page(next);
                }
                return;
            }
            KeyCode::Char('p' | '[') | KeyCode::Left => {
                if v.page > 1 {
                    let prev = v.page - 1;
                    self.finder_page(prev);
                }
                return;
            }
            KeyCode::Char('r') => {
                self.load_finder();
                return;
            }
            KeyCode::Char('o') => {
                let at = v.list.selected().unwrap_or(0);
                match v.hits.as_ref() {
                    Some(Hits::Users(u)) => {
                        if let Some(u) = u.get(at) {
                            let url = format!("https://github.com/{}", u.login);
                            self.spawn_detached("xdg-open", &[url], "opened in the browser");
                        }
                    }
                    Some(Hits::Files(f)) => {
                        if let Some(f) = f.get(at) {
                            let args = ["browse".to_string(), f.path.clone(), "-R".into(), f.slug.clone()];
                            self.spawn_detached("gh", &args, "opened in the browser");
                        }
                    }
                    Some(Hits::Repos(_)) => {
                        if let Some(slug) = v.selected_slug() {
                            self.spawn_detached("gh", &["browse".to_string(), "-R".into(), slug], "opened in the browser");
                        }
                    }
                    None => {}
                }
                return;
            }
            KeyCode::Enter => {
                // A user opens the list of their repositories; repositories and files lead to the repository.
                let login = match v.hits.as_ref() {
                    Some(Hits::Users(u)) => v.list.selected().and_then(|i| u.get(i)).map(|u| u.login.clone()),
                    _ => None,
                };
                if let Some(login) = login {
                    *v = FinderView::new(format!("repositories of {login}"), Scope::Owner, login);
                    self.load_finder();
                } else if let Some(slug) = v.selected_slug() {
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

    /// Deletes the selected repository's folder, but only when `typed` is exactly its name.
    fn delete_selected(&mut self, typed: &str) {
        let Some((path, name)) = self.selected().map(|r| (r.path.clone(), r.name.clone())) else { return };
        if typed != name {
            self.log(Level::Info, format!("{name}: not deleted (the name typed did not match)"));
            return;
        }
        self.log(Level::Warn, format!("{name}: deleting {} from disk", path.display()));
        self.run_ops(vec![(path, Op::Delete)]);
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
            KeyCode::Char('e') => self.edit_from_diff(),
            // Stage the hunk at the top of the screen (unstaged view), or unstage it (staged view).
            KeyCode::Char('s') => {
                if !v.stageable() {
                    self.log(Level::Warn, "only the staged or unstaged diff of a file has hunks to stage");
                    return;
                }
                let reverse = v.kind == DiffKind::Staged;
                let picked = v.hunk().and_then(|k| diff::hunk_patch(&v.raw, k)).map(|patch| (v.repo.clone(), patch));
                match picked {
                    Some((repo, patch)) => self.run_ops(vec![(repo, Op::Hunk { patch, reverse })]),
                    None => self.log(Level::Warn, "no hunk to stage here"),
                }
            }
            _ => {}
        }
    }

    /// Loads the open diff again (after a hunk was staged or unstaged), keeping the scroll position.
    fn reload_diff(&mut self, repo: &Path) {
        self.diff_id += 1;
        if let Some(v) = self.diff.as_mut().filter(|v| v.repo == repo) {
            v.id = self.diff_id;
            worker::spawn_diff(v.repo.clone(), v.file.clone(), v.kind, v.id, self.tx.clone());
        }
    }

    /// Keeps an open staged/unstaged diff pointing at a version of the file that still has changes:
    /// once its last hunk moved to the other side, the viewer follows it.
    fn follow_diff(&mut self, repo: &Repo) {
        let Some(v) = self.diff.as_mut().filter(|v| v.repo == repo.path && v.stageable()) else { return };
        let Some(f) = repo.files.iter().find(|f| f.path == v.file) else { return };
        v.both = f.staged() && f.unstaged() && !f.conflict();
        let wanted = if v.kind == DiffKind::Unstaged && !f.unstaged() && f.staged() {
            Some(DiffKind::Staged)
        } else if v.kind == DiffKind::Staged && !f.staged() && f.unstaged() {
            Some(DiffKind::Unstaged)
        } else {
            None
        };
        if let Some(kind) = wanted {
            v.kind = kind;
            self.reload_diff(&repo.path);
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
                    let code = palette::ENTRIES[entry].code;
                    self.on_normal_key(KeyEvent::new(code, if palette::ENTRIES[entry].ctrl { KeyModifiers::CONTROL } else { KeyModifiers::NONE }));
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
            if self.mode == Mode::WorkflowRun {
                // The run is started: the form and the workflows list are done with, not left open behind it.
                self.run_form = None;
                self.workflows = None;
                self.mode = Mode::Normal;
            }
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
            // In a commit message, Tab walks through the conventional-commit types (feat, fix, docs ...).
            KeyCode::Tab | KeyCode::BackTab if matches!(kind, InputKind::Commit | InputKind::Amend) => {
                cycle_type(&mut self.input, key.code == KeyCode::Tab);
            }
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
        self.input_back = if matches!(self.mode, Mode::Branches | Mode::Finder | Mode::Trees | Mode::Remotes | Mode::Tags) { self.mode } else { Mode::Normal };
        self.mode = Mode::Input(kind);
    }

    fn open_commit(&mut self) {
        match self.selected().map(|r| r.staged()) {
            None => {}
            Some(0) => self.log(Level::Warn, "nothing staged to commit (stage files with Space or a)"),
            Some(_) => {
                self.open_input(InputKind::Commit);
                self.prefill_commit();
            }
        }
    }

    fn submit(&mut self, kind: InputKind) {
        self.mode = self.input_back;
        let text = std::mem::take(&mut self.input).trim().to_string();
        match kind {
            InputKind::Init => self.init_repo(&text),
            InputKind::GhSearch => {
                let (scope, terms) = github::parse(&text);
                if scope != Scope::Repos && terms.is_empty() {
                    self.log(Level::Warn, "nothing to look for: type a name after user:, repo: or file:");
                    return;
                }
                self.finder = Some(FinderView::new(text, scope, terms));
                self.mode = Mode::Finder;
                self.load_finder();
            }
            InputKind::CloneTo => self.clone_repo(&text),
            InputKind::DeleteRepo => self.delete_selected(&text),
            InputKind::Branch if text.is_empty() => self.log(Level::Warn, "empty branch name, nothing created"),
            InputKind::Branch => {
                if let Some(repo) = self.branches.as_ref().map(|v| v.repo.clone()) {
                    self.run_ops(vec![(repo, Op::NewBranch(text))]);
                }
            }
            InputKind::Search => {} // already applied while typing
            InputKind::Commit if text.is_empty() => self.log(Level::Warn, "empty commit message, commit cancelled"),
            InputKind::Commit => {
                self.pending_msg = self.selected().map(|r| (r.path.clone(), text.clone()));
                self.op_selected(Op::Commit(text));
            }
            InputKind::Amend => self.op_selected(Op::Amend(text)),
            InputKind::Worktree => self.add_worktree(&text),
            InputKind::RemoteAdd => self.add_remote(&text),
            InputKind::RemoteUrl => match self.remote_target.take() {
                Some(name) if !text.is_empty() => self.remote_op(Op::RemoteSetUrl { name, url: text }),
                _ => self.log(Level::Warn, "empty URL, remote left as it was"),
            },
            InputKind::RemoteRename => match self.remote_target.take() {
                Some(old) if !text.is_empty() && text != old => self.remote_op(Op::RemoteRename { old, new: text }),
                _ => self.log(Level::Info, "remote name unchanged"),
            },
            InputKind::Merge => self.op_selected(Op::Merge(text)),
            InputKind::Tag => self.add_tag(&text),
        }
    }

    fn set_filter(&mut self, filter: Filter) {
        self.filter = filter;
        self.rebuild();
    }

    /// Moves the focus along repositories → languages → changes; `Tab` and `→` go forward and wrap
    /// (`→` stops at the end), `Shift+Tab` and `←` go back.
    fn move_focus(&mut self, forward: bool) {
        const ORDER: [Focus; 3] = [Focus::Repos, Focus::Langs, Focus::Files];
        let at = ORDER.iter().position(|f| *f == self.focus).unwrap_or(0);
        self.focus = ORDER[if forward { (at + 1) % ORDER.len() } else { (at + ORDER.len() - 1) % ORDER.len() }];
    }

    fn move_sel(&mut self, delta: isize) {
        match self.focus {
            Focus::Repos => {
                let next = step(self.repo_state.selected(), self.visible.len(), delta);
                if next != self.repo_state.selected() {
                    self.repo_state.select(next);
                    self.file_state.select(None); // a different repository: start at its first file
                    self.lang_state.select(None);
                    self.clamp_files();
                }
            }
            Focus::Langs => {
                let len = self.selected().and_then(|r| self.langs.get(&r.path)).map_or(0, |(_, b)| b.as_ref().map_or(0, |b| b.langs.len()));
                self.lang_state.select(step(self.lang_state.selected(), len, delta));
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

    /// Opens the new-project menu for the selected repository, which must be empty.
    fn open_project(&mut self) {
        let Some((path, name)) = self.selected().map(|r| (r.path.clone(), r.name.clone())) else { return };
        if !scaffold::is_empty(&path) {
            self.log(Level::Warn, format!("{name} is not empty: the new-project menu needs a repository with no files"));
            return;
        }
        let holder = git::global_config("user.name").unwrap_or_default();
        self.project = Some(ProjectView {
            repo: path,
            step: ProjectStep::Lang,
            query: String::new(),
            lang_state: ListState::default().with_selected(Some(0)),
            lang: 0,
            variant_state: ListState::default().with_selected(Some(0)),
            license_state: ListState::default().with_selected(Some(0)),
            package: String::new(),
            package_error: None,
            packages: Vec::new(),
            holder,
            editing: false,
            year: logger::year(),
            commit: false,
            plan: None,
        });
        self.mode = Mode::Project;
    }

    /// Asks to undo the last new-project setup of the selected repository, if one is recorded and still undoable.
    fn undo_setup(&mut self) {
        let Some((path, name)) = self.selected().map(|r| (r.path.clone(), r.name.clone())) else { return };
        let Some(record) = git::setup_record(&path) else {
            self.log(Level::Info, format!("{name}: no project setup to undo (K makes one)"));
            return;
        };
        match git::undo_question(&path, &record) {
            Ok(question) => self.ask(format!("{name}: {question}?"), vec![(path, Op::UndoSetup)]),
            Err(why) => self.log(Level::Warn, format!("{name}: {why}")),
        }
    }

    fn close_project(&mut self) {
        self.project = None;
        self.mode = Mode::Normal;
    }

    /// Keys of the new-project menu; each page has its own, `Esc` goes back one page.
    fn on_project_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(v) = self.project.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        let delta = match key.code {
            KeyCode::Down => 1,
            KeyCode::Up => -1,
            KeyCode::Char('n') if ctrl => 1,
            KeyCode::Char('p') if ctrl => -1,
            KeyCode::PageDown => 8,
            KeyCode::PageUp => -8,
            _ => 0,
        };
        match v.step {
            ProjectStep::Lang => {
                let choices = v.choices();
                match key.code {
                    KeyCode::Esc if !v.query.is_empty() => v.query.clear(),
                    KeyCode::Esc => return self.close_project(),
                    KeyCode::Enter => match v.lang_state.selected().and_then(|i| choices.get(i)) {
                        Some(Choice::Lang(i)) => {
                            v.lang = *i;
                            v.variant_state.select(Some(0));
                            v.package = scaffold::default_package(&v.repo, &langs()[*i], &v.holder).unwrap_or_default();
                            v.package_error = None;
                            v.packages.clear();
                            v.step = v.after_lang();
                            return;
                        }
                        // The menu is only for the common case; anything else is done by hand.
                        Some(Choice::Other) => {
                            self.close_project();
                            return self.open_terminal_in(None);
                        }
                        None => return,
                    },
                    KeyCode::Backspace => {
                        v.query.pop();
                    }
                    KeyCode::Char('u') if ctrl => v.query.clear(),
                    KeyCode::Char(c) if !ctrl => v.query.push(c),
                    _ if delta != 0 => {
                        v.lang_state.select(step(v.lang_state.selected(), choices.len(), delta));
                        return;
                    }
                    _ => return,
                }
                // The query changed: start from the best match again.
                let n = v.choices().len();
                v.lang_state.select(step(Some(0), n, 0));
            }
            ProjectStep::Package => match key.code {
                KeyCode::Esc => v.step = ProjectStep::Lang,
                KeyCode::Enter => match scaffold::parse_packages(&v.package, langs()[v.lang].package.map_or(PackageKind::Packages, |p| p.kind)) {
                    Ok(packages) => {
                        v.packages = packages;
                        v.package_error = None;
                        v.step = v.after_package();
                    }
                    Err(e) => v.package_error = Some(e),
                },
                KeyCode::Backspace => {
                    v.package.pop();
                    v.package_error = None;
                }
                KeyCode::Char('u') if ctrl => v.package.clear(),
                KeyCode::Char(c) if !ctrl => {
                    v.package.push(c);
                    v.package_error = None;
                }
                _ => {}
            },
            ProjectStep::Variant => match key.code {
                KeyCode::Esc | KeyCode::Backspace => v.step = if langs()[v.lang].package.is_some() { ProjectStep::Package } else { ProjectStep::Lang },
                KeyCode::Enter => v.step = ProjectStep::License,
                _ => v.variant_state.select(step(v.variant_state.selected(), langs()[v.lang].variants.len(), delta)),
            },
            ProjectStep::License if v.editing => match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Tab => v.editing = false,
                KeyCode::Backspace => {
                    v.holder.pop();
                }
                KeyCode::Char('u') if ctrl => v.holder.clear(),
                KeyCode::Char(c) if !ctrl => v.holder.push(c),
                _ => {}
            },
            ProjectStep::License => match key.code {
                KeyCode::Esc | KeyCode::Backspace => v.step = v.before_license(),
                KeyCode::Tab => v.editing = true,
                KeyCode::Enter => {
                    let lang = &langs()[v.lang];
                    let variant = &lang.variants[v.variant_state.selected().unwrap_or(0)];
                    v.plan = Some(Plan::build(&v.repo, lang, variant, &v.packages, v.license(), v.holder.trim(), v.year));
                    v.step = ProjectStep::Confirm;
                }
                // One extra row after the licenses: no LICENSE file.
                _ => v.license_state.select(step(v.license_state.selected(), LICENSES.len() + 1, delta)),
            },
            ProjectStep::Confirm => match key.code {
                KeyCode::Esc | KeyCode::Backspace => v.step = ProjectStep::License,
                KeyCode::Char('c' | ' ') => v.commit = !v.commit,
                KeyCode::Enter => {
                    let (repo, commit) = (v.repo.clone(), v.commit);
                    let Some(plan) = v.plan.take() else { return };
                    self.close_project();
                    self.run_ops(vec![(repo, Op::Setup { plan: Box::new(plan), commit })]);
                }
                _ => {}
            },
        }
    }

    /// Shows the embedded terminal: from the changes list, in the folder of the highlighted file, else in the
    /// repository.
    fn open_terminal(&mut self) {
        let folder = match (self.focus, self.selected()) {
            (Focus::Files, Some(r)) => self.file_state.selected().and_then(|i| r.files.get(i)).map(|f| folder_of(&r.path, &f.path)),
            _ => None,
        };
        self.open_terminal_in(folder);
    }

    /// Shows the shell of the selected repository that runs in `dir` (the repository itself when None),
    /// starting it on first use. Each folder has its own shell, which keeps running while hidden.
    fn open_terminal_in(&mut self, dir: Option<PathBuf>) {
        let Some(path) = self.selected().map(|r| r.path.clone()) else { return };
        let dir = dir.unwrap_or_else(|| path.clone());
        let i = match self.terms.iter().position(|t| t.repo == path && t.dir == dir) {
            Some(i) => i,
            None => {
                // The size is corrected to the real panel by the first frame.
                let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
                match Term::spawn(&path, &dir, rows.saturating_sub(2), cols.saturating_sub(2)) {
                    Ok(t) => {
                        self.terms.push(t);
                        self.terms.len() - 1
                    }
                    Err(e) => return self.log(Level::Error, e),
                }
            }
        };
        self.term = Some(i);
        self.mode = Mode::Terminal;
    }

    /// Keys while the terminal is shown: Ctrl+T hides it, Shift+PageUp/PageDown scroll its history,
    /// everything else goes to the shell.
    fn on_term_key(&mut self, key: KeyEvent) {
        let Some(t) = self.term.and_then(|i| self.terms.get_mut(i)) else {
            self.mode = Mode::Normal;
            return;
        };
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => self.hide_terminal(),
            KeyCode::PageUp if shift => t.scroll(10),
            KeyCode::PageDown if shift => t.scroll(-10),
            _ => t.send_key(key),
        }
    }

    /// Leaves the terminal for the dashboard; the shell keeps running. Its commands may have changed the repository.
    fn hide_terminal(&mut self) {
        self.mode = Mode::Normal;
        if let Some(path) = self.term.take().and_then(|i| self.terms.get(i)).map(|t| t.repo.clone()) {
            self.reload(path);
        }
    }

    /// Drops shells that exited; when the shown one did, the dashboard comes back.
    fn reap_terms(&mut self) {
        if self.terms.iter().all(Term::alive) {
            return;
        }
        let shown = self.term.and_then(|i| self.terms.get(i)).map(|t| (t.repo.clone(), t.dir.clone()));
        let gone: Vec<PathBuf> = self.terms.iter().filter(|t| !t.alive()).map(|t| t.repo.clone()).collect();
        self.terms.retain(Term::alive);
        self.term = shown.and_then(|(repo, dir)| self.terms.iter().position(|t| t.repo == repo && t.dir == dir));
        if self.mode == Mode::Terminal && self.term.is_none() {
            self.mode = Mode::Normal;
        }
        for path in gone {
            self.reload(path);
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
        self.reload_code(repo);
        self.reload_diff(repo);
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
/// The folder of `file` (a path inside `repo`; `dir/` for a whole untracked folder) as an absolute path. A file
/// that is gone, with its folder, gives the nearest folder that still exists, at worst the repository.
fn folder_of(repo: &Path, file: &str) -> PathBuf {
    let mut dir = repo.join(file);
    if !file.ends_with('/') {
        dir.pop(); // a collapsed untracked folder is listed as `dir/` and is the folder itself
    }
    while dir != repo && !dir.is_dir() {
        dir.pop();
    }
    if dir.starts_with(repo) { dir } else { repo.to_path_buf() }
}

/// The name to suggest for the next tag: the newest tag with its last number raised (`v0.2.0` -> `v0.2.1`),
/// or `v0.1.0` when nothing looks like a version yet.
fn next_tag(tags: &[Tag]) -> String {
    for t in tags {
        let (prefix, rest) = match t.name.strip_prefix('v') {
            Some(r) => ("v", r),
            None => ("", t.name.as_str()),
        };
        let parts: Vec<&str> = rest.split('.').collect();
        let Some(last) = parts.last().and_then(|p| p.parse::<u64>().ok()) else { continue };
        if parts.len() < 2 || parts[..parts.len() - 1].iter().any(|p| p.parse::<u64>().is_err()) {
            continue;
        }
        return format!("{prefix}{}.{}", parts[..parts.len() - 1].join("."), last + 1);
    }
    "v0.1.0".to_string()
}

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
