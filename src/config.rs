//! Runtime configuration: scan roots, owner names and paths used by Oxide.
//!
//! The optional config file (`$XDG_CONFIG_HOME/oxide/config.lua`, falling back to
//! `~/.config/oxide/config.lua`) is a Lua script. Oxide runs it and reads the table it returns, or, when it
//! returns nothing, the global variables it sets. Keys: `root`, `owner`, `email`, `pin` and `ignore` (a string
//! or a list of strings), `depth` (number), `color`, `hints`, `logs` and `ci` (booleans), `autofetch` (`false`,
//! or an interval such as `"10m"`; a bare number counts minutes) `clone_dir` and `editor` (strings; see `App::editor_line`). `pin` / `ignore` take
//! a repository directory name, or a path when the value contains `/` or starts with `~`. `languages` is a list
//! of tables that add languages to the new-project menu (`K`) or replace a built-in one; see `read_languages`.
//! `tasks` is a list of tables that add commands to the task menu (`M`); see `read_tasks`.

use mlua::{Lua, Table, Value};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

use crate::{
    git,
    scaffold::{Lang, PINS, Package, PackageKind, Step, Variant},
    tasks::Custom,
};

/// How many directory levels below a root are searched for repositories.
const DEFAULT_DEPTH: usize = 6;

