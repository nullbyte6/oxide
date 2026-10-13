//! Rendering of the dashboard with Ratatui.
//!
//! The design is monochrome at its core: one ink color on the terminal's own background. Hierarchy
//! and state come from bold, dim and inverse text, thick borders and symbols. Colors are an optional
//! layer on top (`hue`): every colored span keeps the weight and symbol it has without color, so
//! turning colors off (`C`, `color = false` in config.lua or `NO_COLOR`) leaves the monochrome look unchanged.

mod flows;

use std::sync::{
    OnceLock,
    atomic::{AtomicBool, Ordering},
};

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, List, ListItem, Padding, Paragraph, Tabs},
};

use crate::{
    app::{App, Choice, Filter, Focus, InputKind, Mode, ProjectStep},
    banner,
    diff::{Row, Sign},
    git::{Branch, DiffKind, FileEntry, TreeKind},
    github::{self, Ci, Hits, Kind, Scope},
    langs::Breakdown,
    license::LICENSES,
    logger::{self, Level},
    palette,
    repo::{Health, Owner, Repo},
    scaffold::{PackageKind, langs},
    syntax::Tok,
    term::Term,
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

/// Empty rows between the banner and the header, so the logo has room to breathe.
const BANNER_GAP: u16 = 1;

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
    if banner_h > 0 && f.area().height >= banner_h + BANNER_GAP + MIN_DASHBOARD_ROWS && f.area().width >= banner.width {
        let [banner_area, rest] = Layout::vertical([Constraint::Length(banner_h), Constraint::Min(0)]).spacing(BANNER_GAP).areas(area);
        // Center the art as one block so its lines stay aligned with each other.
        let [_, art, _] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(banner.width), Constraint::Fill(1)]).areas(banner_area);
        f.render_widget(Paragraph::new(banner.lines.clone()).style(hue(BOLD, ACCENT)), art);
        area = rest;
    }

    let [header, body, log] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(8), Constraint::Length(if app.logs { 8 } else { 0 })]).spacing(GAP_Y).areas(area);
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(54), Constraint::Fill(1)]).spacing(GAP_X).areas(body);

    draw_header(f, app, header);
    draw_repos(f, app, left);
    draw_details(f, app, right);
    if app.logs {
        draw_log(f, app, log);
    }
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
        Mode::Reflog => draw_reflog(f, app),
        Mode::Trees => draw_trees(f, app),
        Mode::Remotes => draw_remotes(f, app),
        Mode::GitHub => draw_github(f, app),
        Mode::Text => draw_text(f, app),
        Mode::Finder => draw_finder(f, app),
        Mode::Project => draw_project(f, app),
        Mode::Tags => draw_tags(f, app),
        Mode::Workflows => flows::draw_workflows(f, app),
        Mode::WorkflowRun => {
            flows::draw_workflows(f, app);
            flows::draw_run_form(f, app);
        }
        Mode::WorkflowNew => {
            flows::draw_workflows(f, app);
            flows::draw_wizard(f, app);
        }
        _ => {}
    }
    if app.mode == Mode::Terminal
        && let Some(t) = app.term.and_then(|i| app.terms.get_mut(i))
    {
        draw_terminal(f, t);
    }
    if app.mode == Mode::Palette {
        draw_palette(f, app);
    }
    if app.mode == Mode::Help {
        draw_help(f);
    }
}

