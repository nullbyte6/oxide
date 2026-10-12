//! Rendering of the dashboard with Ratatui.
//!
//! The design is monochrome at its core: one ink color on the terminal's own background. Hierarchy
//! and state come from bold, dim and inverse text, thick borders and symbols. Colors are an optional
//! layer on top (`hue`): every colored span keeps the weight and symbol it has without color, so
//! turning colors off (`C`, `color = off` or `NO_COLOR`) leaves the monochrome look unchanged.

use std::sync::atomic::{AtomicBool, Ordering};

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
    diff::{Row, Sign},
    git::{Branch, DiffKind, FileEntry},
    github::Kind,
    logger::{self, Level},
    palette,
    repo::{Health, Owner, Repo},
    syntax::Tok,
};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The base color of the whole interface. `Reset` is the terminal's own foreground;
/// set any other color here to recolor all of Oxide at once.
const INK: Color = Color::Reset;

/// Accent hues of the color layer. They are the terminal's ANSI colors, so they follow its theme.
const ACCENT: Color = Color::Cyan; // focus, branch names, the logo
const GOOD: Color = Color::Green; // owned, ahead, staged, added, success
const WARN: Color = Color::Yellow; // modified, dirty, warnings, pins
const BAD: Color = Color::Red; // behind, conflicts, deleted, errors
const INFO: Color = Color::Blue; // untracked files, search
const PINK: Color = Color::Magenta; // running operations, stashes

/// Whether `hue` draws colors; set from `App::color` at the start of every frame.
static COLOR: AtomicBool = AtomicBool::new(true);

/// `style` in the accent color `c`, or unchanged when colors are off.
fn hue(style: Style, c: Color) -> Style {
    if COLOR.load(Ordering::Relaxed) { style.fg(c) } else { style }
}

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
    COLOR.store(app.color, Ordering::Relaxed);
    // Paint the ink color over the whole screen so text without an explicit style uses it too.
    f.render_widget(Block::new().style(PLAIN), f.area());

    let [mut area, footer] = Layout::vertical([Constraint::Min(0), Constraint::Length(footer_height(app))]).areas(f.area());

    let banner = banner::get();
    let banner_h = banner.lines.len() as u16;
    if banner_h > 0 && f.area().height >= banner_h + GAP_Y + MIN_DASHBOARD_ROWS && f.area().width >= banner.width {
        let [banner_area, rest] = Layout::vertical([Constraint::Length(banner_h), Constraint::Min(0)]).spacing(GAP_Y).areas(area);
        // Center the art as one block so its lines stay aligned with each other.
        let [_, art, _] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(banner.width), Constraint::Fill(1)]).areas(banner_area);
        f.render_widget(Paragraph::new(banner.lines.clone()).style(hue(BOLD, ACCENT)), art);
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
    // A pending confirmation is drawn on top of the view it was asked from.
    let view = match (app.mode, &app.confirm) {
        (Mode::Confirm, Some(c)) => c.back,
        (mode, _) => mode,
    };
    match view {
        Mode::Diff => draw_diff(f, app),
        Mode::History => draw_history(f, app),
        Mode::Branches => draw_branches(f, app),
        Mode::GitHub => draw_github(f, app),
        Mode::Text => draw_text(f, app),
        Mode::Finder => draw_finder(f, app),
        _ => {}
    }
    if app.mode == Mode::Palette {
        draw_palette(f, app);
    }
    if app.mode == Mode::Help {
        draw_help(f);
    }
}

