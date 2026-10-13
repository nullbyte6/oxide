//! Files: the finder (`Ctrl+P`) that lists every file of the selected repository and opens one, and the code
//! viewer (read-only, highlighted, with a cursor line, search and go to line).

use super::*;
use crate::{code::Loaded, palette, syntax::Tok};

/// The finder shows at most this many matches.
const SHOWN: usize = 300;

/// The file finder: a query and the files that match it.
pub struct PickerView {
    pub repo: PathBuf,
    pub name: String,
    /// None while the file list is loading.
    pub files: Option<Vec<String>>,
    pub error: Option<String>,
    pub query: String,
    /// `(index into files, positions of the matched characters in the path)`, best first.
    pub shown: Vec<(usize, Vec<usize>)>,
    pub state: ListState,
    /// Files with uncommitted changes, which come first while nothing is typed.
    pub changed: HashSet<String>,
    id: u64,
}

/// What the line at the bottom of the viewer is asking for.
#[derive(Clone, Copy, PartialEq)]
pub enum Prompt {
    Find,
    Goto,
}

/// One file in the code viewer.
pub struct CodeView {
    pub repo: PathBuf,
    pub file: String,
    /// None while the file is loading.
    pub lines: Option<Vec<Vec<(Tok, String)>>>,
    plain: Vec<String>,
    /// The file is longer than what is shown.
    pub cut: bool,
    pub error: Option<String>,
    /// The highlighted line (0-based); the editor opens there.
    pub cursor: usize,
    /// First visible line, and how many columns the code is scrolled to the left.
    pub scroll: usize,
    pub hscroll: usize,
    /// Rows that fit on screen; set by the renderer, used for page jumps.
    pub height: usize,
    /// The text being typed at the bottom, if any.
    pub prompt: Option<(Prompt, String)>,
    /// The last search, and the lines that contain it.
    pub query: String,
    pub hits: Vec<usize>,
    /// The line to put the cursor on once the file has loaded.
    goto: Option<usize>,
    /// The mode to return to when the viewer closes.
    back: Mode,
    id: u64,
}

impl CodeView {
    pub fn len(&self) -> usize {
        self.lines.as_ref().map_or(0, Vec::len)
    }

    /// Scrolls just enough to see the cursor.
    fn reveal(&mut self) {
        let h = self.height.max(1);
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + h {
            self.scroll = self.cursor + 1 - h;
        }
    }

    /// Scrolls so that the cursor is in the middle of the screen.
    fn center(&mut self) {
        self.scroll = self.cursor.saturating_sub(self.height.max(1) / 2);
    }

    fn move_to(&mut self, line: usize) {
        self.cursor = line.min(self.len().saturating_sub(1));
        self.reveal();
    }

    /// Puts the cursor on the next (or previous) line that holds the search, wrapping around.
    fn next_hit(&mut self, forward: bool) -> bool {
        let found = if forward {
            self.hits.iter().copied().find(|&h| h > self.cursor).or(self.hits.first().copied())
        } else {
            self.hits.iter().copied().rev().find(|&h| h < self.cursor).or(self.hits.last().copied())
        };
        if let Some(h) = found {
            self.cursor = h;
            if h < self.scroll || h >= self.scroll + self.height.max(1) {
                self.center();
            }
        }
        found.is_some()
    }
}

/// The files that match `query`, best first. A match inside the file name outranks one spread over folders;
/// with nothing typed the files with changes come first.
fn rank(files: &[String], changed: &HashSet<String>, query: &str) -> Vec<(usize, Vec<usize>)> {
    let q: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        let mut order: Vec<usize> = (0..files.len()).collect();
        order.sort_by_key(|&i| !changed.contains(&files[i]));
        return order.into_iter().take(SHOWN).map(|i| (i, Vec::new())).collect();
    }
    let mut found: Vec<(i32, usize, Vec<usize>)> = files
        .iter()
        .enumerate()
        .filter_map(|(i, path)| {
            let (mut score, at) = palette::fuzzy(&q, path)?;
            let name = path.rfind('/').map_or(0, |k| k + 1);
            if palette::fuzzy(&q, &path[name..]).is_some() {
                score += 100;
            }
            Some((score + if changed.contains(path) { 5 } else { 0 }, i, at))
        })
        .collect();
    found.sort_by_key(|(score, ..)| -score);
    found.into_iter().take(SHOWN).map(|(_, i, at)| (i, at)).collect()
}