/// The new-project menu: a centered popup whose content depends on the page.
fn draw_project(f: &mut Frame, app: &mut App) {
    let Some(v) = app.project.as_mut() else { return };
    let name = v.repo.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let page = match v.step {
        ProjectStep::Lang => "language",
        ProjectStep::Package => langs()[v.lang].package.map_or("package", |p| p.kind.noun()),
        ProjectStep::Variant => "variant",
        ProjectStep::License => "license",
        ProjectStep::Confirm => "confirm",
    };
    let area = f.area();
    let w = 80.min(area.width);
    // Rows of the page's list plus the lines around it (borders, filter or holder line).
    let (rows, extra) = match v.step {
        ProjectStep::Lang => (v.choices().len(), 4),
        ProjectStep::Package => (3, 4),
        ProjectStep::Variant => (langs()[v.lang].variants.len(), 3),
        ProjectStep::License => (LICENSES.len() + 1, 4),
        // The plan, a line for programs that are missing and the lines around them.
        ProjectStep::Confirm => (v.plan.as_ref().map_or(0, |p| p.actions.len() + usize::from(!p.missing.is_empty())), 7),
    };
    let h = (rows as u16 + extra).clamp(6, area.height.saturating_sub(2).max(6)).min(area.height);
    let [_, mid, _] = Layout::vertical([Constraint::Fill(1), Constraint::Length(h), Constraint::Fill(1)]).areas(area);
    let [_, popup, _] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(w), Constraint::Fill(1)]).areas(mid);
    f.render_widget(Clear, popup);
    let block = pane(format!(" New project · {name} · {page} "), true);
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    match v.step {
        ProjectStep::Lang => {
            let [filter, list] = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(inner);
            let found = v.choices();
            let count = format!("{}/{}", found.iter().filter(|c| matches!(c, Choice::Lang(_))).count(), langs().len());
            let line = Line::from(vec![
                Span::styled("Filter ▸ ", hue(BOLD, ACCENT)),
                Span::styled(v.query.clone(), BOLD),
                Span::styled("█", PLAIN),
                Span::styled(format!("  {count}"), DIM),
            ]);
            f.render_widget(Paragraph::new(line), filter);
            if found.iter().all(|c| *c == Choice::Other) && !v.query.is_empty() {
                f.render_widget(Paragraph::new("No language matches.").style(DIM.add_modifier(Modifier::ITALIC)), list);
            }
            let note_w = (list.width as usize).saturating_sub(17);
            let items: Vec<ListItem> = found
                .iter()
                .map(|c| match c {
                    Choice::Lang(i) => {
                        let l = &langs()[*i];
                        ListItem::new(Line::from(vec![Span::styled(fit(l.name, 16), BOLD), Span::raw(" "), Span::styled(fit(l.variants[0].note, note_w), DIM)]))
                    }
                    Choice::Other => ListItem::new(Line::from(vec![
                        Span::styled(fit("Other / manual…", 16), hue(BOLD, ACCENT)),
                        Span::raw(" "),
                        Span::styled(fit("open a terminal here and set it up yourself", note_w), DIM),
                    ])),
                })
                .collect();
            f.render_stateful_widget(List::new(items).highlight_style(INVERSE), list, &mut v.lang_state);
        }
        ProjectStep::Package => {
            let kind = langs()[v.lang].package.map_or(PackageKind::Packages, |p| p.kind);
            let mut lines = vec![
                Line::from(Span::styled(kind.help(), DIM)),
                Line::raw(""),
                Line::from(vec![Span::styled(format!("{} ▸ ", kind.noun().chars().next().map_or_else(String::new, |c| c.to_uppercase().chain(kind.noun().chars().skip(1)).collect())), hue(BOLD, ACCENT)), Span::styled(v.package.clone(), BOLD), Span::styled("█", PLAIN)]),
            ];
            if let Some(e) = &v.package_error {
                lines.push(Line::from(Span::styled(fit(e, inner.width as usize), hue(BOLD, BAD))));
            }
            f.render_widget(Paragraph::new(lines), inner);
        }
        ProjectStep::Variant => {
            let lang = &langs()[v.lang];
            let note_w = (inner.width as usize).saturating_sub(24);
            let items: Vec<ListItem> = lang
                .variants
                .iter()
                .enumerate()
                .map(|(i, var)| {
                    let default = if i == 0 { " (default)" } else { "" };
                    ListItem::new(Line::from(vec![
                        Span::styled(fit(&format!("{}{default}", var.name), 23), BOLD),
                        Span::raw(" "),
                        Span::styled(fit(var.note, note_w), DIM),
                    ]))
                })
                .collect();
            f.render_stateful_widget(List::new(items).highlight_style(INVERSE), inner, &mut v.variant_state);
        }
        ProjectStep::License => {
            let [list, holder] = Layout::vertical([Constraint::Min(1), Constraint::Length(2)]).areas(inner);
            let name_w = (list.width as usize).saturating_sub(15);
            let mut items: Vec<ListItem> = LICENSES
                .iter()
                .map(|l| ListItem::new(Line::from(vec![Span::styled(fit(l.id, 14), BOLD), Span::raw(" "), Span::styled(fit(l.name, name_w), DIM)])))
                .collect();
            items.push(ListItem::new(Line::from(vec![Span::styled(fit("None", 14), BOLD), Span::raw(" "), Span::styled(fit("no LICENSE file", name_w), DIM)])));
            f.render_stateful_widget(List::new(items).highlight_style(INVERSE), list, &mut v.license_state);

            let mut spans = vec![Span::styled("Holder ▸ ", hue(BOLD, ACCENT)), Span::styled(v.holder.clone(), if v.editing { BOLD } else { PLAIN })];
            spans.push(if v.editing { Span::styled("█", PLAIN) } else { Span::styled("  (Tab edits)", DIM) });
            spans.push(Span::styled(format!("    Year ▸ {}", v.year), DIM));
            f.render_widget(Paragraph::new(vec![Line::raw(""), Line::from(spans)]), holder);
        }
        ProjectStep::Confirm => {
            let Some(plan) = &v.plan else { return };
            let lang = &langs()[v.lang];
            let variant = lang.variants[v.variant_state.selected().unwrap_or(0)].name;
            let license = v.license().map_or("no license", |l| l.id);
            let packages = if v.packages.is_empty() { String::new() } else { format!(" · {}", v.packages.join(", ")) };
            let mut lines = vec![
                Line::from(Span::styled(fit(&format!("{} · {variant}{packages} · {license}", lang.name), inner.width as usize), BOLD)),
                Line::from(Span::styled(fit(&format!("in {}", v.repo.display()), inner.width as usize), DIM)),
                Line::raw(""),
            ];
            // Leave room for the rows below the plan: a blank line and the commit option.
            let room = (inner.height as usize).saturating_sub(lines.len() + 2 + usize::from(!plan.missing.is_empty()));
            for a in plan.actions.iter().take(room) {
                let label = a.label();
                let style = if label.starts_with('$') { hue(PLAIN, ACCENT) } else { PLAIN };
                lines.push(Line::from(Span::styled(fit(&label, inner.width as usize), style)));
            }
            if plan.actions.len() > room {
                lines.push(Line::from(Span::styled(format!("… and {} more", plan.actions.len() - room), DIM)));
            }
            if !plan.missing.is_empty() {
                lines.push(Line::from(Span::styled(format!("! not installed, skipped: {}", plan.missing.join(", ")), hue(BOLD, WARN))));
            }
            lines.push(Line::raw(""));
            let (mark, style) = if v.commit { ("[x]", hue(BOLD, GOOD)) } else { ("[ ]", PLAIN) };
            lines.push(Line::from(vec![Span::styled(mark, style), Span::styled(" Add initial commit and projects' initial files", PLAIN)]));
            f.render_widget(Paragraph::new(lines), inner);
        }
    }
}