/// Panel frame: a thick bold border while the pane has focus, a thin dim one otherwise.
/// The title sits in a `─┤ title ├─` notch.
fn pane(title: String, focused: bool) -> Block<'static> {
    let (border, style) = if focused { (BorderType::Thick, hue(BOLD, ACCENT)) } else { (BorderType::Plain, DIM) };
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
    let dirty = app.listed().filter(|r| r.health() != Health::Clean).count();
    let ahead = app.listed().filter(|r| r.ahead > 0).count();
    let behind = app.listed().filter(|r| r.behind > 0).count();
    let spin = SPINNER[app.tick % SPINNER.len()];

    // Right side, left to right: search, running operations, scan progress, totals.
    let mut right = Vec::new();
    if !app.search.is_empty() {
        right.push(Span::styled(format!(" /{} ", app.search), hue(BOLD, INFO)));
    }
    if !app.busy.is_empty() {
        right.push(Span::styled(format!(" {spin} {} running ", app.busy.len()), hue(BOLD, PINK)));
    }
    match app.scan {
        Some((_, 0)) => right.push(Span::styled(format!(" {spin} discovering "), hue(BOLD, ACCENT))),
        Some((done, total)) => right.push(Span::styled(format!(" {spin} scanning {} {done}/{total} ", bar(done, total, 10)), hue(BOLD, ACCENT))),
        None => {}
    }
    // Totals: each non-zero count takes its hue, zero counts stay dim.
    right.push(Span::styled(format!(" {} repos", app.listed().count()), DIM));
    for (n, label, color) in [(dirty, "dirty", WARN), (ahead, "ahead", GOOD), (behind, "behind", BAD)] {
        right.push(Span::styled(" · ", DIM));
        right.push(Span::styled(format!("{n} {label}"), if n == 0 { DIM } else { hue(PLAIN, color) }));
    }
    right.push(Span::styled(" ", DIM));

    let block = Block::bordered()
        .padding(Padding::horizontal(SIDE_PAD))
        .border_style(DIM)
        .title(Line::from(vec![
            Span::styled("─┤ ", DIM),
            Span::styled("◆ OXIDE", hue(BOLD, ACCENT)),
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
                Filter::Attention => "Attention",
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
        Owner::Owned => ("own  ", hue(PLAIN, GOOD)),
        Owner::Cloned => ("clone", DIM),
    };
    // The two columns left of the name are breathing room inside the selection bar; they also hold
    // the pin and ignore marks.
    let mark = if r.pinned {
        "★ "
    } else if r.ignored {
        "⊘ "
    } else {
        "  "
    };
    let mut spans = vec![
        Span::styled(mark, if r.ignored { DIM } else { hue(BOLD, WARN) }),
        Span::styled(fit(&r.name, name_w), BOLD),
        Span::raw(" "),
        Span::styled(badge, badge_style),
        Span::raw(" "),
        Span::styled(fit(&r.branch, 14), hue(PLAIN, ACCENT)),
        Span::raw(" "),
    ];
    if r.ahead > 0 {
        spans.push(Span::styled(format!("↑{} ", r.ahead), hue(PLAIN, GOOD)));
    }
    if r.behind > 0 {
        spans.push(Span::styled(format!("↓{} ", r.behind), hue(PLAIN, BAD)));
    }
    if let Some(label) = busy {
        spans.push(Span::styled(format!("{} {label}… ", SPINNER[tick % SPINNER.len()]), hue(BOLD, PINK)));
    }
    if r.error.is_some() {
        spans.push(Span::styled("error", hue(BOLD, BAD)));
    }
    let counters = [
        (r.conflicts(), "!", BAD),
        (r.staged(), "+", GOOD),
        (r.unstaged(), "~", WARN),
        (r.untracked(), "?", INFO),
        (r.stashes as usize, "$", PINK),
    ];
    for (n, sym, color) in counters {
        if n > 0 {
            spans.push(Span::styled(format!("{sym}{n} "), hue(PLAIN, color)));
        }
    }
    ListItem::new(Line::from(spans))
}

fn draw_repos(f: &mut Frame, app: &mut App, area: Rect) {
    let hidden = if app.show_hidden { " · +ignored" } else { "" };
    let title = format!(" Repositories · {} ({}) · by {}{hidden} ", app.filter.name(), app.visible.len(), app.sort.name());
    let block = pane(title, app.focus == Focus::Repos);

    if app.visible.is_empty() {
        let msg = if app.scan.is_some() { "Scanning for repositories…" } else { "No repositories match. Press R to rescan." };
        f.render_widget(Paragraph::new(msg).style(DIM.add_modifier(Modifier::ITALIC)).block(block), area);
        return;
    }

    // Give the name column the spare width, between 12 and 28 characters.
    let name_w = (area.width as usize).saturating_sub(52).clamp(12, 28);
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

/// Hue of a git status letter.
fn status_color(c: char) -> Color {
    match c {
        'A' => GOOD,
        'M' | 'T' => WARN,
        'D' | 'U' => BAD,
        'R' | 'C' => ACCENT,
        _ => INFO,
    }
}

/// `XY path`: the index letter is bold when staged, untracked files are dimmed.
fn file_row(e: &FileEntry) -> ListItem<'static> {
    if e.untracked() {
        return ListItem::new(Line::from(Span::styled(format!("?? {}", e.path), hue(DIM, INFO))));
    }
    let x_style = if e.staged() || e.conflict() { BOLD } else { PLAIN };
    let color = if e.conflict() { BAD } else { status_color(e.x) };
    ListItem::new(Line::from(vec![
        Span::styled(e.x.to_string(), hue(x_style, color)),
        Span::styled(e.y.to_string(), hue(PLAIN, status_color(e.y))),
        Span::styled(format!(" {}", e.path), PLAIN),
    ]))
}

/// A counter that is dimmed while it is zero, so only non-zero values stand out.
fn count_span(n: usize, prefix: &str, suffix: &str, color: Color) -> Span<'static> {
    Span::styled(format!("{prefix}{n}{suffix}"), if n == 0 { DIM } else { hue(BOLD, color) })
}

