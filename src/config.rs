//! Runtime configuration: scan roots, owner names and paths used by Oxide.
//!
//! The optional config file (`$XDG_CONFIG_HOME/oxide/config`, falling back to
//! `~/.config/oxide/config`) is a list of `key = value` lines; `#` starts a comment.
//! Supported keys: `root` (repeatable), `owner` (repeatable) and `depth`.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use crate::git;

/// How many directory levels below a root are searched for repositories.
const DEFAULT_DEPTH: usize = 5;

pub struct Config {
    /// Directories scanned for repositories.
    pub roots: Vec<PathBuf>,
    /// Lowercase account names whose repositories count as "owned".
    pub owners: Vec<String>,
    /// Maximum scan depth below each root.
    pub depth: usize,
}

pub fn home() -> PathBuf {
    env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// Returns `$VAR/oxide` when the variable is set, else `~/<fallback>/oxide`.
fn xdg(var: &str, fallback: &str) -> PathBuf {
    match env::var_os(var) {
        Some(v) if !v.is_empty() => PathBuf::from(v).join("oxide"),
        _ => home().join(fallback).join("oxide"),
    }
}

pub fn config_file() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("config")
}

/// Directory that holds the persistent log file.
pub fn state_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state")
}

/// Expands a leading `~` to the home directory.
pub fn expand(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None if p == "~" => home(),
        None => Path::new(p).to_path_buf(),
    }
}

impl Config {
    /// Builds the configuration; `cli_roots` (from the command line) replace any `root` entries.
    pub fn load(cli_roots: Vec<PathBuf>) -> Config {
        let mut roots = Vec::new();
        let mut owners = Vec::new();
        let mut depth = DEFAULT_DEPTH;

        if let Ok(text) = fs::read_to_string(config_file()) {
            for line in text.lines() {
                let line = line.trim();
                let Some((key, val)) = line.split_once('=') else { continue };
                let (key, val) = (key.trim(), val.trim());
                match key {
                    "root" => roots.push(expand(val)),
                    "owner" => owners.push(val.to_lowercase()),
                    "depth" => depth = val.parse().unwrap_or(depth),
                    _ => {} // comments, blank lines and unknown keys
                }
            }
        }

        if !cli_roots.is_empty() {
            roots = cli_roots;
        }
        if roots.is_empty() {
            roots.push(home());
        }

        // The user's own git identity always counts as an owner name.
        for key in ["user.name", "github.user"] {
            if let Some(v) = git::global_config(key) {
                owners.push(v.to_lowercase());
            }
        }
        owners.sort();
        owners.dedup();

        Config { roots, owners, depth }
    }
}
