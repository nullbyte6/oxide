//! Banner art: `banner.sh` is embedded at compile time and its colored `echo -e "..."` lines
//! are converted into monochrome Ratatui lines. The script is never executed.
//!
//! The art's colors are flattened to its brightness: no color is set, so the text takes the
//! interface's single ink color, and light/dark shades become bold/dim. Backgrounds are dropped.

use std::sync::OnceLock;

use ratatui::{
    style::{Modifier, Style},
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

/// Applies the `;`-separated SGR codes, keeping only reset, bold and the brightness of the foreground.
fn apply_sgr(mut style: Style, params: &str) -> Style {
    for code in params.split(';') {
        let n: u8 = code.parse().unwrap_or(0); // an empty code means reset
        style = match n {
            0 | 39 => Style::new(),
            1 | 91..=97 => Style::new().add_modifier(Modifier::BOLD), // bold and bright colors
            30 | 34 | 90 => Style::new().add_modifier(Modifier::DIM), // dark colors: black, blue, bright black
            31..=33 | 35..=37 => Style::new(),
            _ => style, // backgrounds and unsupported codes
        };
    }
    style
}