pub struct Config {
    /// Why `config.lua` could not be used (a syntax or runtime error); the defaults apply then.
    pub error: Option<String>,
    /// Directories scanned for repositories.
    pub roots: Vec<PathBuf>,
    /// Lowercase account names whose repositories count as "owned".
    pub owners: Vec<String>,
    /// Maximum scan depth below each root.
    pub depth: usize,
    /// Whether accent colors are drawn (`color`; off by default when `NO_COLOR` is set).
    pub color: bool,
    /// Whether the key bar at the bottom starts visible (`hints`); `h` toggles it.
    pub hints: bool,
    /// Whether the log panel starts visible (`logs`); `l` toggles it.
    pub logs: bool,
    /// Lowercase e-mail addresses commits are expected to be made with (`email`); the global `user.email` when empty.
    pub emails: Vec<String>,
    /// Seconds between automatic fetches of every repository (`autofetch = "10m"`); 0 turns it off.
    pub autofetch: u64,
    /// Whether the CI status of GitHub repositories is loaded in the background (`ci`); `w` always refreshes it.
    pub ci: bool,
    /// Folder that GitHub repositories are cloned into by default (`clone_dir`); the home directory otherwise.
    pub clone_dir: PathBuf,
    /// The editor `Ctrl+E` and `e` start, with its arguments (`editor`); `$VISUAL`, `$EDITOR`, or the first of nvim, vim and vi installed when empty.
    pub editor: String,
    /// Repositories always listed first (directory names or paths from `pin` entries).
    pub pins: Vec<String>,
    /// Repositories hidden from the list (directory names or paths from `ignore` entries).
    pub ignores: Vec<String>,
    /// Languages added to the new-project menu (`languages`).
    pub languages: Vec<Lang>,
    /// Commands added to the task menu (`tasks`).
    pub tasks: Vec<Custom>,
    /// Versions that replace the defaults of `scaffold::PINS` (`versions`), as `(name, version)`.
    pub versions: Vec<(String, String)>,
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
    xdg("XDG_CONFIG_HOME", ".config").join("config.lua")
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

/// Runs the config script. Its returned table is the configuration; a script that returns nothing
/// configures through global variables. The `Lua` state is returned too, since the table lives in it.
fn read_table(text: &str) -> mlua::Result<(Lua, Table)> {
    let lua = Lua::new();
    let ret = lua.load(text).set_name("config.lua").call::<Value>(())?;
    let table = match ret {
        Value::Table(t) => t,
        _ => lua.globals(),
    };
    Ok((lua, table))
}

/// A key holding one string or a list of strings, as a list (anything else gives an empty one).
fn strings(conf: &Table, key: &str) -> Vec<String> {
    match conf.get::<Value>(key) {
        Ok(Value::String(s)) => vec![s.to_string_lossy()],
        Ok(Value::Table(list)) => list.sequence_values::<String>().flatten().collect(),
        _ => Vec::new(),
    }
}

/// Text that lives as long as the program, for the language table (a menu entry from `config.lua` is never freed).
fn keep(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// One way to set a language up, from a table with `run` (command lines), `dirs` (directories) and
/// `files` (path -> contents); `note` is the line shown in the menu. They are done in that order. `check` lists
/// commands that prove the finished project works (see `Variant::check`).
fn read_variant(t: &Table, name: String) -> Result<Variant, String> {
    let mut steps: Vec<Step> = strings(t, "run").into_iter().map(|c| Step::Run(keep(c))).collect();
    steps.extend(strings(t, "dirs").into_iter().map(|d| Step::Dir(keep(d))));
    if let Ok(Value::Table(files)) = t.get::<Value>("files") {
        let mut files: Vec<(String, String)> = files.pairs::<String, String>().flatten().collect();
        files.sort(); // Lua tables have no order
        steps.extend(files.into_iter().map(|(path, text)| Step::File(keep(path), keep(text))));
    }
    if steps.is_empty() {
        return Err(format!("variant {name} needs run, dirs or files"));
    }
    let note = t.get::<Option<String>>("note").ok().flatten().unwrap_or_else(|| "your own setup".to_string());
    let check: Vec<&'static str> = strings(t, "check").into_iter().map(keep).collect();
    Ok(Variant { name: keep(name), note: keep(note), steps: steps.leak(), check: check.leak() })
}

/// The `versions` option: a table of version strings keyed by pin name (`java = "17"`, see `scaffold::PINS`).
/// An integer is accepted (`java = 17`); other numbers are not, since `3.10` would become `3.1`. Entries that do
/// not fit are skipped and the first problem is returned so it can be logged.
fn read_versions(conf: &Table) -> (Vec<(String, String)>, Option<String>) {
    let table = match conf.get::<Value>("versions") {
        Ok(Value::Nil) => return (Vec::new(), None),
        Ok(Value::Table(t)) => t,
        _ => return (Vec::new(), Some("versions: expected a table such as { java = \"17\" }".to_string())),
    };
    let (mut out, mut problem) = (Vec::new(), None);
    for pair in table.pairs::<String, Value>().flatten() {
        let (key, value) = pair;
        let version = match value {
            Value::String(s) => s.to_string_lossy(),
            Value::Integer(n) => n.to_string(),
            _ => {
                problem = problem.or(Some(format!("versions.{key}: write the version as a string, e.g. \"3.10\"")));
                continue;
            }
        };
        // A version lands inside templates, some of them in quotes or on a command line.
        if version.is_empty() || version.chars().any(|c| c.is_whitespace() || "'\"\\{}".contains(c)) {
            problem = problem.or(Some(format!("versions.{key}: no spaces, quotes, backslashes or braces, and not empty")));
        } else if PINS.iter().any(|(k, _, _)| *k == key) {
            out.push((key, version));
        } else {
            let known: Vec<&str> = PINS.iter().map(|(k, _, _)| *k).collect();
            problem = problem.or(Some(format!("versions.{key}: unknown; known are {}", known.join(", "))));
        }
    }
    (out, problem)
}

/// The `languages` option: a list of tables, each with `name`, optionally `ignore` (`.gitignore` lines),
/// `key` (a built-in `.gitignore` template such as `"rust"`), `package` (a suggested package name such as
/// `"com.example.{pkg}"`; the menu then asks for packages and steps can use `{package}` and `{package_path}`) and either `variants` (a list of tables with
/// `name`, `note`, `run`, `dirs`, `files`) or those same fields directly for a language with one setup.
/// `run` lines may use `{name}`, `{slug}`, `{pkg}` and `{user}`. Entries that do not fit are skipped; the
/// first problem is returned so it can be logged.
fn read_languages(conf: &Table) -> (Vec<Lang>, Option<String>) {
    let list = match conf.get::<Value>("languages") {
        Ok(Value::Nil) => return (Vec::new(), None),
        Ok(Value::Table(t)) => t,
        _ => return (Vec::new(), Some("languages: expected a list of tables".to_string())),
    };
    let (mut langs, mut problem) = (Vec::new(), None);
    for (i, entry) in list.sequence_values::<Table>().enumerate() {
        let one = entry.map_err(|_| "expected a table".to_string()).and_then(|t| {
            let name = t.get::<String>("name").map_err(|_| "needs a name".to_string())?;
            let variants = match t.get::<Value>("variants") {
                Ok(Value::Table(vs)) => vs
                    .sequence_values::<Table>()
                    .flatten()
                    .map(|v| {
                        let vname = v.get::<Option<String>>("name").ok().flatten().unwrap_or_else(|| "Default".to_string());
                        read_variant(&v, vname)
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("{name}: {e}"))?,
                _ => vec![read_variant(&t, "Default".to_string()).map_err(|e| format!("{name}: {e}"))?],
            };
            if variants.is_empty() {
                return Err(format!("{name}: variants is empty"));
            }
            let key = t.get::<Option<String>>("key").ok().flatten().unwrap_or_default();
            let package = t.get::<Option<String>>("package").ok().flatten().map(|suggest| Package { kind: PackageKind::Packages, suggest: keep(suggest) });
            Ok(Lang { name: keep(name), key: keep(key), ignore: keep(strings(&t, "ignore").join("\n")), package, variants: variants.leak() })
        });
        match one {
            Ok(l) => langs.push(l),
            Err(e) => problem = problem.or(Some(format!("languages[{}]: {e}", i + 1))),
        }
    }
    (langs, problem)
}

/// The `tasks` option: a list of tables with a `name`, the command `run` and, to limit it to one repository, `repo`
/// (a directory name, or a path when it holds a `/` or starts with `~`). A task named like one found in the
/// project files replaces it. A bad entry is reported and left out.
fn read_tasks(conf: &Table) -> (Vec<Custom>, Option<String>) {
    let list = match conf.get::<Value>("tasks") {
        Ok(Value::Nil) => return (Vec::new(), None),
        Ok(Value::Table(t)) => t,
        _ => return (Vec::new(), Some("tasks: expected a list of tables".to_string())),
    };
    let (mut tasks, mut problem) = (Vec::new(), None);
    for (i, entry) in list.sequence_values::<Table>().enumerate() {
        let one = entry.map_err(|_| "expected a table".to_string()).and_then(|t| {
            let name = t.get::<String>("name").map_err(|_| "needs a name".to_string())?;
            let run = t.get::<String>("run").map_err(|_| "needs a run command".to_string())?;
            if name.trim().is_empty() || name.chars().any(char::is_control) || run.trim().is_empty() {
                return Err("name and run must not be empty".to_string());
            }
            let repo = t.get::<Option<String>>("repo").ok().flatten().filter(|r| !r.trim().is_empty());
            Ok(Custom { repo, name: name.trim().to_string(), run })
        });
        match one {
            Ok(c) => tasks.push(c),
            Err(e) => problem = problem.or(Some(format!("tasks[{}]: {e}", i + 1))),
        }
    }
    (tasks, problem)
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
        let mut languages = Vec::new();
        let mut versions = Vec::new();
        let mut tasks = Vec::new();
        let mut hints = true;
        let mut logs = true;
        let mut clone_dir = home();
        let mut editor = String::new();
        let mut emails = Vec::new();
        let mut autofetch = 0;
        let mut ci = true;
        let mut color = env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());

        // A missing file is fine; a file that fails to run is reported (the app logs it) and defaults apply.
        let mut error = None;
        if let Ok(text) = fs::read_to_string(config_file()) {
            match read_table(&text) {
                Ok((lua, conf)) => {
                    roots.extend(strings(&conf, "root").iter().map(|r| expand(r)));
                    owners.extend(strings(&conf, "owner").iter().map(|o| o.to_lowercase()));
                    emails.extend(strings(&conf, "email").iter().map(|e| e.to_lowercase()));
                    pins = strings(&conf, "pin");
                    ignores = strings(&conf, "ignore");
                    let (found, problem) = read_languages(&conf);
                    languages = found;
                    error = error.or(problem);
                    let (found, problem) = read_tasks(&conf);
                    tasks = found;
                    error = error.or(problem);
                    let (found, problem) = read_versions(&conf);
                    versions = found;
                    error = error.or(problem);
                    match conf.get::<Value>("depth") {
                        Ok(Value::Nil) => {}
                        Ok(Value::Integer(n)) if n >= 0 => depth = n as usize,
                        _ => error = Some("depth: expected a whole number".to_string()),
                    }
                    for (key, slot) in [("color", &mut color), ("hints", &mut hints), ("logs", &mut logs), ("ci", &mut ci)] {
                        match conf.get::<Value>(key) {
                            Ok(Value::Nil) => {}
                            Ok(Value::Boolean(b)) => *slot = b,
                            _ => error = Some(format!("{key}: expected true or false")),
                        }
                    }
                    if let Ok(dir) = conf.get::<String>("clone_dir") {
                        clone_dir = expand(&dir);
                    }
                    match conf.get::<Value>("editor") {
                        Ok(Value::Nil) => {}
                        Ok(Value::String(v)) if !v.to_string_lossy().trim().is_empty() => editor = v.to_string_lossy().trim().to_string(),
                        _ => error = Some("editor: expected a command such as \"nvim\" or \"code -g {file}:{line}\"".to_string()),
                    }
                    match conf.get::<Value>("autofetch") {
                        Ok(Value::Boolean(false)) => autofetch = 0,
                        Ok(Value::Integer(n)) => autofetch = u64::try_from(n).unwrap_or(0) * 60,
                        Ok(Value::Number(n)) => autofetch = (n.max(0.0) * 60.0) as u64,
                        Ok(Value::String(v)) => match parse_interval(&v.to_string_lossy()) {
                            Some(secs) => autofetch = secs,
                            None => error = Some("autofetch: expected false or an interval like \"30s\", \"10m\", \"1h\"".to_string()),
                        },
                        _ => {}
                    }
                    drop(lua);
                }
                Err(e) => error = Some(e.to_string()),
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

        Config { error, roots, owners, depth, color, hints, logs, emails, autofetch, ci, clone_dir, editor, pins, ignores, languages, tasks, versions, cross_devices: explicit }
    }
}
