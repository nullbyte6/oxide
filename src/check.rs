//! `oxide --check-setups`: sets up every language variant of the new-project menu in a temporary folder and
//! checks the result, so a broken template, a wrong flag or a stale version pin shows up before a user hits it.
//!
//! Each variant is first linted (placeholders that do not exist, a suggested package its own page would refuse),
//! then set up exactly as the menu would, then built or run with the commands of `Variant::check`. A variant
//! whose tools are not installed is skipped, or only set up when just the check command is missing.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::{
    license::LICENSES,
    scaffold::{self, Action, Lang, PLACEHOLDERS, Plan, Step, Variant, Write as Mode},
};

/// Longest a single check command may run.
const TIMEOUT: Duration = Duration::from_secs(900);

/// Who owns the projects that are made.
const HOLDER: &str = "Tester";

enum Outcome {
    /// Set up and checked.
    Ok,
    /// Set up, but the program that checks it is not installed.
    SetupOnly(String),
    /// Not tried: a program the setup runs is not installed.
    Skip(String),
    Fail(String),
}

/// Checks the languages whose name contains one of `filter` (all of them when it is empty). True when nothing failed.
pub fn run(filter: &[String]) -> bool {
    let base = std::env::temp_dir().join(format!("oxide-check-{}", std::process::id()));
    let (mut ok, mut setup_only, mut skipped, mut failed) = (0, 0, 0, 0);
    let mut n = 0;
    for lang in scaffold::langs() {
        let wanted = |f: &String| lang.name.to_lowercase().contains(&f.to_lowercase());
        if !filter.is_empty() && !filter.iter().any(wanted) {
            continue;
        }
        for variant in lang.variants {
            n += 1;
            let label = format!("{} / {}", lang.name, variant.name);
            match check_one(&base.join(n.to_string()), lang, variant) {
                Outcome::Ok => {
                    ok += 1;
                    println!("ok        {label}");
                }
                Outcome::SetupOnly(tool) => {
                    setup_only += 1;
                    println!("setup     {label}  (set up; {tool} is not installed to check it)");
                }
                Outcome::Skip(tools) => {
                    skipped += 1;
                    println!("skipped   {label}  ({tools} not installed)");
                }
                Outcome::Fail(why) => {
                    failed += 1;
                    println!("FAILED    {label}");
                    for line in why.lines() {
                        println!("            {line}");
                    }
                }
            }
            let _ = std::io::stdout().flush();
        }
    }
    println!("\n{ok} ok, {setup_only} set up only, {skipped} skipped, {failed} failed");
    if failed > 0 {
        println!("projects and logs kept in {}", base.display());
    } else {
        let _ = fs::remove_dir_all(&base); // the folder made above, with nothing else in it
    }
    failed == 0
}

/// Lints, sets up and checks one variant in `dir`.
fn check_one(dir: &Path, lang: &Lang, variant: &Variant) -> Outcome {
    if let Some(problem) = lint(variant) {
        return Outcome::Fail(problem);
    }
    // Same name the menu would see, so every placeholder takes a realistic value.
    let root = dir.join("check-demo");
    if let Err(e) = fs::create_dir_all(&root) {
        return Outcome::Fail(format!("cannot create {}: {e}", root.display()));
    }
    let git = Command::new("git").args(["init", "-q"]).current_dir(&root).status();
    if !git.is_ok_and(|s| s.success()) {
        return Outcome::Fail("git init failed".to_string());
    }

    let packages = match scaffold::default_package(&root, lang, HOLDER) {
        Some(text) => match scaffold::parse_packages(&text, lang.package.map_or(scaffold::PackageKind::Packages, |p| p.kind)) {
            Ok(p) => p,
            Err(e) => return Outcome::Fail(format!("the suggested package '{text}' is refused by its own page: {e}")),
        },
        None => Vec::new(),
    };
    let plan = Plan::build(&root, lang, variant, &packages, LICENSES.first(), HOLDER, 2026);
    if !plan.missing.is_empty() {
        return Outcome::Skip(plan.missing.join(", "));
    }
    if let Err(e) = plan.apply(&root, |_| {}) {
        return Outcome::Fail(e);
    }
    if let Some(problem) = leftovers(&root, &plan) {
        return Outcome::Fail(problem);
    }

    let main = packages.first().map_or("", String::as_str);
    let log = dir.join("check.log");
    for command in variant.check {
        let line = scaffold::fill(&root, HOLDER, main, command);
        let mut words = line.split_whitespace();
        let Some(program) = words.next() else { continue };
        if !scaffold::on_path(program) {
            return Outcome::SetupOnly(program.to_string());
        }
        if let Err(e) = run_logged(program, words, &root, &log) {
            return Outcome::Fail(format!("$ {line}\n{e}"));
        }
    }
    Outcome::Ok
}

/// Placeholders in the table that nothing fills in: the check commands and the setup steps of `variant`.
fn lint(variant: &Variant) -> Option<String> {
    let mut texts: Vec<&str> = variant.check.to_vec();
    for step in variant.steps {
        match step {
            Step::Run(s) | Step::Dir(s) => texts.push(s),
            Step::File(path, text) => texts.extend([*path, *text]),
        }
    }
    texts
        .iter()
        .flat_map(|t| scaffold::braced_words(t))
        .find(|w| looks_like_placeholder(w))
        .map(|w| format!("unknown placeholder {{{w}}} in the language table"))
}

/// Single lowercase words are ordinary code in braces (`{x}`); placeholders are known ones, so anything that
/// resembles one (starts with a known name) but is not known is a typo.
fn looks_like_placeholder(word: &str) -> bool {
    PLACEHOLDERS.iter().any(|p| word.starts_with(p) || p.starts_with(word)) && !PLACEHOLDERS.contains(&word)
}

/// A known placeholder that is still in a written file or path: the fill missed it.
fn leftovers(root: &Path, plan: &Plan) -> Option<String> {
    for action in &plan.actions {
        let Action::File { path, text, mode } = action else { continue };
        if let Some(w) = scaffold::braced_words(path).into_iter().chain(scaffold::braced_words(text)).find(|w| PLACEHOLDERS.contains(w)) {
            return Some(format!("{{{w}}} was left unfilled in {path}"));
        }
        if *mode == Mode::Always && !root.join(path).is_file() {
            return Some(format!("{path} was not written"));
        }
    }
    None
}

/// Runs a command with its output in `log`; the error holds the last lines of that output.
fn run_logged<'a>(program: &str, args: impl Iterator<Item = &'a str>, dir: &Path, log: &PathBuf) -> Result<(), String> {
    let file = fs::File::create(log).map_err(|e| format!("cannot write {}: {e}", log.display()))?;
    let again = file.try_clone().map_err(|e| e.to_string())?;
    let mut child = Command::new(program)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(file)
        .stderr(again)
        .spawn()
        .map_err(|e| format!("cannot run {program}: {e}"))?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > TIMEOUT => {
                let _ = child.kill();
                return Err(format!("timed out after {} s", TIMEOUT.as_secs()));
            }
            Ok(None) => thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e.to_string()),
        }
    };
    if status.success() {
        return Ok(());
    }
    let text = fs::read_to_string(log).unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(8)..].join("\n");
    Err(format!("{status}\n{tail}"))
}
