//! Oxide: a terminal dashboard for every git repository on your machine.

mod app;
mod banner;
mod check;
mod code;
mod config;
mod diff;
mod git;
mod github;
mod ignore;
mod langs;
mod license;
mod logger;
mod marks;
mod palette;
mod repo;
mod scaffold;
mod syntax;
mod tasks;
mod term;
mod ui;
mod watch;
mod workflow;
mod worker;

use std::{
    io,
    process::ExitCode,
    sync::{Arc, mpsc},
    time::Duration,
};

use crossterm::event::{self, Event, KeyEventKind};
use ratatui::DefaultTerminal;

use app::App;
use config::Config;
use repo::{Owner, Repo};
use worker::Msg;

const HELP: &str = "\
Oxide - terminal dashboard for git repositories

USAGE:
    oxide [OPTIONS] [PATH...]

ARGS:
    PATH...        Directories to scan (default: 'root' of config.lua, else every mounted drive)

OPTIONS:
        --list     Print the repository table to stdout and exit (no TUI)
        --check-setups [LANG...]
                   Set up every language variant of the new-project menu (only those whose name contains
                   a LANG, if given) in a temporary folder, build it, and report; exit 1 if any failed
    -h, --help     Show this help
    -V, --version  Show the version

FILES:
    ~/.config/oxide/config.lua   Lua script: root, owner, email, depth, color, hints, logs, ci, autofetch, clone_dir, pin, ignore, languages
    ~/.local/state/oxide/oxide.log
    ~/.local/state/oxide/marks   repositories pinned or ignored with the * and x keys
";

fn main() -> ExitCode {
    let mut paths = Vec::new();
    let mut list = false;
    let mut check = false;
    let mut words: Vec<String> = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("oxide {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "--list" => list = true,
            "--check-setups" => check = true,
            a if a.starts_with('-') => {
                eprintln!("oxide: unknown option '{a}'\n\n{HELP}");
                return ExitCode::from(2);
            }
            a => {
                words.push(a.to_string());
                paths.push(config::expand(a));
            }
        }
    }
    let cfg = Config::load(paths);
    scaffold::register(cfg.languages.clone());
    scaffold::set_pins(cfg.versions.clone());

    if check {
        if let Some(e) = &cfg.error {
            eprintln!("oxide: config.lua ignored: {e}");
        }
        return if check::run(&words) { ExitCode::SUCCESS } else { ExitCode::FAILURE };
    }

    if list {
        if let Some(e) = &cfg.error {
            eprintln!("oxide: config.lua ignored: {e}");
        }
        print_list(cfg);
        return ExitCode::SUCCESS;
    }

    let mut app = App::new(cfg);
    let mut terminal = ratatui::init(); // also installs a panic hook that restores the terminal
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("oxide: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Event loop: draw, handle one key press (or time out after 100 ms), apply worker results.
fn run(terminal: &mut DefaultTerminal, app: &mut App) -> io::Result<()> {
    app.start_scan();
    while !app.quit {
        terminal.draw(|f| ui::draw(f, app))?;
        // Shell output arrives on its own, so the embedded terminal is redrawn much more often.
        let wait = if app.mode == app::Mode::Terminal { 16 } else { 100 };
        if event::poll(Duration::from_millis(wait))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.on_key(key);
        }
        // An editor or shell needs the whole terminal: leave the dashboard, run it, come back.
        if let Some(launch) = app.launch.take() {
            ratatui::restore();
            let result = launch.run();
            *terminal = ratatui::try_init()?;
            app.launch_done(launch.repo(), result);
        }
        app.drain();
        app.background();
        app.tick = app.tick.wrapping_add(1);
    }
    Ok(())
}

/// Non-interactive output for `--list`: one line per repository.
fn print_list(cfg: Config) {
    let cfg = Arc::new(cfg);
    let (tx, rx) = mpsc::channel();
    worker::spawn_scan(cfg, tx);

    let mut repos: Vec<Repo> = Vec::new();
    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Repo(r) => repos.push(r),
            Msg::ScanDone(_) => break,
            _ => {}
        }
    }
    repos.sort_by(|a, b| a.path.cmp(&b.path));
    for r in repos {
        let owner = if r.owner == Owner::Owned { "owned" } else { "cloned" };
        let state = if r.error.is_some() { "error" } else if r.files.is_empty() { "clean" } else { "dirty" };
        println!("{owner:<7}{state:<7}{:<16} +{} ~{} ?{} ↑{} ↓{}  {}", r.branch, r.staged(), r.unstaged(), r.untracked(), r.ahead, r.behind, r.path.display());
    }
}
