//! Runtime configuration: scan roots, owner names and paths used by Oxide.
//!
//! The optional config file (`$XDG_CONFIG_HOME/oxide/config`, falling back to
//! `~/.config/oxide/config`) is a list of `key = value` lines; `#` starts a comment.
//! Supported keys: `root` (repeatable), `owner` (repeatable), `depth`, `color`, `hints`, `logs` and `ci` (`on` / `off`), `email` (repeatable), `autofetch` (`off`, or an
//! interval such as `10m`), `clone_dir`, and the repeatable `pin` and
//! `ignore` (a repository directory name, or a path when the value contains `/` or starts with `~`).

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use crate::git;

/// How many directory levels below a root are searched for repositories.
const DEFAULT_DEPTH: usize = 6;

pub struct Config {
    /// Directories scanned for repositories.
    pub roots: Vec<PathBuf>,
    /// Lowercase account names whose repositories count as "owned".
    pub owners: Vec<String>,
    /// Maximum scan depth below each root.
    pub depth: usize,
    /// Whether accent colors are drawn (`color = on|off`; off by default when `NO_COLOR` is set).
    pub color: bool,
    /// Whether the key bar at the bottom starts visible (`hints = on|off`); `h` toggles it.
    pub hints: bool,
    /// Whether the log panel starts visible (`logs = on|off`); `l` toggles it.
    pub logs: bool,
    /// Lowercase e-mail addresses commits are expected to be made with (`email`); the global `user.email` when empty.
    pub emails: Vec<String>,
    /// Seconds between automatic fetches of every repository (`autofetch = 10m`); 0 turns it off.
    pub autofetch: u64,
    /// Whether the CI status of GitHub repositories is loaded in the background (`ci = on|off`); `w` always refreshes it.
    pub ci: bool,
    /// Folder that GitHub repositories are cloned into by default (`clone_dir`); the home directory otherwise.
    pub clone_dir: PathBuf,
    /// Repositories always listed first (directory names or paths from `pin` entries).
    pub pins: Vec<String>,
    /// Repositories hidden from the list (directory names or paths from `ignore` entries).
    pub ignores: Vec<String>,
    /// False for the automatic whole-machine roots: each mounted drive is its own root, so a scan
    /// never steps onto another filesystem. Explicit roots (CLI or config) may cross mount points.
    pub cross_devices: bool,
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

/// Mount points of every real block-device filesystem (internal and external drives), from `/proc/mounts`.
/// Falls back to the home directory where that file does not exist.
fn machine_roots() -> Vec<PathBuf> {
    let Ok(mounts) = fs::read_to_string("/proc/mounts") else { return vec![home()] };
    let mut roots: Vec<PathBuf> = mounts
        .lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let (dev, mount, fstype) = (f.next()?, f.next()?, f.next()?);
            let mount = unescape_mount(mount);
            // Only block devices; compressed read-only images (snap, appimage) and system mounts are noise.
            let skip = !dev.starts_with("/dev/")
                || matches!(fstype, "squashfs" | "erofs")
                || ["/boot", "/snap", "/var/lib"].iter().any(|p| mount.starts_with(p));
            (!skip).then(|| PathBuf::from(mount))
        })
        .collect();
    roots.sort();
    roots.dedup();
    if roots.is_empty() {
        roots.push(home());
    }
    roots
}

/// Decodes the `\040`-style octal escapes `/proc/mounts` uses for spaces and other characters.
fn unescape_mount(s: &str) -> String {
    let (b, mut out, mut i) = (s.as_bytes(), Vec::new(), 0);
    while i < b.len() {
        let octal = (b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)))
            .then(|| b[i + 1..i + 4].iter().fold(0u32, |n, c| n * 8 + u32::from(c - b'0')) as u8);
        match octal {
            Some(byte) => {
                out.push(byte);
                i += 4;
            }
            None => {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Seconds in an interval such as `30s`, `10m` or `1h`; a bare number counts minutes and `off` means 0.
fn parse_interval(val: &str) -> Option<u64> {
    let val = val.trim().to_lowercase();
    if matches!(val.as_str(), "off" | "false" | "no") {
        return Some(0);
    }
    let digits = val.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let n: u64 = digits.parse().ok()?;
    match &val[digits.len()..] {
        "s" => Some(n),
        "" | "m" => Some(n * 60),
        "h" => Some(n * 3600),
        _ => None,
    }
}

impl Config {
    /// Builds the configuration; `cli_roots` (from the command line) replace any `root` entries.
    pub fn load(cli_roots: Vec<PathBuf>) -> Config {
        let mut roots = Vec::new();
        let mut owners = Vec::new();
        let mut depth = DEFAULT_DEPTH;
        let (mut pins, mut ignores) = (Vec::new(), Vec::new());
        let mut hints = true;
        let mut logs = true;
        let mut clone_dir = home();
        let mut emails = Vec::new();
        let mut autofetch = 0;
        let mut ci = true;
        let mut color = env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());

        if let Ok(text) = fs::read_to_string(config_file()) {
            for line in text.lines() {
                let line = line.trim();
                let Some((key, val)) = line.split_once('=') else { continue };
                let (key, val) = (key.trim(), val.trim());
                match key {
                    "root" => roots.push(expand(val)),
                    "owner" => owners.push(val.to_lowercase()),
                    "depth" => depth = val.parse().unwrap_or(depth),
                    "color" => match val.to_lowercase().as_str() {
                        "on" | "true" | "yes" | "1" => color = true,
                        "off" | "false" | "no" | "0" => color = false,
                        _ => {}
                    },
                    "hints" => match val.to_lowercase().as_str() {
                        "on" | "true" | "yes" | "1" => hints = true,
                        "off" | "false" | "no" | "0" => hints = false,
                        _ => {}
                    },
                    "clone_dir" => clone_dir = expand(val),
                    "email" => emails.push(val.to_lowercase()),
                    "autofetch" => autofetch = parse_interval(val).unwrap_or(autofetch),
                    "ci" => match val.to_lowercase().as_str() {
                        "on" | "true" | "yes" | "1" => ci = true,
                        "off" | "false" | "no" | "0" => ci = false,
                        _ => {}
                    },
                    "logs" => match val.to_lowercase().as_str() {
                        "on" | "true" | "yes" | "1" => logs = true,
                        "off" | "false" | "no" | "0" => logs = false,
                        _ => {}
                    },
                    "pin" => pins.push(val.to_string()),
                    "ignore" => ignores.push(val.to_string()),
                    _ => {} // comments, blank lines and unknown keys
                }
            }
        }

        if !cli_roots.is_empty() {
            roots = cli_roots;
        }
        let explicit = !roots.is_empty();
        if !explicit {
            roots = machine_roots();
        }

        // The user's own git identity always counts as an owner name.
        for key in ["user.name", "github.user"] {
            if let Some(v) = git::global_config(key) {
                owners.push(v.to_lowercase());
            }
        }
        owners.sort();
        owners.dedup();

        Config { roots, owners, depth, color, hints, logs, emails, autofetch, ci, clone_dir, pins, ignores, cross_devices: explicit }
    }
}