/// `<hash> <subject> (<relative date>)` with the hash dimmed.
fn commit_line(commit: &str, label: Span<'static>) -> Line<'static> {
    match commit.split_once(' ') {
        Some((hash, rest)) => Line::from(vec![label, Span::styled(format!("{hash} "), hue(DIM, WARN)), Span::styled(rest.to_string(), PLAIN)]),
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
            Span::styled(r.branch.clone(), hue(BOLD, ACCENT)),
            Span::styled(
                match &r.upstream {
                    Some(u) => format!("  → {u}"),
                    None => "  (no upstream)".to_string(),
                },
                PLAIN,
            ),
        ]),
        Line::from(vec![label("remote"), Span::styled(r.remote.clone().unwrap_or_else(|| "none (local only)".into()), ITALIC)]),
        Line::from(vec![
            label("owner"),
            if r.owner == Owner::Owned { Span::styled("owned", hue(PLAIN, GOOD)) } else { Span::styled("cloned", PLAIN) },
        ]),
        Line::from(vec![
            label("sync"),
            count_span(r.ahead as usize, "↑", " ahead", GOOD),
            Span::raw("  "),
            count_span(r.behind as usize, "↓", " behind", BAD),
        ]),
        Line::from(vec![
            label("changes"),
            count_span(r.staged(), "", " staged", GOOD),
            Span::styled(" · ", DIM),
            count_span(r.unstaged(), "", " modified", WARN),
            Span::styled(" · ", DIM),
            count_span(r.untracked(), "", " untracked", INFO),
            Span::styled(" · ", DIM),
            count_span(r.stashes as usize, "", " stashed", PINK),
        ]),
        commit_line(&r.last_commit, label("commit")),
    ];
    if let Some(e) = &r.error {
        lines.push(Line::from(Span::styled(format!("error: {}", e.lines().next().unwrap_or("")), hue(BOLD, BAD))));
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
                Level::Ok => ("✓ ", hue(PLAIN, GOOD)),
                Level::Warn => ("! ", hue(BOLD, WARN)),
                Level::Error => ("✗ ", hue(BOLD, BAD)),
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

/// The bottom row shows the key bar unless `h` hid it. Prompts (text input, confirmations) always need it.
fn footer_height(app: &App) -> u16 {
    u16::from(app.hints || matches!(app.mode, Mode::Input(_) | Mode::Confirm))
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    if let Mode::Input(kind) = app.mode {
        let prompt = match kind {
            InputKind::Commit => format!("commit message → {}", app.selected().map_or("", |r| r.name.as_str())),
            InputKind::Merge => format!("merge into {} (empty = upstream)", app.selected().map_or("", |r| r.branch.as_str())),
            InputKind::Search => "search".to_string(),
            InputKind::Branch => format!("new branch from {}", app.selected().map_or("", |r| r.branch.as_str())),
            InputKind::Init => "git init in folder (created if missing)".to_string(),
            InputKind::GhSearch => "search GitHub repositories (empty = yours)".to_string(),
            InputKind::CloneTo => "clone into".to_string(),
        };
        let line = Line::from(vec![
            Span::styled(format!(" {prompt} ▸ "), hue(BOLD, ACCENT)),
            Span::styled(app.input.clone(), PLAIN),
            Span::styled("█", PLAIN),
        ]);
        f.render_widget(Paragraph::new(line), area);
        return;
    }

    if app.mode == Mode::Diff {
        let both = app.diff.as_ref().is_some_and(|v| v.both);
        let mut hints = vec![("↑↓", "scroll"), ("Space", "page"), ("[ ]", "hunk"), ("← →", "pan"), ("g G", "top/end")];
        if both {
            hints.push(("Tab", "staged/unstaged"));
        }
        hints.push(("Esc", "close"));
        let mut spans = Vec::new();
        for (key, what) in hints {
            spans.push(Span::styled(format!(" {key} "), hue(KEYCAP, ACCENT)));
            spans.push(Span::styled(format!(" {what}  "), DIM));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), area);
        return;
    }

    if app.mode == Mode::History {
        let hints = [("↑↓", "move"), ("Enter", "show the commit"), ("g G", "newest/oldest"), ("Esc", "close")];
        f.render_widget(Paragraph::new(Line::from(hint_spans(&hints))), area);
        return;
    }
    if app.mode == Mode::GitHub {
        let mut hints = vec![("↑↓", "move"), ("Enter", "read"), ("Tab", "issues/PRs"), ("s", "state"), ("o", "browser")];
        if app.gh.as_ref().is_some_and(|v| v.kind == Kind::Pulls) {
            hints.extend([("d", "diff"), ("c", "checkout")]);
        }
        hints.extend([("B", "repo in browser"), ("Esc", "close")]);
        f.render_widget(Paragraph::new(Line::from(hint_spans(&hints))), area);
        return;
    }
    if app.mode == Mode::Finder {
        let hints = [("↑↓", "move"), ("Enter", "clone"), ("/", "new search"), ("o", "browser"), ("r", "reload"), ("Esc", "close")];
        f.render_widget(Paragraph::new(Line::from(hint_spans(&hints))), area);
        return;
    }
    if app.mode == Mode::Text {
        let hints = [("↑↓", "scroll"), ("Space", "page"), ("g G", "top/end"), ("o", "browser"), ("Esc", "back")];
        f.render_widget(Paragraph::new(Line::from(hint_spans(&hints))), area);
        return;
    }
    if app.mode == Mode::Branches {
        let hints = [("↑↓", "move"), ("Enter", "switch"), ("n", "new"), ("d", "delete"), ("D", "force delete"), ("m", "merge"), ("Esc", "close")];
        f.render_widget(Paragraph::new(Line::from(hint_spans(&hints))), area);
        return;
    }

    if app.mode == Mode::Confirm {
        let prompt = app.confirm.as_ref().map_or("", |c| c.prompt.as_str());
        let line = Line::from(vec![Span::styled(format!(" {prompt} "), hue(BOLD, WARN)), Span::styled(" y ", hue(KEYCAP, WARN)), Span::styled(" yes   any other key cancels", DIM)]);
        f.render_widget(Paragraph::new(line), area);
        return;
    }

    // In order of importance; the last three are always kept, the rest only while they fit the width.
    let hints = [
        ("↑↓", "move"),
        (":", "palette"),
        ("Tab", "pane"),
        ("Space", "stage"),
        ("c", "commit"),
        ("d", "diff"),
        ("f", "filter"),
        ("s", "sort"),
        ("/", "search"),
        ("p", "pull"),
        ("P", "push"),
        ("F", "fetch"),
        ("m", "merge"),
        ("h", "hide keys"),
        ("?", "help"),
        ("q", "quit"),
    ];
    let width = |(key, what): &(&str, &str)| key.chars().count() + what.chars().count() + 6;
    let tail = hints.len() - 3;
    let reserved: usize = hints[tail..].iter().map(width).sum();
    let (mut used, mut shown) = (reserved, Vec::new());
    for (i, hint) in hints[..tail].iter().enumerate() {
        if used + width(hint) > area.width as usize {
            break;
        }
        used += width(hint);
        shown.push(i);
    }
    shown.extend(tail..hints.len());
    let spans = hint_spans(&shown.iter().map(|&i| hints[i]).collect::<Vec<_>>());
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Style of a highlighted token. With colors off only the weight differs: bold keywords and
/// dim italic comments.
fn tok_style(t: Tok) -> Style {
    match t {
        Tok::Plain => PLAIN,
        Tok::Keyword => hue(BOLD, PINK),
        Tok::Type => hue(PLAIN, WARN),
        Tok::Func | Tok::Key => hue(PLAIN, Color::LightBlue),
        Tok::Str => hue(PLAIN, GOOD),
        Tok::Number | Tok::Const => hue(PLAIN, Color::LightRed),
        Tok::Macro => hue(PLAIN, ACCENT),
        Tok::Comment if COLOR.load(Ordering::Relaxed) => ITALIC.fg(Color::DarkGray),
        Tok::Comment => DIM.add_modifier(Modifier::ITALIC),
    }
}

/// The code of a diff line as spans, with the first `skip` columns scrolled out of view.
fn code_spans(toks: &[(Tok, String)], mut skip: usize) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (tok, text) in toks {
        let n = text.chars().count();
        if skip >= n {
            skip -= n;
            continue;
        }
        spans.push(Span::styled(text.chars().skip(skip).collect::<String>(), tok_style(*tok)));
        skip = 0;
    }
    spans
}

