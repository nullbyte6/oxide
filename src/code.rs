//! Reading a source file for the code viewer: the text cut to a sane size, cleaned so every character is
//! one terminal cell, and highlighted line by line.

use std::{fs, io::Read, path::Path};

use crate::{
    diff::clean,
    syntax::{self, State, Tok},
};

/// A file larger than this many bytes is read only up to this point (the viewer says so).
const MAX_BYTES: u64 = 2 * 1024 * 1024;
/// At most this many lines are kept.
const MAX_LINES: usize = 50_000;
/// A line longer than this many characters is cut (minified files would otherwise be one huge line).
const MAX_WIDTH: usize = 4_000;

/// A file ready to be shown.
pub struct Loaded {
    /// The highlighted lines.
    pub lines: Vec<Vec<(Tok, String)>>,
    /// The same lines as plain lowercase text, for searching.
    pub plain: Vec<String>,
    /// The file was longer than what is shown.
    pub cut: bool,
}

/// Reads `file` (relative to `repo`); text files only.
pub fn load(repo: &Path, file: &str) -> Result<Loaded, String> {
    let path = repo.join(file);
    let meta = fs::metadata(&path).map_err(|e| format!("cannot read {file}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{file} is not a file"));
    }
    let mut bytes = Vec::new();
    fs::File::open(&path).and_then(|f| f.take(MAX_BYTES).read_to_end(&mut bytes)).map_err(|e| format!("cannot read {file}: {e}"))?;
    if bytes.iter().take(8192).any(|&b| b == 0) {
        return Err(format!("{file} is a binary file"));
    }
    let text = String::from_utf8_lossy(&bytes);
    let lang = syntax::lang_for(file);
    let mut state = State::Normal;
    let (mut lines, mut plain) = (Vec::new(), Vec::new());
    for raw in text.lines().take(MAX_LINES) {
        let line: String = clean(raw).chars().take(MAX_WIDTH).collect();
        lines.push(syntax::highlight(lang, &line, &mut state));
        plain.push(line.to_lowercase());
    }
    let cut = meta.len() > MAX_BYTES || text.lines().nth(MAX_LINES).is_some();
    Ok(Loaded { lines, plain, cut })
}
