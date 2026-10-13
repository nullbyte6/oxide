//! Tasks: the build, test, lint and other commands a repository offers, found from its project files and from
//! `config.lua`, and the background process that runs one and streams its output back.

use std::{
    fs,
    io::Read,
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    thread,
    time::{Duration, Instant},
};

use crate::{diff::clean, worker::Msg};

/// A command of one repository.
pub struct Task {
    /// What it is called in the menu: `build`, `test`, a script or a make target.
    pub name: String,
    /// The shell command line, run in the repository.
    pub cmd: String,
    /// Where it comes from: a project file, or `config.lua`.
    pub source: String,
}

/// A task from `config.lua` (`tasks`): for every repository, or only for the one `repo` names (a directory name,
/// or a path when it holds a `/` or starts with `~`).
pub struct Custom {
    pub repo: Option<String>,
    pub name: String,
    pub run: String,
}

/// Commands that every repository with the file has, as `(file, [(name, command)])`.
const KNOWN: &[(&str, &[(&str, &str)])] = &[
    ("Cargo.toml", &[("build", "cargo build"), ("test", "cargo test"), ("check", "cargo check"), ("lint", "cargo clippy --all-targets"), ("fmt", "cargo fmt --check"), ("run", "cargo run")]),
    ("go.mod", &[("build", "go build ./..."), ("test", "go test ./..."), ("vet", "go vet ./..."), ("run", "go run .")]),
    ("pom.xml", &[("build", "mvn -q compile"), ("test", "mvn test"), ("package", "mvn package")]),
    ("CMakeLists.txt", &[("build", "cmake -S . -B build && cmake --build build"), ("test", "ctest --test-dir build --output-on-failure")]),
    ("Package.swift", &[("build", "swift build"), ("test", "swift test"), ("run", "swift run")]),
    ("build.zig", &[("build", "zig build"), ("test", "zig build test"), ("run", "zig build run")]),
    ("mix.exs", &[("build", "mix compile"), ("test", "mix test"), ("run", "mix run")]),
    ("pubspec.yaml", &[("analyze", "dart analyze"), ("test", "dart test"), ("run", "dart run")]),
    ("build.sbt", &[("build", "sbt compile"), ("test", "sbt test"), ("run", "sbt run")]),
    ("project.clj", &[("test", "lein test"), ("run", "lein run")]),
    ("stack.yaml", &[("build", "stack build"), ("test", "stack test")]),
    ("Rakefile", &[("rake", "rake"), ("test", "rake test")]),
];

/// Names that may become part of a command line (a script, a make target ...); anything else is left out, because
/// the name comes from a file of the repository and would be run by the shell.
fn safe(name: &str) -> bool {
    !name.is_empty() && name.len() <= 60 && !name.starts_with('-') && name.chars().all(|c| c.is_ascii_alphanumeric() || "_.:/@-".contains(c))
}