/// The embedded terminal, filling the whole screen: the shell's screen is copied cell by cell into the
/// frame, in the colors the shell asked for. The pseudo-terminal is resized to the room inside the border.
fn draw_terminal(f: &mut Frame, term: &mut Term) {
    let area = f.area();
    f.render_widget(Clear, area);
    let mut name = term.repo.file_name().map_or_else(|| term.repo.display().to_string(), |n| n.to_string_lossy().into_owned());
    // A shell in a folder of the repository says which one.
    if let Ok(rel) = term.dir.strip_prefix(&term.repo)
        && !rel.as_os_str().is_empty()
    {
        name = format!("{name}/{}", rel.display());
    }
    let hint = " Ctrl+T hide · Shift+PgUp/PgDn scroll · exit closes ";
    let block = pane(format!(" Terminal · {name} "), true).padding(Padding::ZERO).title_bottom(Line::from(Span::styled(hint, DIM)));
    let inner = block.inner(area);
    f.render_widget(block, area);
    term.resize(inner.height, inner.width);

    let buf = f.buffer_mut();
    let cursor = term.with_screen(|screen| {
        for row in 0..inner.height {
            for col in 0..inner.width {
                let Some(cell) = screen.cell(row, col) else { continue };
                if cell.is_wide_continuation() {
                    continue; // drawn together with the wide character before it
                }
                let mut style = Style::new().fg(term_color(cell.fgcolor())).bg(term_color(cell.bgcolor()));
                for (on, m) in [
                    (cell.bold(), Modifier::BOLD),
                    (cell.dim(), Modifier::DIM),
                    (cell.italic(), Modifier::ITALIC),
                    (cell.underline(), Modifier::UNDERLINED),
                    (cell.inverse(), Modifier::REVERSED),
                ] {
                    if on {
                        style = style.add_modifier(m);
                    }
                }
                let text = if cell.has_contents() { cell.contents() } else { " " };
                buf[(inner.x + col, inner.y + row)].set_symbol(text).set_style(style);
            }
        }
        // The cursor is only shown at the bottom of the history, where the shell is actually typing.
        (screen.scrollback() == 0 && !screen.hide_cursor()).then(|| screen.cursor_position())
    });
    if let Some((row, col)) = cursor {
        f.set_cursor_position((inner.x + col, inner.y + row));
    }
}

/// A color of the shell's screen as a Ratatui color.
fn term_color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
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