fn diff_line(row: &Row, num_w: usize, hscroll: usize) -> Line<'static> {
    let blank = " ".repeat(num_w * 2 + 2);
    match row {
        Row::Meta(s) | Row::Note(s) => Line::from(Span::styled(format!("{blank}{s}"), DIM.add_modifier(Modifier::ITALIC))),
        Row::File(s) => Line::from(vec![Span::styled(format!("{} ", "━".repeat(num_w * 2 + 2)), DIM), Span::styled(s.clone(), hue(BOLD, WARN))]),
        Row::Hunk(s) => Line::from(Span::styled(format!("{blank}{s}"), hue(BOLD, ACCENT))),
        Row::Line { sign, old, new, toks } => {
            let num = |n: &Option<u32>| n.map_or_else(|| " ".repeat(num_w), |n| format!("{n:>num_w$}"));
            let (mark, style) = match sign {
                Sign::Add => (" + ", hue(PLAIN.add_modifier(Modifier::REVERSED).add_modifier(Modifier::BOLD), GOOD)),
                Sign::Del => (" - ", hue(PLAIN.add_modifier(Modifier::REVERSED).add_modifier(Modifier::BOLD), BAD)),
                Sign::Ctx => ("   ", PLAIN),
            };
            let mut spans = vec![Span::styled(format!("{} {} ", num(old), num(new)), DIM), Span::styled(mark, style), Span::raw(" ")];
            spans.extend(code_spans(toks, hscroll));
            Line::from(spans)
        }
    }
}

