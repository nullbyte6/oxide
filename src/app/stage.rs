//! Staging and discarding single lines in the diff viewer: `v` picks the lines of the hunk at the top of the
//! screen one by one, `s` stages (or unstages) the picked ones and `x` throws them away.

use std::collections::BTreeSet;
use std::ops::Range;

use super::*;
use crate::diff::{Row, Sign};

/// Line mode: the lines of one hunk, some of them picked.
pub struct LinePick {
    /// The hunk (index into `Diff::hunks`).
    pub hunk: usize,
    /// The row under the cursor (an added or removed line of that hunk).
    pub cursor: usize,
    /// The picked rows.
    pub picked: BTreeSet<usize>,
}

impl DiffView {
    /// The rows after the header of hunk `k`.
    fn body(&self, k: usize) -> Option<Range<usize>> {
        let d = self.diff.as_ref()?;
        let start = *d.hunks.get(k)? + 1;
        Some(start..d.hunks.get(k + 1).copied().unwrap_or(d.rows.len()).min(d.rows.len()))
    }

    /// The added and removed rows of hunk `k`.
    pub fn changed(&self, k: usize) -> Vec<usize> {
        let (Some(d), Some(body)) = (self.diff.as_ref(), self.body(k)) else { return Vec::new() };
        body.filter(|&r| matches!(d.rows[r], Row::Line { sign: Sign::Add | Sign::Del, .. })).collect()
    }

    /// The patch for the rows `rows` of hunk `k`, in the direction this diff needs: reversed when it is the
    /// staged diff (unstaging) or when the lines are to be removed from the working tree.
    fn patch_of(&self, k: usize, rows: &BTreeSet<usize>, reverse: bool) -> Option<String> {
        let first = *self.diff.as_ref()?.hunks.get(k)? + 1;
        let offsets: HashSet<usize> = rows.iter().map(|r| r - first).collect();
        diff::lines_patch(&self.raw, k, &offsets, reverse)
    }
}

impl App {
    /// `v`: line mode on the hunk at the top of the screen.
    pub(super) fn start_pick(&mut self) {
        let Some(v) = self.diff.as_mut() else { return };
        if !v.stageable() {
            return self.log(Level::Warn, "only the staged or unstaged diff of a file has lines to pick");
        }
        let Some(k) = v.hunk() else { return self.log(Level::Warn, "no hunk here") };
        match v.changed(k).first() {
            Some(&cursor) => {
                v.pick = Some(LinePick { hunk: k, cursor, picked: BTreeSet::new() });
                v.reveal(cursor);
            }
            None => self.log(Level::Warn, "this hunk has no changed lines"),
        }
    }

    /// Keys while picking lines.
    pub(super) fn on_pick_key(&mut self, key: KeyEvent) {
        let Some(v) = self.diff.as_mut() else { return };
        let Some(p) = v.pick.as_ref() else { return };
        let changed = v.changed(p.hunk);
        let at = changed.iter().position(|&r| r == p.cursor).unwrap_or(0);
        let Some(p) = v.pick.as_mut() else { return };
        match key.code {
            KeyCode::Esc | KeyCode::Char('v' | 'q') => v.pick = None,
            KeyCode::Down | KeyCode::Char('j') => p.cursor = changed[(at + 1).min(changed.len() - 1)],
            KeyCode::Up | KeyCode::Char('k') => p.cursor = changed[at.saturating_sub(1)],
            KeyCode::Char(' ') => {
                if !p.picked.remove(&p.cursor) {
                    p.picked.insert(p.cursor);
                }
                p.cursor = changed[(at + 1).min(changed.len() - 1)];
            }
            KeyCode::Char('a') => {
                if p.picked.len() == changed.len() {
                    p.picked.clear();
                } else {
                    p.picked.extend(changed.iter().copied());
                }
            }
            KeyCode::Enter | KeyCode::Char('s') => return self.send_lines(false),
            KeyCode::Char('x') => return self.send_lines(true),
            _ => {}
        }
        if let Some(cursor) = v.pick.as_ref().map(|p| p.cursor) {
            v.reveal(cursor);
        }
    }

    /// Stages (unstages, in the staged diff) or, with `discard`, throws away the picked lines; the line under the
    /// cursor when none is picked.
    fn send_lines(&mut self, discard: bool) {
        let Some(v) = self.diff.as_mut() else { return };
        let Some(p) = v.pick.as_ref() else { return };
        let rows = if p.picked.is_empty() { BTreeSet::from([p.cursor]) } else { p.picked.clone() };
        let k = p.hunk;
        self.send_rows(k, rows, discard);
    }

    /// `x` in the diff viewer outside line mode: throws away the whole hunk at the top of the screen.
    pub(super) fn discard_hunk(&mut self) {
        let Some(v) = self.diff.as_ref() else { return };
        if v.kind != DiffKind::Unstaged {
            return self.log(Level::Warn, "only changes that are not staged can be discarded here");
        }
        let Some(k) = v.hunk() else { return self.log(Level::Warn, "no hunk here") };
        let rows: BTreeSet<usize> = v.changed(k).into_iter().collect();
        self.send_rows(k, rows, true);
    }

    fn send_rows(&mut self, k: usize, rows: BTreeSet<usize>, discard: bool) {
        let Some(v) = self.diff.as_mut() else { return };
        let (repo, file, n) = (v.repo.clone(), v.file.clone(), rows.len());
        if discard {
            if v.kind != DiffKind::Unstaged {
                return self.log(Level::Warn, "only changes that are not staged can be discarded here");
            }
            let Some(patch) = v.patch_of(k, &rows, true) else { return self.log(Level::Warn, "nothing to discard") };
            v.pick = None;
            let prompt = format!("discard {n} changed line{} in {file}? it cannot be undone", if n == 1 { "" } else { "s" });
            return self.ask(prompt, vec![(repo, Op::Discard { patch })]);
        }
        let reverse = v.kind == DiffKind::Staged;
        let Some(patch) = v.patch_of(k, &rows, reverse) else { return self.log(Level::Warn, "no changed line picked") };
        v.pick = None;
        self.run_ops(vec![(repo, Op::Hunk { patch, reverse })]);
    }
}

impl DiffView {
    /// Scrolls just enough to see `row`.
    fn reveal(&mut self, row: usize) {
        let h = self.height.max(1);
        if row < self.scroll {
            self.scroll = row;
        } else if row >= self.scroll + h {
            self.scroll = row + 1 - h;
        }
    }
}
