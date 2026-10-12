//! Banner art: `banner.sh` is embedded at compile time and its colored `echo -e "..."` lines
//! are converted into styled Ratatui lines. The script is never executed.

use std::sync::OnceLock;

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

const SOURCE: &str = include_str!("banner.sh");

pub struct Banner {
    pub lines: Vec<Line<'static>>,
    /// Width in terminal columns of the widest line.
    pub width: u16,
}

/// The parsed banner; parsed once, on first use.
pub fn get() -> &'static Banner {
    static BANNER: OnceLock<Banner> = OnceLock::new();
    BANNER.get_or_init(|| {
        // Only `echo -e "<text>"` lines are art; anything else in the script is ignored.
        let lines: Vec<Line<'static>> = SOURCE
            .lines()
            .filter_map(|l| {
                let body = l.trim().strip_prefix("echo -e")?.trim();
                body.strip_prefix('"')?.strip_suffix('"').map(ansi_line)
            })
            .collect();
        let width = lines.iter().map(Line::width).max().unwrap_or(0) as u16;
        Banner { lines, width }
    })
}

/// Converts one string containing `ESC[...m` color sequences into a styled line.
fn ansi_line(s: &str) -> Line<'static> {
    // `echo -e` spells the escape character in several ways.
    let s = s.replace("\\033", "\x1b").replace("\\x1b", "\x1b").replace("\\e", "\x1b");
    let (mut spans, mut text, mut style) = (Vec::new(), String::new(), Style::new());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            let params: String = chars.by_ref().take_while(|&p| p != 'm').collect();
            if !text.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut text), style));
            }
            style = apply_sgr(style, &params);
        } else {
            text.push(c);
        }
    }
    if !text.is_empty() {
        spans.push(Span::styled(text, style));
    }
    Line::from(spans)
}

/// Applies the `;`-separated SGR codes (reset, bold, 16 foreground and 16 background colors).
fn apply_sgr(mut style: Style, params: &str) -> Style {
    for code in params.split(';') {
        let n: u8 = code.parse().unwrap_or(0); // an empty code means reset
        style = match n {
            0 => Style::new(),
            1 => style.add_modifier(Modifier::BOLD),
            30..=37 => style.fg(color(n - 30, false)),
            90..=97 => style.fg(color(n - 90, true)),
            39 => style.fg(Color::Reset),
            40..=47 => style.bg(color(n - 40, false)),
            100..=107 => style.bg(color(n - 100, true)),
            49 => style.bg(Color::Reset),
            _ => style,
        };
    }
    style
}

/// ANSI palette index (0-7), normal or bright, as a terminal-themed color.
fn color(i: u8, bright: bool) -> Color {
    match (i, bright) {
        (0, false) => Color::Black,
        (1, false) => Color::Red,
        (2, false) => Color::Green,
        (3, false) => Color::Yellow,
        (4, false) => Color::Blue,
        (5, false) => Color::Magenta,
        (6, false) => Color::Cyan,
        (_, false) => Color::Gray,
        (0, true) => Color::DarkGray,
        (1, true) => Color::LightRed,
        (2, true) => Color::LightGreen,
        (3, true) => Color::LightYellow,
        (4, true) => Color::LightBlue,
        (5, true) => Color::LightMagenta,
        (6, true) => Color::LightCyan,
        (_, true) => Color::White,
    }
}
