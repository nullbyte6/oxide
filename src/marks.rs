//! Pinned and ignored repositories.
//!
//! Two sources are combined: `pin` / `ignore` entries of the config file (read-only for Oxide) and
//! a state file, `<state dir>/marks`, that the `*` and `x` keys edit at runtime. Each line of the
//! state file is `pin <path>` or `ignore <path>`.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::config::{self, Config};

#[derive(Clone, Copy, PartialEq)]
pub enum Mark {
    Pin,
    Ignore,
}

pub struct Marks {
    cfg_pins: Vec<String>,
    cfg_ignores: Vec<String>,
    pins: Vec<PathBuf>,
    ignores: Vec<PathBuf>,
}

fn state_file() -> PathBuf {
    config::state_dir().join("marks")
}

/// A config pattern matches a repository by directory name (case-insensitive), or, when it holds a
/// `/` or starts with `~`, by path: the repository must be that directory or live below it.
fn matches(pattern: &str, path: &Path) -> bool {
    if pattern.contains('/') || pattern.starts_with('~') {
        path.starts_with(config::expand(pattern))
    } else {
        path.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(pattern))
    }
}

impl Marks {
    pub fn load(cfg: &Config) -> Marks {
        let (mut pins, mut ignores) = (Vec::new(), Vec::new());
        for line in fs::read_to_string(state_file()).unwrap_or_default().lines() {
            match line.split_once(' ') {
                Some(("pin", p)) => pins.push(PathBuf::from(p)),
                Some(("ignore", p)) => ignores.push(PathBuf::from(p)),
                _ => {}
            }
        }
        Marks { cfg_pins: cfg.pins.clone(), cfg_ignores: cfg.ignores.clone(), pins, ignores }
    }

    /// True when `path` is marked by the config file or by the state file.
    pub fn has(&self, mark: Mark, path: &Path) -> bool {
        let (patterns, saved) = match mark {
            Mark::Pin => (&self.cfg_pins, &self.pins),
            Mark::Ignore => (&self.cfg_ignores, &self.ignores),
        };
        saved.iter().any(|p| p == path) || patterns.iter().any(|p| matches(p, path))
    }

    /// Adds the mark to `path`, or removes it when the state file already holds it. Returns whether
    /// the repository is marked afterwards. A mark that comes from the config file cannot be removed here.
    pub fn toggle(&mut self, mark: Mark, path: &Path) -> Result<bool, String> {
        let from_config = match mark {
            Mark::Pin => &self.cfg_pins,
            Mark::Ignore => &self.cfg_ignores,
        }
        .iter()
        .any(|p| matches(p, path));
        let saved = match mark {
            Mark::Pin => &mut self.pins,
            Mark::Ignore => &mut self.ignores,
        };
        if let Some(i) = saved.iter().position(|p| p == path) {
            saved.remove(i);
        } else if from_config {
            return Err("set by an entry in the config file, remove it there".to_string());
        } else {
            saved.push(path.to_path_buf());
        }
        self.save()?;
        Ok(self.has(mark, path))
    }

    fn save(&self) -> Result<(), String> {
        let mut text = String::new();
        for (tag, list) in [("pin", &self.pins), ("ignore", &self.ignores)] {
            for p in list {
                text.push_str(&format!("{tag} {}\n", p.display()));
            }
        }
        let file = state_file();
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        fs::write(&file, text).map_err(|e| format!("cannot write {}: {e}", file.display()))
    }
}
