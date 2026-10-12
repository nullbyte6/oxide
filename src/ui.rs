//! Rendering of the dashboard with Ratatui.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, Padding, Paragraph, Tabs},
};

use crate::{
    app::{App, Filter, Focus, InputKind, Mode},
    banner,
    git::FileEntry,
    logger::Level,
    repo::{Health, Owner, Repo},
};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const ACCENT: Color = Color::Cyan;

/// Rows the dashboard needs below the banner; on shorter terminals the banner is hidden.
const MIN_DASHBOARD_ROWS: u16 = 24;

/// Inner side padding of every panel. A terminal cell is about twice as tall as it is wide,
/// so one column of padding makes the sides look as thick as the one-row top and bottom borders.
const SIDE_PAD: u16 = 1;

pub fn draw(f: &mut Frame, app: &mut App) {
    let banner = banner::get();
    let banner_h = banner.lines.len() as u16;
    let show_banner = banner_h > 0 && f.area().height >= banner_h + MIN_DASHBOARD_ROWS && f.area().width >= banner.width;
    let [banner_area, header, body, log, footer] = Layout::vertical([
        Constraint::Length(if show_banner { banner_h } else { 0 }),
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(8),
        Constraint::Length(1),
    ])
    .areas(f.area());
    if show_banner {
        // Center the art as one block so its lines stay aligned with each other.
        let [_, art, _] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(banner.width), Constraint::Fill(1)]).areas(banner_area);
        f.render_widget(Paragraph::new(banner.lines.clone()), art);
    }
    let [left, right] = Layout::horizontal([Constraint::Percentage(54), Constraint::Percentage(46)]).areas(body);

    draw_header(f, app, header);
    draw_repos(f, app, left);
    draw_details(f, app, right);
    draw_log(f, app, log);
    draw_footer(f, app, footer);
    if app.mode == Mode::Help {
        draw_help(f);
    }
}

/// Block with a border that is highlighted while its pane has focus.
fn pane(title: String, focused: bool) -> Block<'static> {
    let color = if focused { ACCENT } else { Color::DarkGray };
    Block::bordered().padding(Padding::horizontal(SIDE_PAD)).border_style(Style::new().fg(color)).title(Span::styled(title, Style::new().fg(color).bold()))
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

fn health_color(h: Health) -> Color {
    match h {
        Health::Clean => Color::Green,
        Health::Dirty => Color::Yellow,
        Health::Conflict | Health::Error => Color::Red,
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let dirty = app.repos.iter().filter(|r| r.health() != Health::Clean).count();
    let ahead = app.repos.iter().filter(|r| r.ahead > 0).count();
    let behind = app.repos.iter().filter(|r| r.behind > 0).count();

    let mut right = vec![Span::raw(format!(
        " {} repos · {dirty} dirty · {ahead} ahead · {behind} behind ",
        app.repos.len()
    ))];
    if let Some((done, total)) = app.scan {
        right.insert(0, Span::styled(format!(" {} scanning {done}/{total} ", SPINNER[app.tick % SPINNER.len()]), Style::new().fg(Color::Yellow)));
    }
    if !app.search.is_empty() {
        right.insert(0, Span::styled(format!(" /{} ", app.search), Style::new().fg(Color::Magenta)));
    }

    let block = Block::bordered()
        .padding(Padding::horizontal(SIDE_PAD))
        .border_style(Style::new().fg(Color::DarkGray))
        .title(Span::styled(format!(" ◆ OXIDE v{} ", env!("CARGO_PKG_VERSION")), Style::new().fg(ACCENT).bold()))
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
            format!("{name} ({})", app.count(flt))
        })
        .collect();
    let tabs = Tabs::new(titles)
        .select(app.filter.index())
        .highlight_style(Style::new().fg(Color::Black).bg(ACCENT).bold())
        .divider("│");
    f.render_widget(tabs, inner);
}

