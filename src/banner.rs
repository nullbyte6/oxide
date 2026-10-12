//! Banner art: `banner.sh` is embedded at compile time and its colored `echo -e "..."` lines
//! are converted into Ratatui lines. The script is never executed.
//!
//! Two versions are built. The monochrome one flattens the art's colors to its brightness: no color
//! is set, so the text takes the interface's single ink color, and light/dark shades become
//! bold/dim; backgrounds are dropped. The color one keeps the art's own foreground and background colors.

use std::sync::OnceLock;

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

const SOURCE: &str = include_str!("banner.sh");

pub struct Banner {
    pub lines: Vec<Line<'static>>,
    /// The same art with its original colors.
    pub color_lines: Vec<Line<'static>>,
    /// Width in terminal columns of the widest line.
    pub width: u16,
}

/// The parsed banner; parsed once, on first use.
pub fn get() -> &'static Banner {
    static BANNER: OnceLock<Banner> = OnceLock::new();
    BANNER.get_or_init(|| {
        // Only `echo -e "<text>"` lines are art; anything else in the script is ignored.
        let art: Vec<&str> = SOURCE
            .lines()
            .filter_map(|l| {
                let body = l.trim().strip_prefix("echo -e")?.trim();
                body.strip_prefix('"')?.strip_suffix('"')
            })
            .collect();
        let lines: Vec<Line<'static>> = art.iter().map(|s| ansi_line(s, apply_sgr)).collect();
        let color_lines = art.iter().map(|s| ansi_line(s, apply_sgr_color)).collect();
        let width = lines.iter().map(Line::width).max().unwrap_or(0) as u16;
        Banner { lines, color_lines, width }
    })
}

/// Converts one string containing `ESC[...m` color sequences into a styled line; `sgr` decides how
/// each sequence changes the style.
fn ansi_line(s: &str, sgr: fn(Style, &str) -> Style) -> Line<'static> {
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
            style = sgr(style, &params);
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

/// Applies the `;`-separated SGR codes keeping the art's colors: the 16 ANSI foregrounds and
/// backgrounds, bold and reset.
fn apply_sgr_color(mut style: Style, params: &str) -> Style {
    for code in params.split(';') {
        let n: u8 = code.parse().unwrap_or(0); // an empty code means reset
        style = match n {
            0 => Style::new(),
            1 => style.add_modifier(Modifier::BOLD),
            30..=37 | 90..=97 => style.fg(ansi(n)),
            40..=47 | 100..=107 => style.bg(ansi(n - 10)),
            39 => style.fg(Color::Reset),
            49 => style.bg(Color::Reset),
            _ => style,
        };
    }
    style
}

/// The ANSI color for a foreground code (`30`-`37`, `90`-`97`).
fn ansi(code: u8) -> Color {
    match code {
        30 => Color::Black,
        31 => Color::Red,
        32 => Color::Green,
        33 => Color::Yellow,
        34 => Color::Blue,
        35 => Color::Magenta,
        36 => Color::Cyan,
        37 => Color::Gray,
        90 => Color::DarkGray,
        91 => Color::LightRed,
        92 => Color::LightGreen,
        93 => Color::LightYellow,
        94 => Color::LightBlue,
        95 => Color::LightMagenta,
        96 => Color::LightCyan,
        _ => Color::White,
    }
}
