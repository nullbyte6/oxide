//! Parsing of `git diff` output into rows for the diff viewer, with syntax highlighting.

use std::collections::HashSet;

use crate::syntax::{self, State, Tok};

/// A diff longer than this many rows is cut off (the viewer says so).
const MAX_ROWS: usize = 20_000;

#[derive(Clone, Copy, PartialEq)]
pub enum Sign {
    Add,
    Del,
    Ctx,
}

pub enum Row {
    /// Header information worth keeping: new/deleted file, renames, binary files, modes.
    Meta(String),
    /// The file a diff section belongs to (a commit can touch many files).
    File(String),
    /// `@@ -a,b +c,d @@ section`.
    Hunk(String),
    /// `\ No newline at end of file` and similar notes.
    Note(String),
    Line { sign: Sign, old: Option<u32>, new: Option<u32>, toks: Vec<(Tok, String)> },
}

pub struct Diff {
    pub rows: Vec<Row>,
    /// Indices into `rows` of every hunk header, for hunk-to-hunk jumps.
    pub hunks: Vec<usize>,
    pub added: usize,
    pub removed: usize,
    /// Width of the widest line number, for the gutter.
    pub num_w: usize,
    pub truncated: bool,
}

/// Tabs become spaces and control characters are dropped, so every char is one terminal cell.
pub(crate) fn clean(s: &str) -> String {
    s.trim_end_matches('\r').chars().flat_map(|c| if c == '\t' { vec![' '; 4] } else if c.is_control() { vec![] } else { vec![c] }).collect()
}

/// First line number of the `-` (old) and `+` (new) side of a hunk header.
fn hunk_start(header: &str) -> (u32, u32) {
    let side = |sign: char, last: bool| {
        let mut nums = header
            .split_whitespace()
            .filter_map(|t| t.strip_prefix(sign))
            .filter_map(|t| t.split(',').next()?.parse::<u32>().ok());
        (if last { nums.last() } else { nums.next() }).unwrap_or(1)
    };
    (side('-', false), side('+', true))
}

/// The patch that applies only hunk number `index` (counting from 0) of a single-file diff: the file
/// header lines, then that hunk's lines, ready for `git apply`.
pub fn hunk_patch(raw: &str, index: usize) -> Option<String> {
    let (mut header, mut body) = (String::new(), String::new());
    let mut seen = 0usize;
    for line in raw.split_inclusive('\n') {
        if line.starts_with("@@") {
            seen += 1;
        }
        match seen {
            0 => header.push_str(line),
            n if n == index + 1 => body.push_str(line),
            _ => {}
        }
    }
    (!body.is_empty()).then(|| header + &body)
}

/// One line of the side-by-side layout, as indices into `Diff::rows`.
pub enum SRow {
    /// A row across both sides: file name, hunk header, notes.
    Full(usize),
    /// An old line on the left and the new line it became on the right; either may be missing.
    Pair(Option<usize>, Option<usize>),
}

/// The side-by-side layout of a diff.
pub struct Split {
    pub rows: Vec<SRow>,
    /// Indices into `rows` of every hunk header.
    pub hunks: Vec<usize>,
    /// For each row, the index of the first of `Diff::rows` it shows, so a position can move between layouts.
    pub origin: Vec<usize>,
}

/// Lays `d` out side by side: context on both sides, and each run of removed lines next to the run of added
/// lines that follows it.
pub fn split(d: &Diff) -> Split {
    let mut s = Split { rows: Vec::new(), hunks: Vec::new(), origin: Vec::new() };
    let sign_at = |i: usize| match d.rows.get(i) {
        Some(Row::Line { sign, .. }) => Some(*sign),
        _ => None,
    };
    let mut i = 0;
    while i < d.rows.len() {
        match sign_at(i) {
            Some(Sign::Ctx) => {
                s.origin.push(i);
                s.rows.push(SRow::Pair(Some(i), Some(i)));
                i += 1;
            }
            Some(_) => {
                let (mut dels, mut adds) = (Vec::new(), Vec::new());
                while sign_at(i) == Some(Sign::Del) {
                    dels.push(i);
                    i += 1;
                }
                while sign_at(i) == Some(Sign::Add) {
                    adds.push(i);
                    i += 1;
                }
                for k in 0..dels.len().max(adds.len()) {
                    let (l, r) = (dels.get(k).copied(), adds.get(k).copied());
                    s.origin.push(l.or(r).unwrap_or(0));
                    s.rows.push(SRow::Pair(l, r));
                }
            }
            None => {
                if matches!(d.rows[i], Row::Hunk(_)) {
                    s.hunks.push(s.rows.len());
                }
                s.origin.push(i);
                s.rows.push(SRow::Full(i));
                i += 1;
            }
        }
    }
    s
}