/// The diff viewer: a full-screen panel over the dashboard.
fn draw_diff(f: &mut Frame, app: &mut App) {
    let [area, _] = Layout::vertical([Constraint::Min(0), Constraint::Length(footer_height(app))]).areas(f.area());
    let Some(v) = app.diff.as_mut() else { return };
    let name = v.repo.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let title = if v.kind == DiffKind::Commit {
        format!(" Commit · {name} · {} ", v.file)
    } else if v.kind == DiffKind::Pull {
        format!(" Pull request · {} ", v.file)
    } else {
        format!(" Diff · {name}/{} · {} ", v.file, v.kind.label())
    };
    let mut block = pane(title, true);
    v.height = area.height.saturating_sub(2) as usize;
    f.render_widget(Clear, area);

    let note = |msg: String, block: Block<'static>, f: &mut Frame| {
        f.render_widget(Paragraph::new(msg).style(DIM.add_modifier(Modifier::ITALIC)).block(block), area);
    };
    if let Some(e) = &v.error {
        return note(format!("git diff failed: {}", e.lines().next().unwrap_or("")), block, f);
    }
    let Some(d) = &v.diff else {
        return note("Loading diff…".to_string(), block, f);
    };
    if d.rows.is_empty() {
        return note("No textual changes.".to_string(), block, f);
    }

    v.scroll = v.scroll.min(d.rows.len().saturating_sub(v.height));
    let last = (v.scroll + v.height).min(d.rows.len());
    block = block.title_top(
        Line::from(vec![
            Span::styled(format!(" +{}", d.added), hue(BOLD, GOOD)),
            Span::styled(format!(" −{}", d.removed), hue(BOLD, BAD)),
            Span::styled(format!(" · {}-{last}/{} ", v.scroll + 1, d.rows.len()), DIM),
        ])
        .right_aligned(),
    );
    let mut lines: Vec<Line> = d.rows[v.scroll..last].iter().map(|r| diff_line(r, d.num_w, v.hscroll)).collect();
    if d.truncated && last == d.rows.len() {
        lines.push(Line::from(Span::styled("… diff cut off after the first rows", DIM.add_modifier(Modifier::ITALIC))));
    }
    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// Footer key caps with their labels.
fn hint_spans(hints: &[(&str, &str)]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (key, what) in hints {
        spans.push(Span::styled(format!(" {key} "), hue(KEYCAP, ACCENT)));
        spans.push(Span::styled(format!(" {what}  "), DIM));
    }
    spans
}

/// The commit log: a full-screen list, newest commit first.
fn draw_history(f: &mut Frame, app: &mut App) {
    let [area, _] = Layout::vertical([Constraint::Min(0), Constraint::Length(footer_height(app))]).areas(f.area());
    let Some(v) = app.history.as_mut() else { return };
    f.render_widget(Clear, area);
    let count = v.commits.as_ref().map_or(String::new(), |c| format!(" ({})", c.len()));
    let block = pane(format!(" Log · {} · {}{count} ", v.name, v.branch), true);
    let note = |msg: String, f: &mut Frame, block: Block<'static>| {
        f.render_widget(Paragraph::new(msg).style(DIM.add_modifier(Modifier::ITALIC)).block(block), area);
    };
    if let Some(e) = &v.error {
        return note(format!("git log failed: {}", e.lines().next().unwrap_or("")), f, block);
    }
    let Some(commits) = &v.commits else { return note("Loading commits…".to_string(), f, block) };
    if commits.is_empty() {
        return note("No commits yet.".to_string(), f, block);
    }
    let items: Vec<ListItem> = commits
        .iter()
        .map(|c| {
            let mut spans = vec![
                Span::styled(format!("{:<8}", c.hash), hue(DIM, WARN)),
                Span::styled(fit(&c.date, 15), DIM),
                Span::styled(fit(&c.author, 16), hue(PLAIN, INFO)),
                Span::raw(" "),
            ];
            if !c.refs.is_empty() {
                spans.push(Span::styled(format!("({}) ", c.refs), hue(BOLD, ACCENT)));
            }
            spans.push(Span::styled(c.subject.clone(), PLAIN));
            ListItem::new(Line::from(spans))
        })
        .collect();
    f.render_stateful_widget(List::new(items).block(block).highlight_style(INVERSE), area, &mut v.state);
}