impl App {
    /// Opens the file finder on the selected repository.
    pub(super) fn open_picker(&mut self) {
        let Some((repo, name, changed)) = self.selected().map(|r| (r.path.clone(), r.name.clone(), r.files.iter().map(|f| f.path.clone()).collect())) else { return };
        self.diff_id += 1;
        worker::spawn_files(repo.clone(), self.diff_id, self.tx.clone());
        self.picker = Some(PickerView {
            repo,
            name,
            files: None,
            error: None,
            query: String::new(),
            shown: Vec::new(),
            state: ListState::default(),
            changed,
            id: self.diff_id,
        });
        self.mode = Mode::Picker;
    }

    pub(super) fn files_loaded(&mut self, id: u64, result: Result<Vec<String>, String>) {
        let Some(v) = self.picker.as_mut().filter(|v| v.id == id) else { return };
        match result {
            Ok(files) => {
                v.files = Some(files);
                v.refilter();
            }
            Err(e) => v.error = Some(e),
        }
    }

    pub(super) fn on_picker_key(&mut self, key: KeyEvent) {
        let Some(v) = self.picker.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let n = v.shown.len() as isize;
        let step = |v: &mut PickerView, by: isize| {
            if n > 0 {
                let at = v.state.selected().unwrap_or(0) as isize;
                v.state.select(Some((at + by).clamp(0, n - 1) as usize));
            }
        };
        match key.code {
            KeyCode::Esc if !v.query.is_empty() => {
                v.query.clear();
                v.refilter();
            }
            KeyCode::Esc => {
                self.picker = None;
                self.mode = Mode::Normal;
            }
            KeyCode::Down => step(v, 1),
            KeyCode::Up => step(v, -1),
            KeyCode::Char('n') if ctrl => step(v, 1),
            KeyCode::Char('p') if ctrl => step(v, -1),
            KeyCode::PageDown => step(v, 10),
            KeyCode::PageUp => step(v, -10),
            KeyCode::Enter => {
                let picked = v.state.selected().and_then(|i| v.shown.get(i)).and_then(|(k, _)| v.files.as_ref()?.get(*k)).cloned();
                let repo = v.repo.clone();
                if let Some(file) = picked {
                    self.open_code(repo, file, None, Mode::Picker);
                }
            }
            KeyCode::Backspace => {
                v.query.pop();
                v.refilter();
            }
            KeyCode::Char('u') if ctrl => {
                v.query.clear();
                v.refilter();
            }
            KeyCode::Char(c) if !ctrl => {
                v.query.push(c);
                v.refilter();
            }
            _ => {}
        }
    }

    /// Opens `file` in the code viewer with the cursor on `line` (1-based); `back` is the mode Esc returns to.
    pub(super) fn open_code(&mut self, repo: PathBuf, file: String, line: Option<usize>, back: Mode) {
        self.diff_id += 1;
        worker::spawn_code(repo.clone(), file.clone(), self.diff_id, self.tx.clone());
        self.code = Some(CodeView {
            repo,
            file,
            lines: None,
            plain: Vec::new(),
            cut: false,
            error: None,
            cursor: 0,
            scroll: 0,
            hscroll: 0,
            height: 0,
            prompt: None,
            query: String::new(),
            hits: Vec::new(),
            goto: line.map(|l| l.saturating_sub(1)),
            back,
            id: self.diff_id,
        });
        self.mode = Mode::Code;
    }

