//! Rendering of the dashboard with Ratatui.
//!
//! The design is monochrome: one ink color on the terminal's own background. Hierarchy and
//! state come from bold, dim and inverse text, thick borders and symbols, never from hues.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, List, ListItem, Padding, Paragraph, Tabs},
};

use crate::{
    app::{App, Filter, Focus, InputKind, Mode},
    banner,
    git::FileEntry,
    logger::{self, Level},
    repo::{Health, Owner, Repo},
};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The one color of the whole interface. `Reset` is the terminal's own foreground;
/// set any other color here to recolor all of Oxide at once.
const INK: Color = Color::Reset;

const PLAIN: Style = Style::new().fg(INK);
const BOLD: Style = PLAIN.add_modifier(Modifier::BOLD);
const DIM: Style = PLAIN.add_modifier(Modifier::DIM);
const ITALIC: Style = PLAIN.add_modifier(Modifier::ITALIC);
/// Selection and active tab: swaps ink and background.
const INVERSE: Style = PLAIN.add_modifier(Modifier::REVERSED).add_modifier(Modifier::BOLD);
/// Selection in a pane that does not have focus: inverse, but faded.
const INVERSE_DIM: Style = PLAIN.add_modifier(Modifier::REVERSED).add_modifier(Modifier::DIM);
/// Key caps in the footer.
const KEYCAP: Style = PLAIN.add_modifier(Modifier::REVERSED);

/// Empty columns between side-by-side panels and empty rows between stacked panels.
/// A terminal cell is about twice as tall as it is wide, so the 2 adjacent border columns with
/// a 1-column gap are as far apart as 2 stacked border rows with no gap.
const GAP_X: u16 = 1;
const GAP_Y: u16 = 0;

/// Inner side padding of every panel (same cell-ratio reasoning: the sides get a column of
/// padding to match the one-row borders on top and bottom).
const SIDE_PAD: u16 = 1;

/// Rows the dashboard needs besides the banner; on shorter terminals the banner is hidden.
const MIN_DASHBOARD_ROWS: u16 = 24;

pub fn draw(f: &mut Frame, app: &mut App) {
    // Paint the ink color over the whole screen so text without an explicit style uses it too.
    f.render_widget(Block::new().style(PLAIN), f.area());

    let [mut area, footer] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(f.area());

    let banner = banner::get();
    let banner_h = banner.lines.len() as u16;
    if banner_h > 0 && f.area().height >= banner_h + GAP_Y + MIN_DASHBOARD_ROWS && f.area().width >= banner.width {
        let [banner_area, rest] = Layout::vertical([Constraint::Length(banner_h), Constraint::Min(0)]).spacing(GAP_Y).areas(area);
        // Center the art as one block so its lines stay aligned with each other.
        let [_, art, _] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(banner.width), Constraint::Fill(1)]).areas(banner_area);
        f.render_widget(Paragraph::new(banner.lines.clone()), art);
        area = rest;
    }

    let [header, body, log] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(8), Constraint::Length(8)]).spacing(GAP_Y).areas(area);
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(54), Constraint::Fill(1)]).spacing(GAP_X).areas(body);

    draw_header(f, app, header);
    draw_repos(f, app, left);
    draw_details(f, app, right);
    draw_log(f, app, log);
    draw_footer(f, app, footer);
    if app.mode == Mode::Help {
        draw_help(f);
    }
}

/// Panel frame: a thick bold border while the pane has focus, a thin dim one otherwise.
/// The title sits in a `─┤ title ├─` notch.
fn pane(title: String, focused: bool) -> Block<'static> {
    let (border, style) = if focused { (BorderType::Thick, BOLD) } else { (BorderType::Plain, DIM) };
    let title = Line::from(vec![
        Span::styled("─┤ ", style),
        Span::styled(title.trim().to_string(), style),
        Span::styled(" ├", style),
    ]);
    Block::bordered()
        .border_type(border)
        .padding(Padding::horizontal(SIDE_PAD))
        .border_style(style)
        .title(title)
}

/// Truncates (with `…`) or pads `s` to exactly `w` characters.
fn fit(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n > w {
        let mut t: String = s.chars().take(w.saturating_sub(1)).collect();
        t.push('…');
        t
    } else {
        format!("{s:<w$}")
    }
}