/// The `"name": "value"` pairs of the object under `key` in a JSON file (`scripts` of package.json and
/// composer.json). A tiny scanner, not a JSON parser: values that are not strings are skipped.
fn json_strings(text: &str, key: &str) -> Vec<(String, String)> {
    let Some(at) = text.find(&format!("\"{key}\"")) else { return Vec::new() };
    let rest = &text[at + key.len() + 2..];
    let Some(open) = rest.find('{') else { return Vec::new() };
    let (mut out, mut depth) = (Vec::new(), 0usize);
    // At depth 1 a string is a name when one is expected (after `{` or `,`) and a value otherwise.
    let (mut name, mut expect_name) = (None::<String>, true);
    let mut chars = rest[open..].chars();
    while let Some(c) = chars.next() {
        match c {
            '{' | '[' => depth += 1,
            '}' | ']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            ',' if depth == 1 => (name, expect_name) = (None, true),
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(e) => s.push(e),
                            None => break,
                        },
                        '"' => break,
                        c => s.push(c),
                    }
                }
                if depth == 1 {
                    if expect_name {
                        (name, expect_name) = (Some(s), false);
                    } else if let Some(n) = name.take() {
                        out.push((n, s));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Targets of a Makefile, in order: `name:` at the start of a line, not an assignment, a pattern or a special target.
fn make_targets(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in text.lines() {
        if line.starts_with(|c: char| c.is_whitespace() || c == '#' || c == '.') {
            continue;
        }
        let Some(at) = line.find(':') else { continue };
        if line[at..].starts_with(":=") || line[at..].starts_with("::=") || line[..at].contains(['=', '$', '%']) {
            continue;
        }
        for name in line[..at].split_whitespace() {
            if safe(name) && !found.iter().any(|f| f == name) {
                found.push(name.to_string());
            }
        }
    }
    found.truncate(40);
    found
}

/// Recipes of a justfile: `name args:` at the start of a line.
fn just_recipes(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in text.lines() {
        if line.starts_with(|c: char| c.is_whitespace() || c == '#') || !line.contains(':') {
            continue;
        }
        let head = line.split(':').next().unwrap_or("");
        let name = head.split_whitespace().next().unwrap_or("").trim_start_matches('@');
        if head.contains('=') || line.contains(":=") || matches!(name, "set" | "alias" | "import" | "export" | "mod") {
            continue;
        }
        if safe(name) && !found.iter().any(|f| f == name) {
            found.push(name.to_string());
        }
    }
    found.truncate(40);
    found
}

fn matches_repo(spec: &str, repo: &Path) -> bool {
    if spec.contains('/') || spec.starts_with('~') {
        repo.starts_with(crate::config::expand(spec))
    } else {
        repo.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(spec))
    }
}

/// The tasks of `repo`: what its project files offer, then `custom` ones (which replace a task of the same name).
pub fn detect(repo: &Path, custom: &[Custom]) -> Vec<Task> {
    let mut tasks: Vec<Task> = Vec::new();
    let mut add = |name: &str, cmd: String, source: &str| tasks.push(Task { name: name.to_string(), cmd, source: source.to_string() });
    let read = |file: &str| fs::read_to_string(repo.join(file)).ok();
    let has = |file: &str| repo.join(file).is_file();

    for (file, list) in KNOWN {
        if has(file) {
            for (name, cmd) in *list {
                add(name, cmd.to_string(), file);
            }
        }
    }
    if let Some(text) = read("package.json") {
        let runner = if has("pnpm-lock.yaml") {
            "pnpm run"
        } else if has("yarn.lock") {
            "yarn run"
        } else if has("bun.lock") || has("bun.lockb") {
            "bun run"
        } else {
            "npm run"
        };
        for (name, _) in json_strings(&text, "scripts").into_iter().filter(|(n, _)| safe(n)) {
            add(&name, format!("{runner} {name}"), "package.json");
        }
    }
    if let Some(text) = read("composer.json") {
        for (name, _) in json_strings(&text, "scripts").into_iter().filter(|(n, _)| safe(n)) {
            add(&name, format!("composer run-script {name}"), "composer.json");
        }
    }
    if has("build.gradle") || has("build.gradle.kts") {
        let gradle = if has("gradlew") { "./gradlew" } else { "gradle" };
        for name in ["build", "test", "check", "run"] {
            add(name, format!("{gradle} {name}"), "build.gradle");
        }
    }
    let top: Vec<String> = fs::read_dir(repo).map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    if top.iter().any(|n| n.ends_with(".sln") || n.ends_with(".csproj") || n.ends_with(".fsproj")) {
        for name in ["build", "test", "run"] {
            add(name, format!("dotnet {name}"), "dotnet");
        }
    }
    if has("pyproject.toml") || has("setup.py") || has("pytest.ini") || has("tox.ini") {
        add("test", "python -m pytest".to_string(), "python");
        if read("pyproject.toml").is_some_and(|t| t.contains("ruff")) {
            add("lint", "ruff check .".to_string(), "pyproject.toml");
        }
    }
    for file in ["Makefile", "makefile", "GNUmakefile"] {
        if let Some(text) = read(file) {
            for target in make_targets(&text) {
                add(&target, format!("make {target}"), file);
            }
            break;
        }
    }
    for file in ["justfile", "Justfile"] {
        if let Some(text) = read(file) {
            for recipe in just_recipes(&text) {
                add(&recipe, format!("just {recipe}"), file);
            }
            break;
        }
    }
    for c in custom.iter().filter(|c| c.repo.as_deref().is_none_or(|r| matches_repo(r, repo))) {
        let task = Task { name: c.name.clone(), cmd: c.run.clone(), source: "config.lua".to_string() };
        match tasks.iter().position(|t| t.name == c.name) {
            Some(i) => tasks[i] = task,
            None => tasks.push(task),
        }
    }
    // The same name from two files (a script `build` and a make target `build`): the first one wins.
    let mut seen = std::collections::HashSet::new();
    tasks.retain(|t| seen.insert(t.name.clone()));
    tasks
}

/// A running task. Dropping it stops the task.
pub struct Handle {
    pid: u32,
    done: Arc<AtomicBool>,
}

impl Handle {
    /// Asks the task's whole process group to stop, and kills it if it is still there after a few seconds.
    pub fn cancel(&self) {
        if self.done.load(Ordering::Relaxed) {
            return;
        }
        let group = format!("-{}", self.pid);
        let _ = Command::new("kill").args(["-TERM", "--", &group]).stdout(Stdio::null()).stderr(Stdio::null()).status();
        let done = Arc::clone(&self.done);
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(3));
            if !done.load(Ordering::Relaxed) {
                let _ = Command::new("kill").args(["-KILL", "--", &group]).stdout(Stdio::null()).stderr(Stdio::null()).status();
            }
        });
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.cancel(); // quitting Oxide must not leave a build running
    }
}