/// Colored label for a branch, if any: merged and stale ones are candidates for cleanup.
fn branch_tags(b: &Branch) -> Vec<Span<'static>> {
    let mut tags = Vec::new();
    if b.merged && !b.current {
        tags.push(Span::styled("merged ", hue(PLAIN, GOOD)));
    }
    if b.stale {
        tags.push(Span::styled("stale ", hue(PLAIN, WARN)));
    }
    tags
}

/// Issues or pull requests of a repository on GitHub, as a full-screen list.
fn draw_github(f: &mut Frame, app: &mut App) {
    let [area, _] = Layout::vertical([Constraint::Min(0), Constraint::Length(footer_height(app))]).areas(f.area());
    let Some(v) = app.gh.as_mut() else { return };
    f.render_widget(Clear, area);
    let count = v.items.as_ref().map_or(String::new(), |i| format!(" ({})", i.len()));
    let block = pane(format!(" {} · {} · {}{count} ", v.kind.label(), v.name, v.state.name()), true);
    let note = |msg: String, f: &mut Frame, block: Block<'static>| {
        f.render_widget(Paragraph::new(msg).style(DIM.add_modifier(Modifier::ITALIC)).block(block), area);
    };
    if let Some(e) = &v.error {
        return note(format!("gh failed: {}", e.lines().next().unwrap_or("")), f, block);
    }
    let Some(items) = &v.items else { return note("Loading from GitHub…".to_string(), f, block) };
    if items.is_empty() {
        return note(format!("No {} {}.", v.state.name(), v.kind.label().to_lowercase()), f, block);
    }
    let pulls = v.kind == Kind::Pulls;
    // Fixed columns: number, branch or labels, author, age; the title takes what is left.
    let side_w = if pulls { 24 } else { 22 };
    let title_w = (area.width as usize).saturating_sub(4 + 8 + 2 + side_w + 15 + 5).max(12);
    let rows: Vec<ListItem> = items
        .iter()
        .map(|i| {
            let (mark, mark_style) = match (i.draft, i.review.as_str()) {
                (true, _) => ("◌ ", DIM),
                (_, "APPROVED") => ("✓ ", hue(BOLD, GOOD)),
                (_, "CHANGES_REQUESTED") => ("± ", hue(BOLD, BAD)),
                _ => ("  ", PLAIN),
            };
            let side = if pulls { i.branch.as_str() } else { i.labels.as_str() };
            let mut spans = vec![Span::styled(format!("#{:<7}", i.number), hue(DIM, WARN))];
            if pulls {
                spans.push(Span::styled(mark, mark_style));
            }
            spans.push(Span::styled(fit(&i.title, title_w), PLAIN));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(fit(side, side_w), hue(DIM, ACCENT)));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(fit(&i.author, 14), hue(PLAIN, INFO)));
            spans.push(Span::styled(format!("{:>4}", i.age), DIM));
            ListItem::new(Line::from(spans))
        })
        .collect();
    f.render_stateful_widget(List::new(rows).block(block).highlight_style(INVERSE), area, &mut v.list);
}

/// GitHub repository search results (or your own repositories) as a popup.
fn draw_finder(f: &mut Frame, app: &mut App) {
    let cloned: Vec<String> = app.repos.iter().filter_map(|r| r.slug.as_ref().map(|s| s.to_lowercase())).collect();
    let last = app.log.last().map(|l| l.text.clone()).unwrap_or_default();
    let Some(v) = app.finder.as_mut() else { return };
    let rows = v.items.as_ref().map_or(1, Vec::len) as u16;
    let area = f.area();
    let w = 120.min(area.width);
    let h = (rows + 2).clamp(5, area.height.saturating_sub(2).max(5)).min(area.height);
    let [_, mid, _] = Layout::vertical([Constraint::Fill(1), Constraint::Length(h), Constraint::Fill(1)]).areas(area);
    let [_, popup, _] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(w), Constraint::Fill(1)]).areas(mid);
    f.render_widget(Clear, popup);
    let what = if v.query.is_empty() { "your repositories".to_string() } else { format!("“{}”", v.query) };
    let count = v.items.as_ref().map_or(String::new(), |i| format!(" ({})", i.len()));
    let block = pane(format!(" GitHub · {what}{count} "), true).title_bottom(Line::from(Span::styled(format!(" {last} "), DIM)));
    let note = |msg: String, f: &mut Frame, block: Block<'static>| {
        f.render_widget(Paragraph::new(msg).style(DIM.add_modifier(Modifier::ITALIC)).block(block), popup);
    };
    if let Some(e) = &v.error {
        return note(format!("gh failed: {}", e.lines().next().unwrap_or("")), f, block);
    }
    let Some(items) = &v.items else { return note("Searching GitHub…".to_string(), f, block) };
    if items.is_empty() {
        return note("No repositories found.".to_string(), f, block);
    }
    let desc_w = (popup.width as usize).saturating_sub(4 + 38 + 7 + 11 + 5 + 8).max(8);
    let rows: Vec<ListItem> = items
        .iter()
        .map(|i| {
            let mut tags = String::new();
            if i.private {
                tags.push_str("private ");
            }
            if i.fork {
                tags.push_str("fork ");
            }
            let have = cloned.contains(&i.slug.to_lowercase());
            let stars = i.stars.map_or_else(String::new, |s| format!("★{s}"));
            ListItem::new(Line::from(vec![
                Span::styled(fit(&i.slug, 38), if have { hue(BOLD, GOOD) } else { BOLD }),
                Span::styled(format!("{stars:>7} "), hue(DIM, WARN)),
                Span::styled(fit(&i.language, 10), hue(PLAIN, ACCENT)),
                Span::styled(format!("{:>4} ", i.age), DIM),
                Span::styled(if have { "cloned " } else { "       " }, hue(PLAIN, GOOD)),
                Span::styled(fit(&format!("{tags}{}", i.description), desc_w), DIM),
            ]))
        })
        .collect();
    f.render_stateful_widget(List::new(rows).block(block).highlight_style(INVERSE), popup, &mut v.list);
}