/// Progress bar of `width` cells, e.g. `▰▰▰▱▱▱▱▱▱▱`.
fn bar(done: usize, total: usize, width: usize) -> String {
    let filled = (done * width).checked_div(total).unwrap_or(0).min(width);
    format!("{}{}", "▰".repeat(filled), "▱".repeat(width - filled))
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let dirty = app.repos.iter().filter(|r| r.health() != Health::Clean).count();
    let ahead = app.repos.iter().filter(|r| r.ahead > 0).count();
    let behind = app.repos.iter().filter(|r| r.behind > 0).count();
    let spin = SPINNER[app.tick % SPINNER.len()];

    // Right side, left to right: search, running operations, scan progress, totals.
    let mut right = Vec::new();
    if !app.search.is_empty() {
        right.push(Span::styled(format!(" /{} ", app.search), BOLD));
    }
    if !app.busy.is_empty() {
        right.push(Span::styled(format!(" {spin} {} running ", app.busy.len()), BOLD));
    }
    match app.scan {
        Some((_, 0)) => right.push(Span::styled(format!(" {spin} discovering "), BOLD)),
        Some((done, total)) => right.push(Span::styled(format!(" {spin} scanning {} {done}/{total} ", bar(done, total, 10)), BOLD)),
        None => {}
    }
    right.push(Span::styled(format!(" {} repos · {dirty} dirty · {ahead} ahead · {behind} behind ", app.repos.len()), DIM));

    let block = Block::bordered()
        .padding(Padding::horizontal(SIDE_PAD))
        .border_style(DIM)
        .title(Line::from(vec![
            Span::styled("─┤ ", DIM),
            Span::styled("◆ OXIDE", BOLD),
            Span::styled(format!(" v{}", env!("CARGO_PKG_VERSION")), DIM),
            Span::styled(" ├", DIM),
        ]))
        .title_top(Line::from(right).right_aligned());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let titles: Vec<String> = Filter::ALL
        .iter()
        .map(|&flt| {
            let name = match flt {
                Filter::All => "All",
                Filter::Owned => "Owned",
                Filter::Cloned => "Cloned",
            };
            format!(" {name} {} ", app.count(flt))
        })
        .collect();
    let tabs = Tabs::new(titles).style(PLAIN).select(app.filter.index()).highlight_style(INVERSE).divider(Span::styled("┊", DIM));
    f.render_widget(tabs, inner);
}

fn repo_row(r: &Repo, busy: Option<&str>, tick: usize, name_w: usize) -> ListItem<'static> {
    // Cloned repositories are dimmed so owned ones stand out.
    let (badge, badge_style) = match r.owner {
        Owner::Owned => ("own  ", PLAIN),
        Owner::Cloned => ("clone", DIM),
    };
    let mut spans = vec![
        Span::styled(fit(&r.name, name_w), BOLD),
        Span::raw(" "),
        Span::styled(badge, badge_style),
        Span::raw(" "),
        Span::styled(fit(&r.branch, 14), PLAIN),
        Span::raw(" "),
    ];
    if r.ahead > 0 {
        spans.push(Span::styled(format!("↑{} ", r.ahead), PLAIN));
    }
    if r.behind > 0 {
        spans.push(Span::styled(format!("↓{} ", r.behind), PLAIN));
    }
    if let Some(label) = busy {
        spans.push(Span::styled(format!("{} {label}… ", SPINNER[tick % SPINNER.len()]), BOLD));
    }
    if r.error.is_some() {
        spans.push(Span::styled("error", BOLD));
    }
    for (n, sym) in [(r.conflicts(), "!"), (r.staged(), "+"), (r.unstaged(), "~"), (r.untracked(), "?")] {
        if n > 0 {
            spans.push(Span::styled(format!("{sym}{n} "), PLAIN));
        }
    }
    ListItem::new(Line::from(spans))
}

fn draw_repos(f: &mut Frame, app: &mut App, area: Rect) {
    let filter = match app.filter {
        Filter::All => "all",
        Filter::Owned => "owned",
        Filter::Cloned => "cloned",
    };
    let block = pane(format!(" Repositories · {filter} ({}) ", app.visible.len()), app.focus == Focus::Repos);

    if app.visible.is_empty() {
        let msg = if app.scan.is_some() { "Scanning for repositories…" } else { "No repositories match. Press R to rescan." };
        f.render_widget(Paragraph::new(msg).style(DIM.add_modifier(Modifier::ITALIC)).block(block), area);
        return;
    }

    // Give the name column the spare width, between 12 and 28 characters.
    let name_w = (area.width as usize).saturating_sub(46).clamp(12, 28);
    let items: Vec<ListItem> = app
        .visible
        .iter()
        .map(|&i| {
            let r = &app.repos[i];
            repo_row(r, app.busy.get(&r.path).copied(), app.tick, name_w)
        })
        .collect();
    let selection = if app.focus == Focus::Repos { INVERSE } else { INVERSE_DIM };
    let list = List::new(items).block(block).highlight_style(selection);
    f.render_stateful_widget(list, area, &mut app.repo_state);
}