    pub(super) fn code_loaded(&mut self, id: u64, result: Result<Loaded, String>) {
        let Some(v) = self.code.as_mut().filter(|v| v.id == id) else { return };
        match result {
            Ok(loaded) => {
                v.lines = Some(loaded.lines);
                v.plain = loaded.plain;
                v.cut = loaded.cut;
                v.error = None;
                if let Some(line) = v.goto.take() {
                    v.cursor = line.min(v.len().saturating_sub(1));
                    v.center();
                }
                v.cursor = v.cursor.min(v.len().saturating_sub(1));
                if !v.query.is_empty() {
                    v.search(&v.query.clone());
                }
            }
            Err(e) => v.error = Some(e),
        }
    }

    pub(super) fn on_code_key(&mut self, key: KeyEvent) {
        let Some(v) = self.code.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        if let Some((kind, text)) = v.prompt.as_mut() {
            match key.code {
                KeyCode::Esc => v.prompt = None,
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if *kind == Prompt::Find || c.is_ascii_digit() {
                        text.push(c);
                    }
                }
                KeyCode::Enter => {
                    let (kind, text) = (*kind, text.clone());
                    v.prompt = None;
                    match kind {
                        Prompt::Find => {
                            v.search(&text);
                            if !v.next_hit(true) && !text.is_empty() {
                                self.log(Level::Info, format!("\"{text}\" not found"));
                            }
                        }
                        Prompt::Goto => {
                            if let Ok(n) = text.parse::<usize>() {
                                v.cursor = n.saturating_sub(1).min(v.len().saturating_sub(1));
                                v.center();
                            }
                        }
                    }
                }
                _ => {}
            }
            return;
        }
        let page = v.height.saturating_sub(1).max(1);
        let last = v.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                let back = if v.back == Mode::Picker && self.picker.is_none() { Mode::Normal } else { v.back };
                self.mode = back;
                self.code = None;
            }
            KeyCode::Down | KeyCode::Char('j') => v.move_to(v.cursor + 1),
            KeyCode::Up | KeyCode::Char('k') => v.move_to(v.cursor.saturating_sub(1)),
            KeyCode::PageDown | KeyCode::Char(' ') => v.move_to(v.cursor + page),
            KeyCode::PageUp | KeyCode::Char('b') => v.move_to(v.cursor.saturating_sub(page)),
            KeyCode::Home | KeyCode::Char('g') => v.move_to(0),
            KeyCode::End | KeyCode::Char('G') => v.move_to(last),
            KeyCode::Right | KeyCode::Char('l') => v.hscroll += 8,
            KeyCode::Left | KeyCode::Char('h') => v.hscroll = v.hscroll.saturating_sub(8),
            KeyCode::Char('0') => v.hscroll = 0,
            KeyCode::Char('/') => v.prompt = Some((Prompt::Find, v.query.clone())),
            KeyCode::Char(':') => v.prompt = Some((Prompt::Goto, String::new())),
            KeyCode::Char('n') => {
                v.next_hit(true);
            }
            KeyCode::Char('N') => {
                v.next_hit(false);
            }
            _ => {}
        }
    }
}

impl PickerView {
    fn refilter(&mut self) {
        self.shown = self.files.as_ref().map_or_else(Vec::new, |f| rank(f, &self.changed, &self.query));
        self.state.select(if self.shown.is_empty() { None } else { Some(0) });
    }
}

impl CodeView {
    /// Remembers `text` as the search and lists the lines that hold it (case does not matter).
    fn search(&mut self, text: &str) {
        self.query = text.to_string();
        let needle = text.to_lowercase();
        self.hits = if needle.is_empty() { Vec::new() } else { self.plain.iter().enumerate().filter(|(_, l)| l.contains(&needle)).map(|(i, _)| i).collect() };
    }
}
