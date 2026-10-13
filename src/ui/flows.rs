//! Drawing of the workflows view and the run form (see `app::flows`).

use super::*;
use crate::workflow::InputKind;

/// The workflows of a repository as a popup: name, file, whether they can be started by hand, and the latest run.
pub(super) fn draw_workflows(f: &mut Frame, app: &mut App) {
    let last = app.log.last().map(|l| l.text.clone()).unwrap_or_default();
    let fresh = app.fresh_log().map(|l| l.text.clone());
    let tick = app.tick;
    let Some(v) = app.workflows.as_mut() else { return };
    let rows = v.items.len().max(1) as u16;
    let area = f.area();
    let w = 120.min(area.width);
    let h = (rows + 2).clamp(5, area.height.saturating_sub(2).max(5)).min(area.height);
    let [_, mid, _] = Layout::vertical([Constraint::Fill(1), Constraint::Length(h), Constraint::Fill(1)]).areas(area);
    let [_, popup, _] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(w), Constraint::Fill(1)]).areas(mid);
    f.render_widget(Clear, popup);
    // The bottom line says what could not be learned (no gh, no GitHub remote), else the newest log message.
    let note = match (&fresh, &v.runs) {
        (Some(msg), _) => msg.trim().to_string(),
        (None, Some(Err(e))) => format!("latest runs: {}", e.lines().next().unwrap_or("")),
        _ => last,
    };
    let block = pane(format!(" Workflows · {} ", v.name), true).title_bottom(Line::from(Span::styled(format!(" {note} "), DIM)));
    if v.items.is_empty() {
        let text = format!("No workflows in {}. Press n to create one.", crate::workflow::DIR);
        f.render_widget(Paragraph::new(text).style(DIM.add_modifier(Modifier::ITALIC)).block(block), popup);
        return;
    }
    let tail_w = (popup.width as usize).saturating_sub(4 + 28 + 24 + 9 + 3).max(10);
    let rows: Vec<ListItem> = v
        .items
        .iter()
        .map(|wf| {
            let (kind, kind_style) = if wf.dispatch { ("manual", hue(BOLD, GOOD)) } else { ("auto", DIM) };
            let last_run = match &v.runs {
                None => Span::styled("…", DIM),
                Some(Err(_)) => Span::raw(""),
                Some(Ok(map)) => match map.get(&wf.name) {
                    None => Span::styled("never run", DIM),
                    Some(r) => {
                        let (sym, style) = match (r.status.as_str(), r.conclusion.as_str()) {
                            ("completed", "success") => ("✓".to_string(), hue(PLAIN, GOOD)),
                            ("completed", "failure" | "timed_out" | "startup_failure") => ("✗".to_string(), hue(BOLD, BAD)),
                            ("completed", _) => ("○".to_string(), DIM),
                            _ => (SPINNER[tick % SPINNER.len()].to_string(), hue(PLAIN, PINK)),
                        };
                        Span::styled(fit(&format!("{sym} {} ago · {}", r.age, r.branch), tail_w), style)
                    }
                },
            };
            ListItem::new(Line::from(vec![
                Span::styled(fit(&wf.name, 28), BOLD),
                Span::styled(format!(" {}", fit(&wf.file, 23)), DIM),
                Span::styled(format!(" {}", fit(kind, 7)), kind_style),
                Span::raw(" "),
                last_run,
            ]))
        })
        .collect();
    f.render_stateful_widget(List::new(rows).block(block).highlight_style(INVERSE), popup, &mut v.state);
}

