//! Tasks (`M`): the menu of a repository's build, test and other commands, and the pane that runs one and shows
//! its output as it arrives.

use std::time::Instant;

use super::*;
use crate::tasks::{self, Handle, Task};

/// The output pane keeps this many lines; older ones are dropped (and counted).
const KEPT: usize = 20_000;

/// The task menu of one repository.
pub struct TasksView {
    pub repo: PathBuf,
    pub name: String,
    pub items: Vec<Task>,
    pub state: ListState,
}

/// Where a task is.
#[derive(Clone, Copy, PartialEq)]
pub enum TaskState {
    Running,
    /// `code` is None when a signal ended it.
    Ended { code: Option<i32>, secs: f64 },
}

/// A task that is running or has run, with its output.
pub struct TaskView {
    pub repo: PathBuf,
    pub name: String,
    pub cmd: String,
    pub lines: Vec<String>,
    /// Lines dropped from the top because the output grew past `KEPT`.
    pub dropped: usize,
    pub state: TaskState,
    /// The pane sticks to the last line while output arrives.
    pub follow: bool,
    pub cancelled: bool,
    pub scroll: usize,
    pub hscroll: usize,
    /// Rows that fit on screen; set by the renderer, used for page jumps.
    pub height: usize,
    pub started: Instant,
    /// The process; None once it has ended.
    handle: Option<Handle>,
    id: u64,
}

impl App {
    /// `M`: lists the tasks of the selected repository.
    pub(super) fn open_tasks(&mut self) {
        let Some((repo, name)) = self.selected().map(|r| (r.path.clone(), r.name.clone())) else { return };
        let items = tasks::detect(&repo, &self.cfg.tasks);
        let state = ListState::default().with_selected(if items.is_empty() { None } else { Some(0) });
        self.tasks = Some(TasksView { repo, name, items, state });
        self.mode = Mode::Tasks;
    }

    pub(super) fn on_tasks_key(&mut self, key: KeyEvent) {
        let Some(v) = self.tasks.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        let n = v.items.len();
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.tasks = None;
                self.mode = Mode::Normal;
            }
            KeyCode::Down | KeyCode::Char('j') if n > 0 => v.state.select(Some((v.state.selected().unwrap_or(0) + 1).min(n - 1))),
            KeyCode::Up | KeyCode::Char('k') if n > 0 => v.state.select(Some(v.state.selected().unwrap_or(0).saturating_sub(1))),
            KeyCode::Home | KeyCode::Char('g') if n > 0 => v.state.select(Some(0)),
            KeyCode::End | KeyCode::Char('G') if n > 0 => v.state.select(Some(n - 1)),
            KeyCode::Enter => {
                let picked = v.state.selected().and_then(|i| v.items.get(i)).map(|t| (v.repo.clone(), t.name.clone(), t.cmd.clone()));
                if let Some((repo, name, cmd)) = picked {
                    self.start_task(repo, name, cmd);
                }
            }
            // The output of the last task, which may still be running.
            KeyCode::Char('o') => {
                if self.task.is_some() {
                    self.tasks = None;
                    self.mode = Mode::Task;
                } else {
                    self.log(Level::Info, "no task has run yet");
                }
            }
            _ => {}
        }
    }

    /// Starts `cmd` in `repo` and shows its output. Only one task runs at a time.
    fn start_task(&mut self, repo: PathBuf, name: String, cmd: String) {
        if self.task.as_ref().is_some_and(|t| t.state == TaskState::Running) {
            return self.log(Level::Warn, "a task is still running: M, then o shows it (c stops it)");
        }
        self.diff_id += 1;
        let id = self.diff_id;
        match tasks::spawn(&repo, &cmd, id, self.tx.clone()) {
            Ok(handle) => {
                self.task = Some(TaskView {
                    repo,
                    name,
                    cmd,
                    lines: Vec::new(),
                    dropped: 0,
                    state: TaskState::Running,
                    follow: true,
                    cancelled: false,
                    scroll: 0,
                    hscroll: 0,
                    height: 0,
                    started: Instant::now(),
                    handle: Some(handle),
                    id,
                });
                self.tasks = None;
                self.mode = Mode::Task;
            }
            Err(e) => self.log(Level::Error, e),
        }
    }

    pub(super) fn task_lines(&mut self, id: u64, lines: Vec<String>) {
        let Some(v) = self.task.as_mut().filter(|v| v.id == id) else { return };
        v.lines.extend(lines);
        if v.lines.len() > KEPT {
            let extra = v.lines.len() - KEPT;
            v.lines.drain(..extra);
            v.dropped += extra;
            v.scroll = v.scroll.saturating_sub(extra);
        }
    }

    pub(super) fn task_done(&mut self, id: u64, code: Option<i32>, secs: f64) {
        let Some(v) = self.task.as_mut().filter(|v| v.id == id) else { return };
        v.state = TaskState::Ended { code, secs };
        v.handle = None;
        let (name, repo) = (v.name.clone(), v.repo.clone());
        let (level, text) = match (v.cancelled, code) {
            (true, _) => (Level::Warn, format!("{name}: stopped after {secs:.1} s")),
            (false, Some(0)) => (Level::Ok, format!("{name}: passed in {secs:.1} s")),
            (false, Some(c)) => (Level::Error, format!("{name}: failed (exit {c}) in {secs:.1} s")),
            (false, None) => (Level::Error, format!("{name}: ended by a signal after {secs:.1} s")),
        };
        self.log(level, text);
        self.reload(repo); // a formatter or a build may have changed files
    }

    pub(super) fn on_task_key(&mut self, key: KeyEvent) {
        let Some(v) = self.task.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        let page = v.height.saturating_sub(1).max(1);
        let max = v.lines.len().saturating_sub(v.height);
        let at = if v.follow { max } else { v.scroll.min(max) };
        let scroll_to = |v: &mut TaskView, to: usize| {
            v.scroll = to.min(max);
            v.follow = v.scroll >= max;
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                let running = v.state == TaskState::Running;
                self.mode = Mode::Normal;
                if running {
                    self.log(Level::Info, "the task keeps running: M, then o shows it");
                } else {
                    self.task = None;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => scroll_to(v, at + 1),
            KeyCode::Up | KeyCode::Char('k') => scroll_to(v, at.saturating_sub(1)),
            KeyCode::PageDown | KeyCode::Char(' ') => scroll_to(v, at + page),
            KeyCode::PageUp | KeyCode::Char('b') => scroll_to(v, at.saturating_sub(page)),
            KeyCode::Home | KeyCode::Char('g') => scroll_to(v, 0),
            KeyCode::End | KeyCode::Char('G') => scroll_to(v, max),
            KeyCode::Right | KeyCode::Char('l') => v.hscroll += 8,
            KeyCode::Left | KeyCode::Char('h') => v.hscroll = v.hscroll.saturating_sub(8),
            KeyCode::Char('0') => v.hscroll = 0,
            KeyCode::Char('c') => {
                if let Some(h) = v.handle.as_ref().filter(|_| v.state == TaskState::Running) {
                    h.cancel();
                    v.cancelled = true;
                }
            }
            KeyCode::Char('r') => {
                if v.state != TaskState::Running {
                    let (repo, name, cmd) = (v.repo.clone(), v.name.clone(), v.cmd.clone());
                    self.start_task(repo, name, cmd);
                }
            }
            _ => {}
        }
    }
}
