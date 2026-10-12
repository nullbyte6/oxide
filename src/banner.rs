//! Banner art: `banner.txt` is embedded at compile time and shown above the dashboard as plain text.
//!
//! The art takes the interface's styling: bold in the single ink color, or in the accent color when
//! colors are on (see `ui::draw`).

use std::sync::OnceLock;

use ratatui::text::Line;

const SOURCE: &str = include_str!("banner.txt");

pub struct Banner {
    pub lines: Vec<Line<'static>>,
    /// Width in terminal columns of the widest line.
    pub width: u16,
}

/// The parsed banner; parsed once, on first use.
pub fn get() -> &'static Banner {
    static BANNER: OnceLock<Banner> = OnceLock::new();
    BANNER.get_or_init(|| {
        // Trailing spaces would only widen the art, and blank lines around it would waste rows.
        let mut rows: Vec<&str> = SOURCE.lines().map(str::trim_end).collect();
        while rows.last().is_some_and(|l| l.is_empty()) {
            rows.pop();
        }
        let skip = rows.iter().take_while(|l| l.is_empty()).count();
        let lines: Vec<Line<'static>> = rows[skip..].iter().map(|l| Line::from(l.to_string())).collect();
        let width = lines.iter().map(Line::width).max().unwrap_or(0) as u16;
        Banner { lines, width }
    })
}