fn repo_row(r: &Repo, busy: Option<&str>, tick: usize, name_w: usize) -> ListItem<'static> {
    let color = health_color(r.health());
    let marker = match busy {
        Some(_) => Span::styled(SPINNER[tick % SPINNER.len()], Style::new().fg(Color::Yellow)),
        None => Span::styled("●", Style::new().fg(color)),
    };
    let (badge, badge_color) = match r.owner {
        Owner::Owned => ("own  ", Color::Green),
        Owner::Cloned => ("clone", Color::Blue),
    };
    let mut spans = vec![
        marker,
        Span::raw(" "),
        Span::styled(fit(&r.name, name_w), Style::new().bold()),
        Span::raw(" "),
        Span::styled(badge, Style::new().fg(badge_color)),
        Span::raw(" "),
        Span::styled(fit(&r.branch, 14), Style::new().fg(Color::Magenta)),
        Span::raw(" "),
    ];
    if r.ahead > 0 {
        spans.push(Span::styled(format!("↑{} ", r.ahead), Style::new().fg(Color::Green)));
    }
    if r.behind > 0 {
        spans.push(Span::styled(format!("↓{} ", r.behind), Style::new().fg(Color::Red)));
    }
    if let Some(label) = busy {
        spans.push(Span::styled(format!("{label}… "), Style::new().fg(Color::Yellow)));
    }
    if r.error.is_some() {
        spans.push(Span::styled("error", Style::new().fg(Color::Red)));
    }
    for (n, sym, c) in [(r.conflicts(), "!", Color::Red), (r.staged(), "+", Color::Green), (r.unstaged(), "~", Color::Yellow), (r.untracked(), "?", Color::Blue)] {
        if n > 0 {
            spans.push(Span::styled(format!("{sym}{n} "), Style::new().fg(c)));
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
        f.render_widget(Paragraph::new(msg).dark_gray().block(block), area);
        return;
    }

    // Give the name column the spare width, between 12 and 28 characters.
    let name_w = (area.width as usize).saturating_sub(48).clamp(12, 28);
    let items: Vec<ListItem> = app
        .visible
        .iter()
        .map(|&i| {
            let r = &app.repos[i];
            repo_row(r, app.busy.get(&r.path).copied(), app.tick, name_w)
        })
        .collect();
    let list = List::new(items).block(block).highlight_style(Style::new().bg(Color::Rgb(40, 44, 52)).add_modifier(Modifier::BOLD)).highlight_symbol("▶ ");
    f.render_stateful_widget(list, area, &mut app.repo_state);
}

fn file_row(e: &FileEntry) -> ListItem<'static> {
    let (x, y) = if e.untracked() { ('?', '?') } else { (e.x, e.y) };
    let xc = if e.conflict() { Color::Red } else { Color::Green };
    let yc = if e.conflict() { Color::Red } else if e.untracked() { Color::Blue } else { Color::Yellow };
    ListItem::new(Line::from(vec![
        Span::styled(x.to_string(), Style::new().fg(xc)),
        Span::styled(y.to_string(), Style::new().fg(yc)),
        Span::raw(format!(" {}", e.path)),
    ]))
}

fn draw_details(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Files;
    let Some(r) = app.selected().cloned() else {
        f.render_widget(Paragraph::new("Select a repository.").dark_gray().block(pane(" Details ".into(), false)), area);
        return;
    };

    let [info_area, files_area] = Layout::vertical([Constraint::Length(9), Constraint::Min(3)]).areas(area);

    let label = |s: &'static str| Span::styled(format!("{s:<9}"), Style::new().fg(Color::DarkGray));
    let mut lines = vec![
        Line::from(vec![label("path"), Span::raw(r.path.display().to_string())]),
        Line::from(vec![
            label("branch"),
            Span::styled(r.branch.clone(), Style::new().fg(Color::Magenta)),
            Span::raw(match &r.upstream {
                Some(u) => format!("  → {u}"),
                None => "  (no upstream)".to_string(),
            }),
        ]),
        Line::from(vec![label("remote"), Span::raw(r.remote.clone().unwrap_or_else(|| "none (local only)".into()))]),
        Line::from(vec![
            label("owner"),
            match r.owner {
                Owner::Owned => Span::styled("owned", Style::new().fg(Color::Green)),
                Owner::Cloned => Span::styled("cloned", Style::new().fg(Color::Blue)),
            },
        ]),
        Line::from(vec![
            label("sync"),
            Span::styled(format!("↑{} ahead", r.ahead), Style::new().fg(Color::Green)),
            Span::raw("  "),
            Span::styled(format!("↓{} behind", r.behind), Style::new().fg(Color::Red)),
        ]),
        Line::from(vec![
            label("changes"),
            Span::styled(format!("{} staged", r.staged()), Style::new().fg(Color::Green)),
            Span::raw(" · "),
            Span::styled(format!("{} modified", r.unstaged()), Style::new().fg(Color::Yellow)),
            Span::raw(" · "),
            Span::styled(format!("{} untracked", r.untracked()), Style::new().fg(Color::Blue)),
        ]),
        Line::from(vec![label("commit"), Span::raw(r.last_commit.clone())]),
    ];
    if let Some(e) = &r.error {
        lines.push(Line::from(Span::styled(format!("error: {}", e.lines().next().unwrap_or("")), Style::new().fg(Color::Red))));
    }
    f.render_widget(Paragraph::new(lines).block(pane(format!(" {} ", r.name), false)), info_area);

    let block = pane(format!(" Changes ({}) ", r.files.len()), focused);
    if r.files.is_empty() {
        f.render_widget(Paragraph::new("Working tree clean ✓").green().block(block), files_area);
        return;
    }
    let items: Vec<ListItem> = r.files.iter().map(file_row).collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(if focused { Style::new().bg(Color::Rgb(40, 44, 52)).add_modifier(Modifier::BOLD) } else { Style::new() })
        .highlight_symbol(if focused { "▶ " } else { "  " });
    f.render_stateful_widget(list, files_area, &mut app.file_state);
}