/// `XY path`: the index letter is bold when staged, untracked files are dimmed.
fn file_row(e: &FileEntry) -> ListItem<'static> {
    if e.untracked() {
        return ListItem::new(Line::from(Span::styled(format!("?? {}", e.path), DIM)));
    }
    let x_style = if e.staged() || e.conflict() { BOLD } else { PLAIN };
    ListItem::new(Line::from(vec![
        Span::styled(e.x.to_string(), x_style),
        Span::styled(e.y.to_string(), PLAIN),
        Span::styled(format!(" {}", e.path), PLAIN),
    ]))
}

/// A counter that is dimmed while it is zero, so only non-zero values stand out.
fn count_span(n: usize, prefix: &str, suffix: &str) -> Span<'static> {
    Span::styled(format!("{prefix}{n}{suffix}"), if n == 0 { DIM } else { BOLD })
}

/// `<hash> <subject> (<relative date>)` with the hash dimmed.
fn commit_line(commit: &str, label: Span<'static>) -> Line<'static> {
    match commit.split_once(' ') {
        Some((hash, rest)) => Line::from(vec![label, Span::styled(format!("{hash} "), DIM), Span::styled(rest.to_string(), PLAIN)]),
        None => Line::from(vec![label, Span::styled(commit.to_string(), DIM)]),
    }
}

fn draw_details(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Files;
    let Some(r) = app.selected().cloned() else {
        f.render_widget(Paragraph::new("Select a repository.").style(DIM.add_modifier(Modifier::ITALIC)).block(pane(" Details ".into(), false)), area);
        return;
    };

    let [info_area, files_area] = Layout::vertical([Constraint::Length(9), Constraint::Min(3)]).spacing(GAP_Y).areas(area);

    let label = |s: &'static str| Span::styled(format!("{s:<9}"), DIM);
    let mut lines = vec![
        Line::from(vec![label("path"), Span::styled(r.path.display().to_string(), ITALIC)]),
        Line::from(vec![
            label("branch"),
            Span::styled(r.branch.clone(), BOLD),
            Span::styled(
                match &r.upstream {
                    Some(u) => format!("  → {u}"),
                    None => "  (no upstream)".to_string(),
                },
                PLAIN,
            ),
        ]),
        Line::from(vec![label("remote"), Span::styled(r.remote.clone().unwrap_or_else(|| "none (local only)".into()), ITALIC)]),
        Line::from(vec![label("owner"), Span::styled(if r.owner == Owner::Owned { "owned" } else { "cloned" }, PLAIN)]),
        Line::from(vec![
            label("sync"),
            count_span(r.ahead as usize, "↑", " ahead"),
            Span::raw("  "),
            count_span(r.behind as usize, "↓", " behind"),
        ]),
        Line::from(vec![
            label("changes"),
            count_span(r.staged(), "", " staged"),
            Span::styled(" · ", DIM),
            count_span(r.unstaged(), "", " modified"),
            Span::styled(" · ", DIM),
            count_span(r.untracked(), "", " untracked"),
        ]),
        commit_line(&r.last_commit, label("commit")),
    ];
    if let Some(e) = &r.error {
        lines.push(Line::from(Span::styled(format!("error: {}", e.lines().next().unwrap_or("")), BOLD)));
    }
    f.render_widget(Paragraph::new(lines).block(pane(format!(" {} ", r.name), false)), info_area);

    let block = pane(format!(" Changes ({}) ", r.files.len()), focused);
    if r.files.is_empty() {
        f.render_widget(Paragraph::new("Working tree clean ✓").style(DIM.add_modifier(Modifier::ITALIC)).block(block), files_area);
        return;
    }
    let items: Vec<ListItem> = r.files.iter().map(file_row).collect();
    let list = List::new(items).block(block).highlight_style(if focused { INVERSE } else { INVERSE_DIM });
    f.render_stateful_widget(list, files_area, &mut app.file_state);
}

fn draw_log(f: &mut Frame, app: &App, area: Rect) {
    let rows = area.height.saturating_sub(2) as usize;
    let start = app.log.len().saturating_sub(rows);
    let lines: Vec<Line> = app.log[start..]
        .iter()
        .enumerate()
        .map(|(i, l)| {
            // Level is shown by a symbol and weight: dim info, plain ok, bold warnings and errors.
            let (sym, style) = match l.level {
                Level::Info => ("  ", DIM),
                Level::Ok => ("✓ ", PLAIN),
                Level::Warn => ("! ", BOLD),
                Level::Error => ("✗ ", BOLD),
            };
            // Entries fade out as they age: only the newest three keep their full weight.
            let age = app.log.len() - start - 1 - i;
            let style = if age >= 3 { style.add_modifier(Modifier::DIM) } else { style };
            Line::from(vec![
                Span::styled(format!("{} ", l.time), DIM),
                Span::styled(sym, style),
                Span::styled(l.text.clone(), style),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(pane(format!(" Log ({}) ", logger::zone_name()), false)), area);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    if let Mode::Input(kind) = app.mode {
        let prompt = match kind {
            InputKind::Commit => format!("commit message → {}", app.selected().map_or("", |r| r.name.as_str())),
            InputKind::Merge => format!("merge into {} (empty = upstream)", app.selected().map_or("", |r| r.branch.as_str())),
            InputKind::Search => "search".to_string(),
        };
        let line = Line::from(vec![
            Span::styled(format!(" {prompt} ▸ "), BOLD),
            Span::styled(app.input.clone(), PLAIN),
            Span::styled("█", PLAIN),
        ]);
        f.render_widget(Paragraph::new(line), area);
        return;
    }

    let hints = [
        ("↑↓", "move"),
        ("Tab", "pane"),
        ("f", "filter"),
        ("/", "search"),
        ("Space", "stage"),
        ("c", "commit"),
        ("p", "pull"),
        ("P", "push"),
        ("F", "fetch"),
        ("m", "merge"),
        ("?", "help"),
        ("q", "quit"),
    ];
    let mut spans = Vec::new();
    for (key, what) in hints {
        spans.push(Span::styled(format!(" {key} "), KEYCAP));
        spans.push(Span::styled(format!(" {what}  "), DIM));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_help(f: &mut Frame) {
    let rows: [(&str, &str); 21] = [
        ("↑ ↓ / j k", "move selection"),
        ("g / G", "first / last"),
        ("Tab, ← →, h l", "switch between repositories and changes"),
        ("f, 1 2 3", "filter: all / owned / cloned"),
        ("/", "search by name, branch or path (Esc clears)"),
        ("Space, Enter", "stage / unstage the highlighted file"),
        ("a / u", "stage all / unstage all"),
        ("c", "commit staged changes"),
        ("p", "pull (fast-forward only)"),
        ("P", "push (publishes new branches with -u)"),
        ("F", "fetch selected repository"),
        ("A", "fetch every repository in the current view"),
        ("m", "merge a branch (empty = upstream)"),
        ("r", "reload selected repository"),
        ("R", "rescan all roots"),
        ("?", "this help"),
        ("q, Ctrl+C", "quit"),
        ("", ""),
        ("↑n ↓n", "commits ahead / behind upstream"),
        ("+ ~ ? !", "staged · modified · untracked · conflicts"),
        ("", "Any key closes this window."),
    ];
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, d)| Line::from(vec![Span::styled(format!("{k:<16}"), BOLD), Span::styled(*d, PLAIN)]))
        .collect();

    let area = f.area();
    let w = 72.min(area.width);
    let h = (lines.len() as u16 + 2).min(area.height);
    let [_, mid, _] = Layout::vertical([Constraint::Fill(1), Constraint::Length(h), Constraint::Fill(1)]).areas(area);
    let [_, popup, _] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(w), Constraint::Fill(1)]).areas(mid);
    f.render_widget(Clear, popup);
    let block = Block::bordered()
        .border_type(BorderType::Double)
        .padding(Padding::horizontal(SIDE_PAD))
        .border_style(BOLD)
        .title(Line::from(vec![Span::styled("═╡ ", BOLD), Span::styled("◆ OXIDE keys", BOLD), Span::styled(" ╞", BOLD)]));
    f.render_widget(Paragraph::new(lines).block(block), popup);
}