/// `30s`, `10m` or `2h` for a number of seconds.
fn interval(secs: u64) -> String {
    match secs {
        s if s % 3600 == 0 => format!("{}h", s / 3600),
        s if s % 60 == 0 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
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
    if app.cfg.autofetch > 0 {
        right.push(Span::styled(format!(" ⟳ {} ", interval(app.cfg.autofetch)), DIM));
    }
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

fn repo_row(r: &Repo, busy: Option<&str>, ci: Option<Ci>, tick: usize, name_w: usize) -> ListItem<'static> {
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
        ci_span(ci, tick),
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

/// The two-column CI slot of a repository row: ✓ passing, ✗ failing, a spinner while running, ○ inconclusive.
fn ci_span(ci: Option<Ci>, tick: usize) -> Span<'static> {
    match ci {
        Some(Ci::Passing) => Span::styled("✓ ", hue(PLAIN, GOOD)),
        Some(Ci::Failing) => Span::styled("✗ ", hue(BOLD, BAD)),
        Some(Ci::Running) => Span::styled(format!("{} ", SPINNER[tick % SPINNER.len()]), hue(PLAIN, PINK)),
        Some(Ci::Other) => Span::styled("○ ", DIM),
        None => Span::raw("  "),
    }
}

/// CI state of `r` when it was loaded for the branch that is checked out now.
fn ci_of(app: &App, r: &Repo) -> Option<Ci> {
    app.ci.get(&r.path).filter(|(branch, _)| *branch == r.branch).map(|(_, state)| *state)
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
    let name_w = (area.width as usize).saturating_sub(54).clamp(12, 28);
    let items: Vec<ListItem> = app
        .visible
        .iter()
        .map(|&i| {
            let r = &app.repos[i];
            repo_row(r, app.busy.get(&r.path).copied(), ci_of(app, r), app.tick, name_w)
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

/// A language's own colour: true colour where the terminal has it (`COLORTERM`), else the nearest of the
/// 256-colour palette's 6×6×6 cube.
fn lang_color(rgb: u32) -> Color {
    static TRUECOLOR: OnceLock<bool> = OnceLock::new();
    let truecolor = *TRUECOLOR.get_or_init(|| std::env::var("COLORTERM").is_ok_and(|v| v.contains("truecolor") || v.contains("24bit")));
    let (r, g, b) = ((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
    if truecolor {
        return Color::Rgb(r, g, b);
    }
    let q = |v: u8| (u16::from(v) * 5 + 127) / 255;
    Color::Indexed((16 + 36 * q(r) + 6 * q(g) + q(b)) as u8)
}

/// Fill glyphs that tell languages apart by rank when colors are off.
const SHADES: [&str; 6] = ["█", "▓", "▒", "░", "▚", "▞"];

/// The glyph and style a language's bar and swatch are drawn with: its colour, or a shade by rank without colors.
fn lang_mark(rank: usize, rgb: u32) -> (&'static str, Style) {
    if COLOR.load(Ordering::Relaxed) { ("█", PLAIN.fg(lang_color(rgb))) } else { (SHADES[rank % SHADES.len()], PLAIN) }
}

/// `82%`, or `<1%` for the tiny shares.
fn percent(p: f64) -> String {
    if p < 1.0 { "<1%".to_string() } else { format!("{p:.0}%") }
}

/// `12.3 kB` style size.
fn human(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} kB", b as f64 / 1024.0),
        b => format!("{:.1} MB", b as f64 / 1048576.0),
    }
}

/// One line for the details pane: a stacked bar of `cells` cells, one segment per language in its colour
/// (or shade). The names and shares are in the Languages pane.
fn langs_line(entry: Option<&(String, Result<Breakdown, String>)>, cells: usize) -> Vec<Span<'static>> {
    let note = |msg: &str| vec![Span::styled(msg.to_string(), DIM.add_modifier(Modifier::ITALIC))];
    let bd = match entry {
        None => return note("counting…"),
        Some((_, Err(_))) => return note("no commits yet"),
        Some((_, Ok(bd))) => bd,
    };
    if bd.langs.is_empty() {
        return note("none recognised");
    }
    // Each language gets the cells between its cumulative boundaries, so the bar always adds up to `cells`.
    let mut spans = Vec::new();
    let (mut cum, mut drawn) = (0u64, 0usize);
    for (rank, st) in bd.langs.iter().enumerate() {
        cum += st.bytes;
        let end = (cum as f64 * cells as f64 / bd.total as f64).round() as usize;
        if end > drawn {
            let (glyph, style) = lang_mark(rank, st.color);
            spans.push(Span::styled(glyph.repeat(end - drawn), style));
            drawn = end;
        }
    }
    spans
}

/// The Languages pane when it has focus: one row per language with its swatch, a bar, share, size and file count.
fn draw_langs(f: &mut Frame, app: &mut App, area: Rect, path: &std::path::Path) {
    let entry = app.langs.get(path);
    let count = match entry {
        Some((_, Ok(bd))) => format!(" ({})", bd.langs.len()),
        _ => String::new(),
    };
    let block = pane(format!(" Languages{count} "), true);
    let note = |msg: &str, f: &mut Frame, block: Block<'static>| {
        f.render_widget(Paragraph::new(msg.to_string()).style(DIM.add_modifier(Modifier::ITALIC)).block(block), area);
    };
    let bd = match entry {
        None => return note("Counting…", f, block),
        Some((_, Err(e))) => return note(&format!("No languages: {}", e.lines().next().unwrap_or("")), f, block),
        Some((_, Ok(bd))) => bd,
    };
    if bd.langs.is_empty() {
        return note("No programming or markup languages recognised.", f, block);
    }
    // Fixed columns: swatch, name, share, size, files; the bar takes what is left (up to 24 cells).
    let fixed = 2 + 16 + 1 + 5 + 10 + 9;
    let bar_w = (area.width as usize).saturating_sub(4 + fixed).min(24);
    let rows: Vec<ListItem> = bd
        .langs
        .iter()
        .enumerate()
        .map(|(rank, st)| {
            let p = bd.percent(st);
            let (glyph, style) = lang_mark(rank, st.color);
            let mut spans = vec![Span::styled(format!("{glyph} "), style), Span::styled(fit(st.name, 16), BOLD), Span::raw(" ")];
            if bar_w >= 4 {
                let filled = ((p * bar_w as f64 / 100.0).round() as usize).clamp(1, bar_w);
                spans.push(Span::styled(glyph.repeat(filled), style));
                spans.push(Span::styled("░".repeat(bar_w - filled), DIM));
            }
            spans.push(Span::styled(format!("{:>5}", percent(p)), PLAIN));
            spans.push(Span::styled(format!("{:>10}", human(st.bytes)), DIM));
            spans.push(Span::styled(format!("{:>6} f", st.files), DIM));
            ListItem::new(Line::from(spans))
        })
        .collect();
    let list = List::new(rows).block(block).highlight_style(INVERSE);
    f.render_stateful_widget(list, area, &mut app.lang_state);
}

fn draw_details(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Files;
    let Some(r) = app.selected().cloned() else {
        f.render_widget(Paragraph::new("Select a repository.").style(DIM.add_modifier(Modifier::ITALIC)).block(pane(" Details ".into(), false)), area);
        return;
    };

    // With the Languages pane focused it grows to fill the panel and the Changes list shrinks to a few rows.
    let langs_focus = app.focus == Focus::Langs;
    let (info_area, langs_area, files_area) = if langs_focus {
        let [i, l, f] = Layout::vertical([Constraint::Length(10), Constraint::Min(4), Constraint::Length(4)]).spacing(GAP_Y).areas(area);
        (i, Some(l), f)
    } else {
        let [i, f] = Layout::vertical([Constraint::Length(10), Constraint::Min(3)]).spacing(GAP_Y).areas(area);
        (i, None, f)
    };

    let label = |s: &'static str| Span::styled(format!("{s:<9}"), DIM);
    let ci = ci_of(app, &r);
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
            Span::raw(if ci.is_some() { "  ci " } else { "" }),
            ci.map_or_else(|| Span::raw(""), |c| Span::styled(format!("{} ", c.label()), DIM)),
            ci_span(ci, app.tick),
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
    } else {
        // The bar takes the rest of the line after the label (borders and padding take 4 columns).
        let cells = (info_area.width as usize).saturating_sub(4 + 9).clamp(8, 60);
        let mut spans = vec![label("langs")];
        spans.extend(langs_line(app.langs.get(&r.path), cells));
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines).block(pane(format!(" {} ", r.name), false)), info_area);

    if let Some(langs_area) = langs_area {
        draw_langs(f, app, langs_area, &r.path);
    }
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

/// The bottom row shows the key bar unless `h` hid it. Prompts (text input, confirmations) always need it,
/// and so does a message that is shown for a moment while the log panel is hidden.
fn footer_height(app: &App) -> u16 {
    u16::from(app.hints || app.toast().is_some() || matches!(app.mode, Mode::Input(_) | Mode::Confirm))
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    // With the log panel hidden, a new log message takes the key bar's place for a few seconds.
    if let (Some(l), Mode::Normal) = (app.toast(), app.mode) {
        let (sym, style) = match l.level {
            Level::Info => ("  ", DIM),
            Level::Ok => ("✓ ", hue(PLAIN, GOOD)),
            Level::Warn => ("! ", hue(BOLD, WARN)),
            Level::Error => ("✗ ", hue(BOLD, BAD)),
        };
        let line = Line::from(vec![Span::styled(format!(" {} ", l.time), DIM), Span::styled(sym, style), Span::styled(l.text.trim().to_string(), style)]);
        f.render_widget(Paragraph::new(line), area);
        return;
    }

    if let Mode::Input(kind) = app.mode {
        let prompt = match kind {
            InputKind::Commit => format!("commit message → {} (Tab: type)", app.selected().map_or("", |r| r.name.as_str())),
            InputKind::Tag => "new tag on HEAD: a name, then optionally a message (makes it an annotated tag)".to_string(),
            InputKind::RemoteAdd => "add remote: <name> <url> (a lone URL becomes origin when there is none)".to_string(),
            InputKind::RemoteUrl => format!("new URL of remote {}", app.remote_name()),
            InputKind::RemoteRename => format!("new name of remote {}", app.remote_name()),
            InputKind::Worktree => "new worktree for branch (created next to the repository, branch made if missing)".to_string(),
            InputKind::Amend => format!("amend last commit of {} (empty = keep message, Tab: type)", app.selected().map_or("", |r| r.name.as_str())),
            InputKind::Merge => format!("merge into {} (empty = upstream)", app.selected().map_or("", |r| r.branch.as_str())),
            InputKind::Search => "search".to_string(),
            InputKind::Branch => format!("new branch from {}", app.selected().map_or("", |r| r.branch.as_str())),
            InputKind::Init => "git init in folder (created if missing)".to_string(),
            InputKind::GhSearch => "search GitHub: words, user:name, repo:name or file:name (empty = your repositories)".to_string(),
            InputKind::CloneTo => "clone into".to_string(),
            InputKind::DeleteRepo => app.selected().map_or_else(String::new, |r| {
                // `~` instead of the home directory keeps the part that matters on screen.
                let home = crate::config::home();
                let path = r.path.strip_prefix(&home).map_or_else(|_| r.path.display().to_string(), |p| format!("~/{}", p.display()));
                format!("DELETE {path} from disk ({}). Type \"{}\"", r.risk_summary(), r.name)
            }),
        };
        let mut spans = vec![Span::styled(format!(" {prompt} "), hue(BOLD, ACCENT))];
        // The identity warning sits right next to the prompt so it is seen before the message is sent.
        if let (InputKind::Commit | InputKind::Amend, Some(warn)) = (kind, &app.commit_warn) {
            spans.push(Span::styled(format!("⚠ {warn} "), hue(BOLD, WARN)));
        }
        spans.extend([Span::styled("▸ ", hue(BOLD, ACCENT)), Span::styled(app.input.clone(), PLAIN), Span::styled("█", PLAIN)]);
        f.render_widget(Paragraph::new(Line::from(spans)), area);
        return;
    }

    if app.mode == Mode::Diff {
        let both = app.diff.as_ref().is_some_and(|v| v.both);
        let mut hints = vec![("↑↓", "scroll"), ("Space", "page"), ("[ ]", "hunk"), ("← →", "pan"), ("g G", "top/end")];
        if let Some(v) = app.diff.as_ref().filter(|v| v.stageable()) {
            hints.push(("s", if v.kind == DiffKind::Staged { "unstage hunk" } else { "stage hunk" }));
        }
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

    if let (Mode::Project, Some(v)) = (app.mode, &app.project) {
        let hints: &[(&str, &str)] = match (v.step, v.editing) {
            (ProjectStep::Lang, _) => &[("↑↓", "move"), ("type", "filter"), ("Enter", "select"), ("Esc", "clear/close")],
            (ProjectStep::Package, _) => &[("type", "the name"), ("Enter", "continue"), ("Esc", "back")],
            (ProjectStep::Variant, _) => &[("↑↓", "move"), ("Enter", "select"), ("Esc", "back")],
            (ProjectStep::License, false) => &[("↑↓", "move"), ("Tab", "edit holder"), ("Enter", "continue"), ("Esc", "back")],
            (ProjectStep::License, true) => &[("type", "copyright holder"), ("Enter", "done")],
            (ProjectStep::Confirm, _) => &[("Enter", "set up"), ("c", "toggle first commit"), ("Esc", "back")],
        };
        f.render_widget(Paragraph::new(Line::from(hint_spans(hints))), area);
        return;
    }
    if app.mode == Mode::History {
        let hints = [("↑↓", "move"), ("Enter", "show the commit"), ("g G", "newest/oldest"), ("Esc", "close")];
        f.render_widget(Paragraph::new(Line::from(hint_spans(&hints))), area);
        return;
    }
    if app.mode == Mode::Reflog {
        let hints = [("↑↓", "move"), ("u", "undo this entry"), ("Enter", "move branch here"), ("d", "show the commit"), ("Esc", "close")];
        f.render_widget(Paragraph::new(Line::from(hint_spans(&hints))), area);
        return;
    }
    if app.mode == Mode::Workflows {
        let hints = [("↑↓", "move"), ("Enter", "run by hand"), ("n", "new workflow"), ("o", "browser"), ("e", "edit"), ("f", "refresh"), ("Esc", "close")];
        f.render_widget(Paragraph::new(Line::from(hint_spans(&hints))), area);
        return;
    }
    if let (Mode::WorkflowNew, Some(w)) = (app.mode, &app.wizard) {
        use crate::app::WStep;
        let hints: &[(&str, &str)] = match w.step {
            WStep::Kind => &[("↑↓", "move"), ("Enter", "choose"), ("Esc", "cancel")],
            WStep::Toolchain => &[("↑↓", "move"), ("Enter", "choose"), ("Esc", "back")],
            WStep::Triggers | WStep::Options => &[("↑↓", "move"), ("Space", "check"), ("Enter", "next"), ("Esc", "back")],
            WStep::Details => &[("↑↓", "field"), ("type", "edit"), ("Enter", "preview"), ("Esc", "back")],
            WStep::Preview => &[("↑↓", "scroll"), ("Enter", "write the file"), ("c", "write, commit, push and run"), ("Esc", "back")],
        };
        f.render_widget(Paragraph::new(Line::from(hint_spans(hints))), area);
        return;
    }
    if app.mode == Mode::WorkflowRun {
        let hints = [("↑↓", "field"), ("type", "edit"), ("Space ←→", "check / choose"), ("Enter", "run"), ("Esc", "back")];
        f.render_widget(Paragraph::new(Line::from(hint_spans(&hints))), area);
        return;
    }
    if app.mode == Mode::Tags {
        let hints = [("↑↓", "move"), ("Enter", "show the commit"), ("n", "new"), ("p", "push"), ("d", "delete"), ("D", "delete + remote"), ("r", "release"), ("R", "draft release"), ("o", "open release"), ("f", "refresh"), ("Esc", "close")];
        f.render_widget(Paragraph::new(Line::from(hint_spans(&hints))), area);
        return;
    }
    if app.mode == Mode::Remotes {
        let hints = [("↑↓", "move"), ("a", "add"), ("e", "change URL"), ("n", "rename"), ("d", "remove"), ("Enter", "fetch"), ("Esc", "close")];
        f.render_widget(Paragraph::new(Line::from(hint_spans(&hints))), area);
        return;
    }
    if app.mode == Mode::Trees {
        let hints = [("↑↓", "move"), ("Enter", "go to worktree"), ("n", "new worktree"), ("d", "remove"), ("D", "force remove"), ("p", "prune"), ("u", "update submodule"), ("U", "update all"), ("Esc", "close")];
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
        let enter = if app.finder.as_ref().is_some_and(|v| v.scope == Scope::Users) { "repos" } else { "clone" };
        let hints = [("↑↓", "move"), ("←→", "page"), ("Enter", enter), ("/", "new search"), ("o", "browser"), ("r", "reload"), ("Esc", "close")];
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

/// `current` marks the hunk that `s` would stage or unstage.
fn diff_line(row: &Row, num_w: usize, hscroll: usize, current: bool) -> Line<'static> {
    let blank = " ".repeat(num_w * 2 + 2);
    match row {
        Row::Meta(s) | Row::Note(s) => Line::from(Span::styled(format!("{blank}{s}"), DIM.add_modifier(Modifier::ITALIC))),
        Row::File(s) => Line::from(vec![Span::styled(format!("{} ", "━".repeat(num_w * 2 + 2)), DIM), Span::styled(s.clone(), hue(BOLD, WARN))]),
        Row::Hunk(s) => {
            let pad = if current { format!("{}▸ ", &blank[..blank.len() - 2]) } else { blank };
            Line::from(Span::styled(format!("{pad}{s}"), hue(BOLD, ACCENT)))
        }
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
    let hunk = v.hunk();
    let hunk_label = match hunk {
        Some(k) if v.stageable() => format!(" · hunk {}/{}", k + 1, d.hunks.len()),
        _ => String::new(),
    };
    let marked = if v.stageable() { hunk.map(|k| d.hunks[k]) } else { None };
    block = block.title_top(
        Line::from(vec![
            Span::styled(format!(" +{}", d.added), hue(BOLD, GOOD)),
            Span::styled(format!(" −{}", d.removed), hue(BOLD, BAD)),
            Span::styled(format!("{hunk_label} · {}-{last}/{} ", v.scroll + 1, d.rows.len()), DIM),
        ])
        .right_aligned(),
    );
    let mut lines: Vec<Line> =
        d.rows[v.scroll..last].iter().enumerate().map(|(k, r)| diff_line(r, d.num_w, v.hscroll, marked == Some(v.scroll + k))).collect();
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

/// The reflog: a full-screen list of where HEAD has been, newest first.
fn draw_reflog(f: &mut Frame, app: &mut App) {
    let [area, _] = Layout::vertical([Constraint::Min(0), Constraint::Length(footer_height(app))]).areas(f.area());
    let Some(v) = app.reflog.as_mut() else { return };
    f.render_widget(Clear, area);
    let count = v.entries.as_ref().map_or(String::new(), |e| format!(" ({})", e.len()));
    let block = pane(format!(" Reflog · {}{count} ", v.name), true);
    let note = |msg: String, f: &mut Frame, block: Block<'static>| {
        f.render_widget(Paragraph::new(msg).style(DIM.add_modifier(Modifier::ITALIC)).block(block), area);
    };
    if let Some(e) = &v.error {
        return note(format!("git reflog failed: {}", e.lines().next().unwrap_or("")), f, block);
    }
    let Some(entries) = &v.entries else { return note("Loading the reflog…".to_string(), f, block) };
    if entries.is_empty() {
        return note("The reflog is empty.".to_string(), f, block);
    }
    let items: Vec<ListItem> = entries
        .iter()
        .map(|e| {
            // `commit: msg` -> the kind in color, the rest plain.
            let (kind, rest) = e.subject.split_once(": ").unwrap_or((e.subject.as_str(), ""));
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:<8}", e.short), hue(DIM, WARN)),
                Span::styled(fit(&e.date, 15), DIM),
                Span::styled(kind.to_string(), hue(BOLD, ACCENT)),
                Span::styled(if rest.is_empty() { String::new() } else { format!(": {rest}") }, PLAIN),
            ]))
        })
        .collect();
    f.render_stateful_widget(List::new(items).block(block).highlight_style(INVERSE), area, &mut v.state);
}

