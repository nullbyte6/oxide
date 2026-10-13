//! Drawing of the task menu and the task output pane (see `app::tasks`).

use super::*;
use ratatui::widgets::Wrap;

use crate::app::TaskState;

/// The tasks of a repository as a popup: name, command and where it comes from.
pub(super) fn draw_tasks(f: &mut Frame, app: &mut App) {
    let running = app.task.as_ref().map(|t| (t.name.clone(), t.state == TaskState::Running));
    let Some(v) = app.tasks.as_mut() else { return };
    let rows = v.items.len().max(1) as u16;
    let area = f.area();
    let w = 120.min(area.width);
    let h = (rows + 2).clamp(5, area.height.saturating_sub(2).max(5)).min(area.height);
    let [_, mid, _] = Layout::vertical([Constraint::Fill(1), Constraint::Length(h), Constraint::Fill(1)]).areas(area);
    let [_, popup, _] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(w), Constraint::Fill(1)]).areas(mid);
    f.render_widget(Clear, popup);
    let mut block = pane(format!(" Tasks · {} ", v.name), true);
    if let Some((name, active)) = running {
        let text = if active { format!(" {name} is running: o shows it ") } else { format!(" o shows the output of {name} ") };
        block = block.title_bottom(Line::from(Span::styled(text, DIM)));
    }
    if v.items.is_empty() {
        let text = "No tasks found. They come from Cargo.toml, package.json, go.mod, Makefile, justfile and other project files, or from `tasks` in config.lua.";
        f.render_widget(Paragraph::new(text).style(DIM.add_modifier(Modifier::ITALIC)).block(block).wrap(Wrap { trim: true }), popup);
        return;
    }
    let src_w = 16;
    let name_w = 22;
    let cmd_w = (popup.width as usize).saturating_sub(4 + name_w + src_w + 2).max(10);
    let rows: Vec<ListItem> = v
        .items
        .iter()
        .map(|t| {
            ListItem::new(Line::from(vec![
                Span::styled(fit(&t.name, name_w), BOLD),
                Span::styled(format!(" {}", fit(&t.cmd, cmd_w)), PLAIN),
                Span::styled(format!(" {}", fit(&t.source, src_w)), DIM),
            ]))
        })
        .collect();
    f.render_stateful_widget(List::new(rows).block(block).highlight_style(INVERSE), popup, &mut v.state);
}

/// The output of the task: a full-screen panel with the command line on its bottom border.
pub(super) fn draw_task(f: &mut Frame, app: &mut App) {
    let [area, _] = Layout::vertical([Constraint::Min(0), Constraint::Length(footer_height(app))]).areas(f.area());
    let tick = app.tick;
    let Some(v) = app.task.as_mut() else { return };
    let name = v.repo.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    v.height = area.height.saturating_sub(2) as usize;
    f.render_widget(Clear, area);

    let status = match v.state {
        TaskState::Running => {
            let tag = if v.cancelled { "stopping" } else { "running" };
            Span::styled(format!(" {} {tag} {:.0}s ", SPINNER[tick % SPINNER.len()], v.started.elapsed().as_secs_f64()), hue(BOLD, PINK))
        }
        TaskState::Ended { .. } if v.cancelled => Span::styled(" stopped ", hue(BOLD, WARN)),
        TaskState::Ended { code: Some(0), secs } => Span::styled(format!(" ✓ passed in {secs:.1}s "), hue(BOLD, GOOD)),
        TaskState::Ended { code: Some(c), secs } => Span::styled(format!(" ✗ failed (exit {c}) in {secs:.1}s "), hue(BOLD, BAD)),
        TaskState::Ended { code: None, secs } => Span::styled(format!(" ✗ ended by a signal after {secs:.1}s "), hue(BOLD, BAD)),
    };
    let total = v.lines.len();
    let max = total.saturating_sub(v.height);
    v.scroll = if v.follow { max } else { v.scroll.min(max) };
    let last = (v.scroll + v.height).min(total);
    let dropped = if v.dropped > 0 { format!(" · first {} lines dropped", v.dropped) } else { String::new() };
    let block = pane(format!(" Task · {name} · {} ", v.name), true)
        .title_top(Line::from(vec![status, Span::styled(format!("{}-{last}/{total}{dropped} ", (v.scroll + 1).min(total)), DIM)]).right_aligned())
        .title_bottom(Line::from(Span::styled(format!(" $ {} ", v.cmd), DIM)));
    let lines: Vec<Line> = v.lines[v.scroll..last].iter().map(|l| Line::from(Span::raw(l.chars().skip(v.hscroll).collect::<String>()))).collect();
    f.render_widget(Paragraph::new(lines).block(block), area);
}