fn draw_log(f: &mut Frame, app: &App, area: Rect) {
    let rows = area.height.saturating_sub(2) as usize;
    let start = app.log.len().saturating_sub(rows);
    let lines: Vec<Line> = app.log[start..]
        .iter()
        .map(|l| {
            let color = match l.level {
                Level::Info => Color::Gray,
                Level::Ok => Color::Green,
                Level::Warn => Color::Yellow,
                Level::Error => Color::Red,
            };
            Line::from(vec![Span::styled(format!("{} ", l.time), Style::new().fg(Color::DarkGray)), Span::styled(l.text.clone(), Style::new().fg(color))])
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(pane(" Log (UTC) ".into(), false)), area);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    if let Mode::Input(kind) = app.mode {
        let prompt = match kind {
            InputKind::Commit => format!("commit message → {}", app.selected().map_or("", |r| r.name.as_str())),
            InputKind::Merge => format!("merge into {} (empty = upstream)", app.selected().map_or("", |r| r.branch.as_str())),
            InputKind::Search => "search".to_string(),
        };
        let line = Line::from(vec![
            Span::styled(format!(" {prompt} ▸ "), Style::new().fg(ACCENT).bold()),
            Span::raw(app.input.clone()),
            Span::styled("█", Style::new().fg(ACCENT)),
        ]);
        f.render_widget(Paragraph::new(line), area);
        return;
    }

    let hints = [("↑↓", "move"), ("Tab", "pane"), ("f", "filter"), ("/", "search"), ("Space", "stage"), ("c", "commit"), ("p", "pull"), ("P", "push"), ("F", "fetch"), ("m", "merge"), ("R", "rescan"), ("?", "help"), ("q", "quit")];
    let mut spans = Vec::new();
    for (key, what) in hints {
        spans.push(Span::styled(format!(" {key}"), Style::new().fg(ACCENT).bold()));
        spans.push(Span::styled(format!(" {what} "), Style::new().fg(Color::DarkGray)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_help(f: &mut Frame) {
    let rows: [(&str, &str); 22] = [
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
        ("●", "green clean · yellow dirty · red conflict/error"),
        ("↑n ↓n", "commits ahead / behind upstream"),
        ("+ ~ ? !", "staged · modified · untracked · conflicts"),
        ("", "Any key closes this window."),
    ];
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, d)| Line::from(vec![Span::styled(format!("{k:<16}"), Style::new().fg(ACCENT).bold()), Span::raw(*d)]))
        .collect();

    let area = f.area();
    let w = 72.min(area.width);
    let h = (lines.len() as u16 + 2).min(area.height);
    let [_, mid, _] = Layout::vertical([Constraint::Fill(1), Constraint::Length(h), Constraint::Fill(1)]).areas(area);
    let [_, popup, _] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(w), Constraint::Fill(1)]).areas(mid);
    f.render_widget(Clear, popup);
    f.render_widget(Paragraph::new(lines).block(pane(" Oxide keys ".into(), true)), popup);
}