/// The tags of a repository as a popup: kind, commit, age, whether the remote has it, its GitHub release and message.
fn draw_tags(f: &mut Frame, app: &mut App) {
    let last = app.log.last().map(|l| l.text.clone()).unwrap_or_default();
    let Some(v) = app.tags.as_mut() else { return };
    let rows = v.items.as_ref().map_or(1, Vec::len) as u16;
    let area = f.area();
    let w = 130.min(area.width);
    let h = (rows + 2).clamp(5, area.height.saturating_sub(2).max(5)).min(area.height);
    let [_, mid, _] = Layout::vertical([Constraint::Fill(1), Constraint::Length(h), Constraint::Fill(1)]).areas(area);
    let [_, popup, _] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(w), Constraint::Fill(1)]).areas(mid);
    f.render_widget(Clear, popup);
    // The bottom line says what could not be learned (offline, no gh), else the newest log message.
    let note = match (&v.remote, &v.releases) {
        (Some(Err(e)), _) if v.slug.is_none() || !e.is_empty() => format!("remote: {}", e.lines().next().unwrap_or("")),
        (_, Some(Err(e))) if v.slug.is_some() => format!("releases: {}", e.lines().next().unwrap_or("")),
        _ => last,
    };
    let block = pane(format!(" Tags · {} ", v.name), true).title_bottom(Line::from(Span::styled(format!(" {note} "), DIM)));
    let msg = |text: String, f: &mut Frame, block: Block<'static>| {
        f.render_widget(Paragraph::new(text).style(DIM.add_modifier(Modifier::ITALIC)).block(block), popup);
    };
    if let Some(e) = &v.error {
        return msg(format!("git tag failed: {}", e.lines().next().unwrap_or("")), f, block);
    }
    let Some(items) = &v.items else { return msg("Loading…".to_string(), f, block) };
    if items.is_empty() {
        return msg("No tags yet. Press n to make one on the current commit.".to_string(), f, block);
    }
    let subject_w = (popup.width as usize).saturating_sub(4 + 22 + 2 + 9 + 16 + 8 + 9 + 4).max(8);
    let rows: Vec<ListItem> = items
        .iter()
        .map(|t| {
            let (remote, remote_style) = match &v.remote {
                None => ("…", DIM),
                Some(Err(_)) => ("?", DIM),
                Some(Ok(set)) if set.contains(&t.name) => ("pushed", hue(PLAIN, GOOD)),
                Some(Ok(_)) => ("local", hue(BOLD, WARN)),
            };
            let release = match v.releases.as_ref().and_then(|r| r.as_ref().ok()).and_then(|m| m.get(&t.name)) {
                Some(r) if r.draft => "draft",
                Some(r) if r.prerelease => "pre",
                Some(r) if r.latest => "latest",
                Some(_) => "release",
                None => "",
            };
            ListItem::new(Line::from(vec![
                Span::styled(fit(&t.name, 22), BOLD),
                Span::styled(if t.annotated { " ◆ " } else { " · " }, DIM),
                Span::styled(fit(&t.hash, 8), hue(PLAIN, ACCENT)),
                Span::styled(format!(" {}", fit(&t.age, 15)), DIM),
                Span::styled(format!(" {}", fit(remote, 7)), remote_style),
                Span::styled(format!(" {}", fit(release, 8)), hue(PLAIN, ACCENT)),
                Span::styled(format!(" {}", fit(&t.subject, subject_w)), PLAIN),
            ]))
        })
        .collect();
    f.render_stateful_widget(List::new(rows).block(block).highlight_style(INVERSE), popup, &mut v.state);
}

