//! Command palette: the list of actions and the fuzzy matcher behind it.
//!
//! Every entry names the key of the dashboard action it runs; choosing it feeds that key to the
//! dashboard, so the palette and the keyboard can never disagree.

use crossterm::event::KeyCode;

pub struct Entry {
    pub name: &'static str,
    /// The key shown next to the entry.
    pub key: &'static str,
    /// The key the dashboard receives when the entry runs.
    pub code: KeyCode,
}

const fn e(name: &'static str, key: &'static str, code: char) -> Entry {
    Entry { name, key, code: KeyCode::Char(code) }
}

pub const ENTRIES: &[Entry] = &[
    e("Stage or unstage the highlighted file", "Space", ' '),
    e("Stage all changes", "a", 'a'),
    e("Unstage all changes", "u", 'u'),
    e("Commit staged changes", "c", 'c'),
    e("Show the diff of the highlighted file", "d", 'd'),
    e("Show the commit log", "v", 'v'),
    e("Amend the last commit", "E", 'E'),
    e("Reflog: see and undo what happened to HEAD", "O", 'O'),
    e("Manage branches", "b", 'b'),
    e("Worktrees and submodules", "W", 'W'),
    e("Merge a branch into the current one", "m", 'm'),
    e("GitHub issues of this repository", "i", 'i'),
    e("GitHub pull requests of this repository", "y", 'y'),
    e("Open this repository on GitHub in the browser", "B", 'B'),
    e("Find and clone GitHub repositories", "S", 'S'),
    e("Pull (fast-forward only)", "p", 'p'),
    e("Push", "P", 'P'),
    e("Fetch this repository", "F", 'F'),
    e("Fetch every repository in view", "A", 'A'),
    e("Pull every repository in view that is behind", "L", 'L'),
    e("Push every repository in view that is ahead", "U", 'U'),
    e("Stash all changes", "z", 'z'),
    e("Pop the latest stash", "Z", 'Z'),
    e("git init in a folder", "I", 'I'),
    e("Open in yazi", "e", 'e'),
    e("Open the embedded terminal here", "t", 't'),
    e("Set up a new project in an empty repository", "K", 'K'),
    e("Open in the file manager", "o", 'o'),
    e("Delete the repository from disk", "D", 'D'),
    e("Pin or unpin the repository", "*", '*'),
    e("Ignore or unignore the repository", "x", 'x'),
    e("Show or hide ignored repositories", "H", 'H'),
    e("Change the sort order", "s", 's'),
    e("Filter: all repositories", "1", '1'),
    e("Filter: owned repositories", "2", '2'),
    e("Filter: cloned repositories", "3", '3'),
    e("Filter: repositories needing attention", "4", '4'),
    e("Search repositories", "/", '/'),
    Entry { name: "Reload this repository", key: "F5", code: KeyCode::F(5) },
    e("Manage remotes: add, change URL, rename, remove", "r", 'r'),
    e("Rescan all roots", "R", 'R'),
    e("Refresh the CI status of repositories in view", "w", 'w'),
    e("Toggle colors", "C", 'C'),
    e("Show or hide the key bar", "h", 'h'),
    e("Show or hide the log panel", "l", 'l'),
    e("Show the key help", "?", '?'),
    e("Quit", "q", 'q'),
];

/// Matches `query` against every entry. Returns `(entry index, matched character positions)`, best
/// match first. An empty query matches nothing, so the palette stays empty until something is typed.
pub fn matches(query: &str) -> Vec<(usize, Vec<usize>)> {
    let q: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Vec::new();
    }
    let mut found: Vec<(i32, usize, Vec<usize>)> = ENTRIES
        .iter()
        .enumerate()
        .filter_map(|(i, entry)| fuzzy(&q, entry.name).map(|(score, at)| (score, i, at)))
        .collect();
    // Stable sort: equal scores keep the listed order.
    found.sort_by_key(|(score, ..)| -score);
    found.into_iter().map(|(_, i, at)| (i, at)).collect()
}

/// Subsequence match of `q` (lowercase) in `text`: every query character must appear in order.
/// Consecutive matches and matches at the start of a word score higher.
fn fuzzy(q: &[char], text: &str) -> Option<(i32, Vec<usize>)> {
    let chars: Vec<char> = text.chars().collect();
    let (mut at, mut score, mut from) = (Vec::new(), 0, 0);
    for &want in q {
        let pos = (from..chars.len()).find(|&k| chars[k].to_lowercase().next() == Some(want))?;
        let word_start = pos == 0 || !chars[pos - 1].is_alphanumeric();
        score += 1 + if at.last() == Some(&(pos.wrapping_sub(1))) { 8 } else { 0 } + if word_start { 6 } else { 0 };
        at.push(pos);
        from = pos + 1;
    }
    // A query that appears as a whole word ("clone" in "clone GitHub", not in "cloned") ranks first.
    let lower: Vec<char> = chars.iter().flat_map(|c| c.to_lowercase()).collect();
    let whole_word = lower.windows(q.len()).enumerate().any(|(i, w)| {
        w == q && (i == 0 || !lower[i - 1].is_alphanumeric()) && lower.get(i + q.len()).is_none_or(|c| !c.is_alphanumeric())
    });
    // Prefer matches that start early and entries with shorter names.
    Some((score * 4 + if whole_word { 60 } else { 0 } - at.first().map_or(0, |&p| p as i32) - chars.len() as i32 / 8, at))
}