/// Removes terminal escape sequences (colors, cursor movement, window titles) from a line of output.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' || (c == '\x1b' && chars.next() == Some('\\')) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// One line of output as it should be shown: no escape sequences, a progress line that rewrites itself with
/// `\r` shown as its last state, tabs expanded.
fn tidy(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let last = text.rsplit('\r').find(|p| !p.is_empty()).unwrap_or("");
    clean(strip_ansi(last).trim_end())
}

/// Runs `cmd` with `sh` in `repo` in the background. Output (stdout and stderr together, in order) arrives as
/// `Msg::TaskLines` and the end as `Msg::TaskDone`, both carrying `id`.
pub fn spawn(repo: &Path, cmd: &str, id: u64, tx: Sender<Msg>) -> Result<Handle, String> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(format!("exec 2>&1\n{cmd}"))
        .current_dir(repo)
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .env("CARGO_TERM_COLOR", "never")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("cannot run sh: {e}"))?;
    let mut out = child.stdout.take().ok_or("cannot read the output")?;
    let done = Arc::new(AtomicBool::new(false));
    let handle = Handle { pid: child.id(), done: Arc::clone(&done) };
    thread::spawn(move || {
        let start = Instant::now();
        let (mut buf, mut pending) = ([0u8; 8192], Vec::<u8>::new());
        while let Ok(n) = out.read(&mut buf) {
            if n == 0 {
                break;
            }
            pending.extend_from_slice(&buf[..n]);
            let mut lines = Vec::new();
            while let Some(end) = pending.iter().position(|&b| b == b'\n') {
                lines.push(tidy(&pending[..end]));
                pending.drain(..=end);
            }
            if !lines.is_empty() && tx.send(Msg::TaskLines { id, lines }).is_err() {
                break;
            }
        }
        if !pending.is_empty() {
            let _ = tx.send(Msg::TaskLines { id, lines: vec![tidy(&pending)] });
        }
        let code = child.wait().ok().and_then(|s| s.code());
        done.store(true, Ordering::Relaxed);
        let _ = tx.send(Msg::TaskDone { id, code, secs: start.elapsed().as_secs_f64() });
    });
    Ok(handle)
}