/// The patch for only some lines of hunk `index` of a single-file diff. `picked` holds the offsets of the lines
/// to act on, counted from the first line after the hunk header. The other changed lines are left alone: for
/// a forward patch (staging) an unpicked `-` becomes context and an unpicked `+` is dropped; for a reverse patch
/// (unstaging, or discarding from the working tree) an unpicked `+` becomes context and an unpicked `-` is
/// dropped. `None` when no changed line is picked.
pub fn lines_patch(raw: &str, index: usize, picked: &HashSet<usize>, reverse: bool) -> Option<String> {
    let (mut header, mut head, mut body) = (String::new(), "", Vec::new());
    let mut seen = 0usize;
    for line in raw.split_inclusive('\n') {
        if line.starts_with("@@") {
            seen += 1;
            if seen == index + 1 {
                head = line;
            }
            continue;
        }
        match seen {
            0 => header.push_str(line),
            n if n == index + 1 => body.push(line),
            _ => {}
        }
    }
    if head.is_empty() {
        return None;
    }
    let (old_start, new_start) = hunk_start(head);
    let section = head.splitn(3, "@@").nth(2).unwrap_or("\n");
    let (mut out, mut old_n, mut new_n, mut changes) = (String::new(), 0, 0, 0);
    // Whether the previous line is in the patch: a "\ No newline" note follows its line or goes away with it.
    let mut kept = true;
    for (i, line) in body.iter().enumerate() {
        let rest = line.get(1..).unwrap_or("");
        let take = picked.contains(&i);
        match line.chars().next() {
            Some('+') if take => {
                out.push_str(line);
                new_n += 1;
                changes += 1;
                kept = true;
            }
            Some('+') if reverse => {
                out.push(' ');
                out.push_str(rest);
                old_n += 1;
                new_n += 1;
                kept = true;
            }
            Some('-') if take => {
                out.push_str(line);
                old_n += 1;
                changes += 1;
                kept = true;
            }
            Some('-') if !reverse => {
                out.push(' ');
                out.push_str(rest);
                old_n += 1;
                new_n += 1;
                kept = true;
            }
            Some('+' | '-') => kept = false,
            Some('\\') => {
                if kept {
                    out.push_str(line);
                }
            }
            _ => {
                out.push_str(line);
                old_n += 1;
                new_n += 1;
                kept = true;
            }
        }
    }
    let end = if section.ends_with('\n') { "" } else { "\n" };
    (changes > 0).then(|| format!("{header}@@ -{old_start},{old_n} +{new_start},{new_n} @@{section}{end}{out}"))
}

/// Parses unified diff text of one file; `file` picks the language used for highlighting.
pub fn parse(text: &str, file: &str) -> Diff {
    let mut lang = syntax::lang_for(file);
    let mut d = Diff { rows: Vec::new(), hunks: Vec::new(), added: 0, removed: 0, num_w: 1, truncated: false };
    let (mut in_hunk, mut cols) = (false, 1);
    let (mut old_n, mut new_n) = (0u32, 0u32);
    // Open comments and strings are tracked separately for the old and the new side of the file.
    let (mut old_state, mut new_state) = (State::Normal, State::Normal);

    for raw in text.lines() {
        if d.rows.len() >= MAX_ROWS {
            d.truncated = true;
            break;
        }
        // A new file section. Content lines always start with a sign or a space, so this is unambiguous.
        if let Some(head) = raw.strip_prefix("diff --git ").or_else(|| raw.strip_prefix("diff --cc ")) {
            let name = head.rsplit_once(" b/").map_or(head, |(_, b)| b).trim_start_matches("a/");
            in_hunk = false;
            lang = syntax::lang_for(name);
            (old_state, new_state) = (State::Normal, State::Normal);
            d.rows.push(Row::File(clean(name)));
            continue;
        }
        if raw.starts_with("@@") {
            in_hunk = true;
            cols = raw.chars().take_while(|&c| c == '@').count() - 1; // 1 for a normal diff, 2 for a merge
            (old_n, new_n) = hunk_start(raw);
            (old_state, new_state) = (State::Normal, State::Normal);
            d.hunks.push(d.rows.len());
            d.rows.push(Row::Hunk(clean(raw)));
            continue;
        }
        if !in_hunk {
            // The first lines repeat the file name and hashes; the header bar already shows them.
            let noise = ["diff --git", "diff --cc", "diff --combined", "index ", "--- ", "+++ "];
            if !noise.iter().any(|p| raw.starts_with(p)) && !raw.trim().is_empty() {
                d.rows.push(Row::Meta(clean(raw)));
            }
            continue;
        }
        if raw.starts_with('\\') {
            d.rows.push(Row::Note(clean(raw)));
            continue;
        }
        let split = raw.char_indices().nth(cols).map_or(raw.len(), |(i, _)| i);
        let (marks, code) = raw.split_at(split);
        let sign = if marks.contains('+') {
            Sign::Add
        } else if marks.contains('-') {
            Sign::Del
        } else {
            Sign::Ctx
        };
        let code = clean(code);
        let (old, new, toks) = match sign {
            Sign::Add => {
                d.added += 1;
                new_n += 1;
                (None, Some(new_n - 1), syntax::highlight(lang, &code, &mut new_state))
            }
            Sign::Del => {
                d.removed += 1;
                old_n += 1;
                (Some(old_n - 1), None, syntax::highlight(lang, &code, &mut old_state))
            }
            Sign::Ctx => {
                old_n += 1;
                new_n += 1;
                let toks = syntax::highlight(lang, &code, &mut new_state);
                old_state = new_state;
                (Some(old_n - 1), Some(new_n - 1), toks)
            }
        };
        // A merge diff has one old side per parent, so its old line numbers mean nothing.
        let old = if cols > 1 { None } else { old };
        d.num_w = d.num_w.max(old.unwrap_or(0).max(new.unwrap_or(0)).to_string().len());
        d.rows.push(Row::Line { sign, old, new, toks });
    }
    d
}