/// The form that starts a workflow: one line per field, the focused one with its cursor.
pub(super) fn draw_run_form(f: &mut Frame, app: &mut App) {
    let Some(form) = app.run_form.as_ref() else { return };
    let area = f.area();
    let w = 100.min(area.width);
    let extra = u16::from(form.note.is_some()) + u16::from(form.error.is_some());
    let h = (form.fields.len() as u16 + extra + 4).min(area.height);
    let [_, mid, _] = Layout::vertical([Constraint::Fill(1), Constraint::Length(h), Constraint::Fill(1)]).areas(area);
    let [_, popup, _] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(w), Constraint::Fill(1)]).areas(mid);
    f.render_widget(Clear, popup);
    let block = pane(format!(" Run · {} ", form.file), true);
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let label_w = form.fields.iter().map(|x| x.label.chars().count() + usize::from(x.required)).max().unwrap_or(3).max(3);
    let help_w = (inner.width as usize).saturating_sub(label_w + 28).max(8);
    let mut lines: Vec<Line> = Vec::new();
    if let Some(n) = &form.note {
        lines.push(Line::from(Span::styled(fit(&format!("! {n}"), inner.width as usize), hue(BOLD, WARN))));
    }
    lines.push(Line::from(Span::styled(fit(&format!("{} · the workflow starts on GitHub", form.title), inner.width as usize), DIM)));
    for (i, x) in form.fields.iter().enumerate() {
        let here = i == form.cursor;
        let label = format!("{}{}", x.label, if x.required { "*" } else { "" });
        let value: Vec<Span> = match x.kind {
            InputKind::Text => {
                let mut v = vec![Span::styled(x.value.clone(), if here { BOLD } else { PLAIN })];
                if here {
                    v.push(Span::styled("█", PLAIN));
                }
                v
            }
            InputKind::Bool => vec![Span::styled(if x.value == "true" { "[x] yes" } else { "[ ] no" }, if here { BOLD } else { PLAIN })],
            InputKind::Choice => vec![Span::styled(format!("‹ {} ›", x.value), if here { BOLD } else { PLAIN })],
        };
        let mut spans = vec![
            Span::styled(if here { "▸ " } else { "  " }, hue(BOLD, ACCENT)),
            Span::styled(format!("{label:<label_w$} "), if here { hue(BOLD, ACCENT) } else { BOLD }),
        ];
        spans.extend(value);
        if !x.help.is_empty() {
            spans.push(Span::styled(format!("   {}", fit(&x.help, help_w).trim_end()), DIM));
        }
        lines.push(Line::from(spans));
    }
    if let Some(e) = &form.error {
        lines.push(Line::from(Span::styled(format!("✗ {e}"), hue(BOLD, BAD))));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// The wizard that writes a new workflow: a popup per page, the compiled file on the last one.
pub(super) fn draw_wizard(f: &mut Frame, app: &mut App) {
    use crate::app::WStep;
    use crate::workflow::{Kind, TOOLCHAINS};
    let Some(w) = app.wizard.as_mut() else { return };
    let area = f.area();
    let page = match w.step {
        WStep::Kind => "what for",
        WStep::Toolchain => "language",
        WStep::Triggers => "when it runs",
        WStep::Options => "options",
        WStep::Details => "details",
        WStep::Preview => "preview",
    };
    // Rows of the page, plus the lines around them (borders and the error line).
    let rows = match w.step {
        WStep::Kind => Kind::ALL.len(),
        WStep::Toolchain => TOOLCHAINS.len(),
        WStep::Triggers | WStep::Options => w.checks.len(),
        WStep::Details => w.details.len(),
        WStep::Preview => area.height as usize,
    };
    let width = if w.step == WStep::Preview { 110 } else { 96 }.min(area.width);
    let h = (rows as u16 + 4).clamp(7, area.height.saturating_sub(2).max(7)).min(area.height);
    let [_, mid, _] = Layout::vertical([Constraint::Fill(1), Constraint::Length(h), Constraint::Fill(1)]).areas(area);
    let [_, popup, _] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(width), Constraint::Fill(1)]).areas(mid);
    f.render_widget(Clear, popup);
    let block = pane(format!(" New workflow · {} · {page} ", w.name), true);
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    let [body, err] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
    if let Some(e) = &w.error {
        f.render_widget(Paragraph::new(Span::styled(format!("✗ {e}"), hue(BOLD, BAD))), err);
    }

    match w.step {
        WStep::Kind => {
            let note_w = (body.width as usize).saturating_sub(16);
            let items: Vec<ListItem> = Kind::ALL
                .iter()
                .map(|k| ListItem::new(Line::from(vec![Span::styled(fit(k.name(), 15), BOLD), Span::styled(fit(k.note(), note_w), DIM)])))
                .collect();
            f.render_stateful_widget(List::new(items).highlight_style(INVERSE), body, &mut w.kind_state);
        }
        WStep::Toolchain => {
            let items: Vec<ListItem> = TOOLCHAINS
                .iter()
                .enumerate()
                .map(|(i, (_, name, _))| {
                    let mut spans = vec![Span::styled(fit(name, 32), BOLD)];
                    if w.detected == Some(i) {
                        spans.push(Span::styled("◀ this repository looks like it", hue(PLAIN, GOOD)));
                    }
                    ListItem::new(Line::from(spans))
                })
                .collect();
            f.render_stateful_widget(List::new(items).highlight_style(INVERSE), body, &mut w.tool_state);
        }
        WStep::Triggers | WStep::Options => {
            let hint_w = (body.width as usize).saturating_sub(30);
            let items: Vec<ListItem> = w
                .checks
                .iter()
                .map(|c| {
                    let (mark, style) = if c.on { ("[x]", hue(BOLD, GOOD)) } else { ("[ ]", PLAIN) };
                    ListItem::new(Line::from(vec![
                        Span::styled(mark, style),
                        Span::styled(format!(" {}", fit(&c.label, 26)), BOLD),
                        Span::styled(fit(&c.hint, hint_w), DIM),
                    ]))
                })
                .collect();
            f.render_stateful_widget(List::new(items).highlight_style(INVERSE), body, &mut w.check_state);
        }
        WStep::Details => {
            let label_w = w.details.iter().map(|d| d.label.len()).max().unwrap_or(4);
            let help_w = (body.width as usize).saturating_sub(label_w + 30).max(8);
            let lines: Vec<Line> = w
                .details
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    let here = i == w.cursor;
                    let mut spans = vec![
                        Span::styled(if here { "▸ " } else { "  " }, hue(BOLD, ACCENT)),
                        Span::styled(format!("{:<label_w$} ", d.label), if here { hue(BOLD, ACCENT) } else { BOLD }),
                        Span::styled(d.value.clone(), if here { BOLD } else { PLAIN }),
                    ];
                    if here {
                        spans.push(Span::styled("█", PLAIN));
                    }
                    spans.push(Span::styled(format!("   {}", fit(&d.help, help_w).trim_end()), DIM));
                    Line::from(spans)
                })
                .collect();
            f.render_widget(Paragraph::new(lines), body);
        }
        WStep::Preview => {
            let total = w.yaml.lines().count();
            let [head, text] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(body);
            let top = if w.exists {
                Line::from(Span::styled(fit(&format!("! {}/{} exists and will be replaced", crate::workflow::DIR, w.file), body.width as usize), hue(BOLD, WARN)))
            } else {
                Line::from(Span::styled(format!("→ {}/{}   ({total} lines)", crate::workflow::DIR, w.file), DIM))
            };
            f.render_widget(Paragraph::new(top), head);
            w.height = text.height as usize;
            w.scroll = w.scroll.min(total.saturating_sub(w.height));
            let lines: Vec<Line> = w
                .yaml
                .lines()
                .enumerate()
                .skip(w.scroll)
                .take(w.height)
                .map(|(i, l)| Line::from(vec![Span::styled(format!("{:>3} ", i + 1), DIM), Span::styled(l.to_string(), PLAIN)]))
                .collect();
            f.render_widget(Paragraph::new(lines), text);
        }
    }
}
