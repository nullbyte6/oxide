//! Persistent log file (`<state dir>/oxide.log`) with UTC timestamps.

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::config;

/// The log file is rotated to `oxide.log.1` once it grows past this size.
const MAX_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, PartialEq)]
pub enum Level {
    Info,
    Ok,
    Warn,
    Error,
}

impl Level {
    fn tag(self) -> &'static str {
        match self {
            Level::Info => "INFO",
            Level::Ok => "OK",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

pub struct Logger {
    file: Option<File>,
}

pub fn log_path() -> PathBuf {
    config::state_dir().join("oxide.log")
}

impl Logger {
    /// Opens the log file; if that fails Oxide keeps running with in-app logs only.
    pub fn open() -> Logger {
        let path = log_path();
        let _ = fs::create_dir_all(config::state_dir());
        if fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
            let _ = fs::rename(&path, path.with_extension("log.1"));
        }
        Logger { file: OpenOptions::new().create(true).append(true).open(path).ok() }
    }

    pub fn write(&mut self, level: Level, msg: &str) {
        if let Some(f) = &mut self.file {
            let _ = writeln!(f, "{} [{:<5}] {msg}", timestamp(), level.tag());
        }
    }
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// `HH:MM:SS` (UTC) for the in-app log panel.
pub fn clock() -> String {
    let s = now_secs().rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
}

/// ISO-8601 UTC timestamp, e.g. `2026-10-12T15:45:00Z`.
fn timestamp() -> String {
    let secs = now_secs();
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}T{}Z", clock())
}

/// Converts days since 1970-01-01 to a (year, month, day) civil date.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}