/// The remotes of a repository as a popup, with the newest log message below it.
fn draw_remotes(f: &mut Frame, app: &mut App) {
    let last = app.log.last().map(|l| l.text.clone()).unwrap_or_default();
    let Some(v) = app.remotes.as_mut() else { return };
    let rows = v.items.as_ref().map_or(1, Vec::len) as u16;
    let area = f.area();
    let w = 110.min(area.width);
    let h = (rows + 2).clamp(5, area.height.saturating_sub(2).max(5)).min(area.height);
    let [_, mid, _] = Layout::vertical([Constraint::Fill(1), Constraint::Length(h), Constraint::Fill(1)]).areas(area);
    let [_, popup, _] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(w), Constraint::Fill(1)]).areas(mid);
    f.render_widget(Clear, popup);
    let block = pane(format!(" Remotes · {} ", v.name), true).title_bottom(Line::from(Span::styled(format!(" {last} "), DIM)));
    let note = |msg: String, f: &mut Frame, block: Block<'static>| {
        f.render_widget(Paragraph::new(msg).style(DIM.add_modifier(Modifier::ITALIC)).block(block), popup);
    };
    if let Some(e) = &v.error {
        return note(format!("git remote failed: {}", e.lines().next().unwrap_or("")), f, block);
    }
    let Some(items) = &v.items else { return note("Loading…".to_string(), f, block) };
    if items.is_empty() {
        return note("No remotes: this repository is local only. Press a to add one.".to_string(), f, block);
    }
    let url_w = (popup.width as usize).saturating_sub(4 + 14 + 8).max(16);
    let rows: Vec<ListItem> = items
        .iter()
        .map(|r| {
            let mut spans = vec![
                Span::styled(fit(&r.name, 13), BOLD),
                Span::raw(" "),
                Span::styled(fit(&r.url, url_w), PLAIN),
                Span::styled(if github::slug(&r.url).is_some() { " github" } else { "       " }, hue(PLAIN, GOOD)),
            ];
            if let Some(push) = &r.push_url {
                spans.push(Span::styled(format!(" push → {push}"), hue(DIM, WARN)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    f.render_stateful_widget(List::new(rows).block(block).highlight_style(INVERSE), popup, &mut v.state);
}

/// Worktrees and submodules of a repository, as a popup with the newest log message below it.
fn draw_trees(f: &mut Frame, app: &mut App) {
    let last = app.log.last().map(|l| l.text.clone()).unwrap_or_default();
    let Some(v) = app.trees.as_mut() else { return };
    let rows = v.items.as_ref().map_or(1, Vec::len) as u16;
    let area = f.area();
    let w = 110.min(area.width);
    let h = (rows + 2).clamp(5, area.height.saturating_sub(2).max(5)).min(area.height);
    let [_, mid, _] = Layout::vertical([Constraint::Fill(1), Constraint::Length(h), Constraint::Fill(1)]).areas(area);
    let [_, popup, _] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(w), Constraint::Fill(1)]).areas(mid);
    f.render_widget(Clear, popup);
    let block = pane(format!(" Worktrees and submodules · {} ", v.name), true).title_bottom(Line::from(Span::styled(format!(" {last} "), DIM)));
    let note = |msg: String, f: &mut Frame, block: Block<'static>| {
        f.render_widget(Paragraph::new(msg).style(DIM.add_modifier(Modifier::ITALIC)).block(block), popup);
    };
    if let Some(e) = &v.error {
        return note(format!("git worktree failed: {}", e.lines().next().unwrap_or("")), f, block);
    }
    let Some(items) = &v.items else { return note("Loading…".to_string(), f, block) };
    let home = crate::config::home().display().to_string();
    let path_w = (popup.width as usize).saturating_sub(4 + 11 + 22 + 9 + 14).max(16);
    let rows: Vec<ListItem> = items
        .iter()
        .map(|i| {
            let (tag, tag_style, flags) = match &i.kind {
                TreeKind::Main => ("main", hue(BOLD, GOOD), String::new()),
                TreeKind::Linked { locked, prunable } => {
                    let flags = [(*locked, "locked "), (*prunable, "missing ")].iter().filter(|(on, _)| *on).map(|(_, s)| *s).collect::<String>();
                    ("worktree", hue(PLAIN, ACCENT), flags)
                }
                TreeKind::Submodule { state } => {
                    let flags = match state {
                        '-' => "not initialised",
                        '+' => "other commit",
                        'U' => "conflicts",
                        _ => "",
                    };
                    ("submodule", hue(PLAIN, PINK), flags.to_string())
                }
            };
            // Worktree paths are absolute: show `~` for the home directory so the useful part fits.
            let path = i.path.strip_prefix(&home).map_or_else(|| i.path.clone(), |p| format!("~{p}"));
            let flag_style = if flags.starts_with("conflicts") || flags.starts_with("missing") { hue(BOLD, BAD) } else { hue(PLAIN, WARN) };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{tag:<10}"), tag_style),
                Span::styled(fit(&path, path_w), BOLD),
                Span::raw(" "),
                Span::styled(fit(&i.label, 20), hue(PLAIN, ACCENT)),
                Span::raw(" "),
                Span::styled(format!("{:<8}", i.head), hue(DIM, WARN)),
                Span::styled(flags, flag_style),
            ]))
        })
        .collect();
    if rows.is_empty() {
        return note("No worktrees or submodules.".to_string(), f, block);
    }
    f.render_stateful_widget(List::new(rows).block(block).highlight_style(INVERSE), popup, &mut v.state);
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