/// An issue or pull request as plain scrollable text.
fn draw_text(f: &mut Frame, app: &mut App) {
    let [area, _] = Layout::vertical([Constraint::Min(0), Constraint::Length(footer_height(app))]).areas(f.area());
    let Some(v) = app.text.as_mut() else { return };
    f.render_widget(Clear, area);
    v.height = area.height.saturating_sub(2) as usize;
    let block = pane(format!(" {} ", v.title), true);
    let note = |msg: String, f: &mut Frame, block: Block<'static>| {
        f.render_widget(Paragraph::new(msg).style(DIM.add_modifier(Modifier::ITALIC)).block(block), area);
    };
    if let Some(e) = &v.error {
        return note(format!("gh failed: {}", e.lines().next().unwrap_or("")), f, block);
    }
    let Some(all) = &v.lines else { return note("Loading from GitHub…".to_string(), f, block) };
    v.scroll = v.scroll.min(all.len().saturating_sub(v.height));
    let last = (v.scroll + v.height).min(all.len());
    let block = block.title_top(Line::from(Span::styled(format!(" {}-{last}/{} ", (v.scroll + 1).min(last), all.len()), DIM)).right_aligned());
    // The first line is gh's headline; metadata lines ("Open • user opened ...") follow it.
    let lines: Vec<Line> = all[v.scroll..last]
        .iter()
        .enumerate()
        .map(|(k, l)| {
            let style = if v.scroll + k == 0 { hue(BOLD, ACCENT) } else if l.starts_with("--") || l.starts_with("──") { DIM } else { PLAIN };
            Line::from(Span::styled(l.clone(), style))
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// The branch list as a popup, with the newest log message below it so the result of an action is visible.
fn draw_branches(f: &mut Frame, app: &mut App) {
    let last = app.log.last().map(|l| l.text.clone()).unwrap_or_default();
    let Some(v) = app.branches.as_mut() else { return };
    let rows = v.branches.as_ref().map_or(1, Vec::len) as u16;
    let area = f.area();
    let w = 96.min(area.width);
    let h = (rows + 2).clamp(5, area.height.saturating_sub(2).max(5)).min(area.height);
    let [_, mid, _] = Layout::vertical([Constraint::Fill(1), Constraint::Length(h), Constraint::Fill(1)]).areas(area);
    let [_, popup, _] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(w), Constraint::Fill(1)]).areas(mid);
    f.render_widget(Clear, popup);
    let block = pane(format!(" Branches · {} ", v.name), true).title_bottom(Line::from(Span::styled(format!(" {last} "), DIM)));

    if let Some(e) = &v.error {
        let msg = format!("git branch failed: {}", e.lines().next().unwrap_or(""));
        return f.render_widget(Paragraph::new(msg).style(DIM.add_modifier(Modifier::ITALIC)).block(block), popup);
    }
    let Some(list) = &v.branches else {
        return f.render_widget(Paragraph::new("Loading branches…").style(DIM.add_modifier(Modifier::ITALIC)).block(block), popup);
    };
    let items: Vec<ListItem> = list
        .iter()
        .map(|b| {
            let name_style = if b.current { hue(BOLD, GOOD) } else if b.remote { DIM } else { PLAIN };
            let mut spans = vec![
                Span::styled(if b.current { "● " } else { "  " }, hue(BOLD, GOOD)),
                Span::styled(fit(&b.name, 30), name_style),
                Span::styled(fit(&b.upstream, 26), hue(DIM, ACCENT)),
            ];
            spans.extend(branch_tags(b));
            spans.push(Span::styled(b.date.clone(), DIM));
            ListItem::new(Line::from(spans))
        })
        .collect();
    f.render_stateful_widget(List::new(items).block(block).highlight_style(INVERSE), popup, &mut v.state);
}

