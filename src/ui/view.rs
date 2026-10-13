//! Drawing of the file finder and the code viewer (see `app::view`).

use super::*;
use crate::app::Prompt;

/// The file finder: a centered popup with the query on top and the matching files below.
pub(super) fn draw_picker(f: &mut Frame, app: &mut App) {
    let Some(v) = app.picker.as_mut() else { return };
    let area = f.area();
    let w = 110.min(area.width);
    let h = area.height.saturating_sub(4).max(8).min(area.height);
    let [_, mid, _] = Layout::vertical([Constraint::Fill(1), Constraint::Length(h), Constraint::Fill(1)]).areas(area);
    let [_, popup, _] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(w), Constraint::Fill(1)]).areas(mid);
    f.render_widget(Clear, popup);
    let total = v.files.as_ref().map_or(0, Vec::len);
    let count = if v.files.is_some() { format!(" {}/{} ", v.shown.len(), total) } else { " … ".to_string() };
    let block = pane(format!(" Files · {} ", v.name), true).title_top(Line::from(Span::styled(count, DIM)).right_aligned());
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    let [query_row, list_area] = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(inner);
    let query = Line::from(vec![Span::styled("› ", hue(BOLD, ACCENT)), Span::styled(v.query.clone(), BOLD), Span::styled("█", PLAIN)]);
    f.render_widget(Paragraph::new(query), query_row);

    let msg = |text: String, f: &mut Frame| f.render_widget(Paragraph::new(text).style(DIM.add_modifier(Modifier::ITALIC)), list_area);
    if let Some(e) = &v.error {
        return msg(format!("git ls-files failed: {}", e.lines().next().unwrap_or("")), f);
    }
    let Some(files) = &v.files else { return msg("Listing files…".to_string(), f) };
    if v.shown.is_empty() {
        return msg(if files.is_empty() { "This repository has no files.".to_string() } else { "No file matches.".to_string() }, f);
    }
    let width = list_area.width as usize;
    let items: Vec<ListItem> = v
        .shown
        .iter()
        .map(|(k, at)| {
            let path = &files[*k];
            let name_from = path.rfind('/').map_or(0, |i| path[..=i].chars().count());
            let mark = if v.changed.contains(path) { Span::styled("● ", hue(PLAIN, WARN)) } else { Span::raw("  ") };
            let mut spans = vec![mark];
            spans.extend(path.chars().take(width.saturating_sub(2)).enumerate().map(|(i, c)| {
                let base = if i >= name_from { BOLD } else { DIM };
                Span::styled(c.to_string(), if at.contains(&i) { hue(BOLD, WARN) } else { base })
            }));
            ListItem::new(Line::from(spans))
        })
        .collect();
    f.render_stateful_widget(List::new(items).highlight_style(INVERSE), list_area, &mut v.state);
}

/// The code viewer: a full-screen panel over the dashboard, with line numbers and highlighting.
pub(super) fn draw_code(f: &mut Frame, app: &mut App) {
    let [area, _] = Layout::vertical([Constraint::Min(0), Constraint::Length(footer_height(app))]).areas(f.area());
    let Some(v) = app.code.as_mut() else { return };
    let name = v.repo.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let block = pane(format!(" File · {name}/{} ", v.file), true);
    v.height = area.height.saturating_sub(2) as usize;
    f.render_widget(Clear, area);
    let note = |msg: String, block: Block<'static>, f: &mut Frame| {
        f.render_widget(Paragraph::new(msg).style(DIM.add_modifier(Modifier::ITALIC)).block(block), area);
    };
    if let Some(e) = &v.error {
        return note(e.lines().next().unwrap_or("").to_string(), block, f);
    }
    let Some(lines) = &v.lines else { return note("Loading…".to_string(), block, f) };
    if lines.is_empty() {
        return note("The file is empty.".to_string(), block, f);
    }

    v.scroll = v.scroll.min(lines.len().saturating_sub(v.height));
    let last = (v.scroll + v.height).min(lines.len());
    let num_w = lines.len().to_string().len();
    let mut right = vec![Span::styled(format!(" {}/{}", v.cursor + 1, lines.len()), DIM)];
    if !v.query.is_empty() {
        let at = v.hits.iter().position(|&h| h == v.cursor).map_or(String::new(), |i| format!("{}/", i + 1));
        right.push(Span::styled(format!(" · \"{}\" {at}{}", v.query, v.hits.len()), hue(PLAIN, WARN)));
    }
    if v.cut {
        right.push(Span::styled(" · cut off", hue(PLAIN, WARN)));
    }
    right.push(Span::raw(" "));
    let block = block.title_top(Line::from(right).right_aligned());
    let rows: Vec<Line> = (v.scroll..last)
        .map(|i| {
            let here = i == v.cursor;
            let hit = v.hits.binary_search(&i).is_ok();
            let (mark, style) = match (here, hit) {
                (true, _) => ("▸", hue(BOLD, ACCENT)),
                (false, true) => ("•", hue(BOLD, WARN)),
                _ => (" ", DIM),
            };
            let mut spans = vec![Span::styled(format!("{mark}{:>num_w$} ", i + 1), style)];
            spans.extend(code_spans(&lines[i], v.hscroll));
            Line::from(spans)
        })
        .collect();
    f.render_widget(Paragraph::new(rows).block(block), area);
}

/// The footer while a viewer prompt is open: what is being asked, and the text typed so far.
pub(super) fn code_prompt(kind: Prompt, text: &str) -> Line<'static> {
    let what = match kind {
        Prompt::Find => "find",
        Prompt::Goto => "go to line",
    };
    Line::from(vec![Span::styled(format!(" {what} "), hue(BOLD, ACCENT)), Span::styled("▸ ", hue(BOLD, ACCENT)), Span::styled(text.to_string(), PLAIN), Span::styled("█", PLAIN)])
}
