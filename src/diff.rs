//! Parsing of `git diff` output into rows for the diff viewer, with syntax highlighting.

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