/// GitHub search results (repositories, users or files; or your own repositories) as a popup, one page at a time.
fn draw_finder(f: &mut Frame, app: &mut App) {
    let cloned: Vec<String> = app.repos.iter().filter_map(|r| r.slug.as_ref().map(|s| s.to_lowercase())).collect();
    let last = app.log.last().map(|l| l.text.clone()).unwrap_or_default();
    let Some(v) = app.finder.as_mut() else { return };
    let rows = v.hits.as_ref().map_or(1, Hits::len) as u16;
    let area = f.area();
    let w = 120.min(area.width);
    let h = (rows + 2).clamp(5, area.height.saturating_sub(2).max(5)).min(area.height);
    let [_, mid, _] = Layout::vertical([Constraint::Fill(1), Constraint::Length(h), Constraint::Fill(1)]).areas(area);
    let [_, popup, _] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(w), Constraint::Fill(1)]).areas(mid);
    f.render_widget(Clear, popup);
    let what = if v.scope == Scope::Repos && v.terms.is_empty() {
        "your repositories".to_string()
    } else {
        format!("{} “{}”", v.scope.label(), v.terms)
    };
    // GitHub reports how many matches there are (at most 1000 are reachable); your own list only knows whether more follow.
    let pages = match v.total {
        Some(t) => format!(" · page {}/{} · {t} found", v.page, t.min(1000).div_ceil(github::PAGE as u64).max(1)),
        None => format!(" · page {}{}", v.page, if v.more { "+" } else { "" }),
    };
    let block = pane(format!(" GitHub · {what}{pages} "), true).title_bottom(Line::from(Span::styled(format!(" {last} "), DIM)));
    let note = |msg: String, f: &mut Frame, block: Block<'static>| {
        f.render_widget(Paragraph::new(msg).style(DIM.add_modifier(Modifier::ITALIC)).block(block), popup);
    };
    if let Some(e) = &v.error {
        return note(format!("gh failed: {}", e.lines().next().unwrap_or("")), f, block);
    }
    let Some(hits) = &v.hits else { return note("Searching GitHub…".to_string(), f, block) };
    if hits.len() == 0 {
        return note(format!("No {} found.", v.scope.label().trim_end_matches(" of")), f, block);
    }
    let rows: Vec<ListItem> = match hits {
        Hits::Repos(items) => {
            let desc_w = (popup.width as usize).saturating_sub(4 + 38 + 7 + 11 + 5 + 8).max(8);
            items
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
                .collect()
        }
        Hits::Users(items) => items
            .iter()
            .map(|u| ListItem::new(Line::from(vec![Span::styled(fit(&u.login, 38), BOLD), Span::styled(u.kind.clone(), DIM)])))
            .collect(),
        Hits::Files(items) => {
            let path_w = (popup.width as usize).saturating_sub(4 + 46 + 8).max(8);
            items
                .iter()
                .map(|i| {
                    let have = cloned.contains(&i.slug.to_lowercase());
                    ListItem::new(Line::from(vec![
                        Span::styled(fit(&i.slug, 38), if have { hue(BOLD, GOOD) } else { BOLD }),
                        Span::styled(if have { "cloned " } else { "       " }, hue(PLAIN, GOOD)),
                        Span::styled(fit(&i.path, path_w), PLAIN),
                    ]))
                })
                .collect()
        }
    };
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
        ("Tab, ← →", "switch between repositories, languages and changes"),
        ("f, 1 2 3 4", "filter: all / owned / cloned / attention"),
        ("s", "sort: name / recent / changes / behind"),
        ("/", "search by name, branch or path (Esc clears)"),
        ("Space, a, u", "stage / unstage the file, stage all, unstage all"),
        ("c / E", "commit staged changes (Tab picks feat/fix/…) / amend the last commit"),
        ("p / P", "pull (fast-forward only) / push (new branches with -u)"),
        ("F / A", "fetch selected repository / every one in view"),
        ("L / U", "pull every behind / push every ahead repository in view"),
        ("m z Z", "merge a branch (empty = upstream) / stash all changes / pop the stash"),
        ("d v b I", "diff of the file / commit log / branches / git init in any folder"),
        ("W", "worktrees and submodules: add / remove / go to / update"),
        ("N", "GitHub workflows: run one by hand with its inputs, or write a new one (n)"),
        ("V", "tags and releases: make / push / delete a tag, create a GitHub release"),
        ("O", "reflog: where HEAD has been; u undoes an entry, Enter moves the branch there"),
        ("D", "DELETE the repository from disk (asks you to type its name)"),
        ("i y B S", "GitHub: issues / pull requests / repo in browser / find & clone"),
        ("e / o", "yazi on the file or repository / file manager"),
        ("t / T", "embedded terminal in the repository, or in the highlighted file's folder from the changes list (Ctrl+T hides it)"),
        ("K / X", "new project in an empty repository: language, build tool, license / undo that setup"),
        ("* / x / H", "pin / ignore the repository / show ignored ones"),
        ("r", "remotes: add / change URL / rename / remove / fetch one"),
        ("R / w", "rescan all roots / refresh CI states"),
        ("Ctrl+R, F5", "reload the selected repository (changes are also picked up on their own)"),
        (": / Ctrl+K", "command palette: type to find any action, Enter runs it"),
        ("h l C", "hide the key bar / hide the log panel / toggle colors"),
        ("? / q", "this help (any key closes it) / quit"),
        ("↑n ↓n ★ ⊘", "ahead · behind · pinned · ignored"),
        ("✓ ✗ ⠋ ○", "CI: passing · failing · running · inconclusive"),
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