/// The command palette: a query line over the matching actions, with the matched letters emphasised.
fn draw_palette(f: &mut Frame, app: &mut App) {
    let found = palette::matches(&app.palette.query);
    let area = f.area();
    let w = 70.min(area.width);
    let h = (found.len().max(1) as u16 + 3).min(area.height.saturating_sub(2).max(5)).min(area.height);
    let [_, mid, _] = Layout::vertical([Constraint::Percentage(20), Constraint::Length(h), Constraint::Fill(1)]).areas(area);
    let [_, popup, _] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(w), Constraint::Fill(1)]).areas(mid);
    f.render_widget(Clear, popup);
    let block = Block::bordered()
        .border_type(BorderType::Double)
        .padding(Padding::horizontal(SIDE_PAD))
        .border_style(hue(BOLD, ACCENT))
        .title(Line::from(vec![Span::styled("═╡ ", hue(BOLD, ACCENT)), Span::styled("◆ OXIDE commands", hue(BOLD, ACCENT)), Span::styled(" ╞", hue(BOLD, ACCENT))]));
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    let [query_row, list_area] = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(inner);

    let query = Line::from(vec![Span::styled("› ", hue(BOLD, ACCENT)), Span::styled(app.palette.query.clone(), BOLD), Span::styled("█", PLAIN)]);
    f.render_widget(Paragraph::new(query), query_row);

    if found.is_empty() {
        let msg = if app.palette.query.is_empty() { "Try typing..." } else { "No command matches." };
        f.render_widget(Paragraph::new(msg).style(DIM.add_modifier(Modifier::ITALIC)), list_area);
        return;
    }
    let key_w = 6;
    let name_w = (list_area.width as usize).saturating_sub(key_w + 1);
    let items: Vec<ListItem> = found
        .iter()
        .map(|(i, at)| {
            let entry = &palette::ENTRIES[*i];
            let mut spans: Vec<Span> = entry
                .name
                .chars()
                .take(name_w)
                .enumerate()
                .map(|(k, c)| Span::styled(c.to_string(), if at.contains(&k) { hue(BOLD, WARN) } else { PLAIN }))
                .collect();
            let used = spans.len();
            spans.push(Span::raw(" ".repeat((name_w + 1).saturating_sub(used))));
            spans.push(Span::styled(format!("{:>key_w$}", entry.key), hue(DIM, ACCENT)));
            ListItem::new(Line::from(spans))
        })
        .collect();
    f.render_stateful_widget(List::new(items).highlight_style(INVERSE), list_area, &mut app.palette.state);
}

fn draw_help(f: &mut Frame) {
    let rows: &[(&str, &str)] = &[
        ("↑ ↓ / j k, g G", "move selection, first / last"),
        ("Tab, ← →", "switch between repositories and changes"),
        ("f, 1 2 3 4", "filter: all / owned / cloned / attention"),
        ("s", "sort: name / recent / changes / behind"),
        ("/", "search by name, branch or path (Esc clears)"),
        ("Space, a, u", "stage / unstage the file, stage all, unstage all"),
        ("c", "commit staged changes"),
        ("p / P", "pull (fast-forward only) / push (new branches with -u)"),
        ("F / A", "fetch selected repository / every one in view"),
        ("L / U", "pull every behind / push every ahead repository in view"),
        ("m", "merge a branch (empty = upstream)"),
        ("z / Z", "stash all changes / pop the latest stash"),
        ("d v b I", "diff of the file / commit log / branches / git init in any folder"),
        ("i y B S", "GitHub: issues / pull requests / repo in browser / find & clone"),
        ("e / t / o", "yazi on the file or repository / shell / file manager"),
        ("* / x / H", "pin / ignore the repository / show ignored ones"),
        ("r / R", "reload selected repository / rescan all roots"),
        (": / Ctrl+P", "command palette: type to find any action, Enter runs it"),
        ("h / C", "hide or show this key bar / toggle colors"),
        ("? / q", "this help (any key closes it) / quit"),
        ("↑n ↓n ★ ⊘", "ahead · behind · pinned · ignored"),
        ("+ ~ ? ! $", "staged · modified · untracked · conflicts · stashes"),
    ];
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, d)| Line::from(vec![Span::styled(format!("{k:<16}"), hue(BOLD, ACCENT)), Span::styled(*d, PLAIN)]))
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
        .border_style(hue(BOLD, ACCENT))
        .title(Line::from(vec![
            Span::styled("═╡ ", hue(BOLD, ACCENT)),
            Span::styled("◆ OXIDE keys", hue(BOLD, ACCENT)),
            Span::styled(" ╞", hue(BOLD, ACCENT)),
        ]));
    f.render_widget(Paragraph::new(lines).block(block), popup);
}
