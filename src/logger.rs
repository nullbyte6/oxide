//! Persistent log file (`<state dir>/oxide.log`) with local-time timestamps.

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::PathBuf,
    process::Command,
    sync::OnceLock,
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

/// The local timezone: offset from UTC in seconds and its abbreviation (e.g. `CEST`).
struct Zone {
    offset: i64,
    name: String,
}

/// Local timezone, read once from the system's `date` (it honors `TZ` and `/etc/localtime`).
/// It is fixed for the whole run, so a daylight-saving change while Oxide is open is not
/// picked up until the next start. Falls back to UTC when `date` is unavailable.
fn zone() -> &'static Zone {
    static ZONE: OnceLock<Zone> = OnceLock::new();
    ZONE.get_or_init(|| {
        let utc = || Zone { offset: 0, name: "UTC".to_string() };
        let Ok(out) = Command::new("date").arg("+%z %Z").output() else { return utc() };
        let text = String::from_utf8_lossy(&out.stdout);
        let mut parts = text.split_whitespace();
        let (Some(off), Some(name)) = (parts.next(), parts.next()) else { return utc() };
        // `%z` looks like +0200 or -0530.
        let (Some(sign), Ok(h), Ok(m)) = (off.get(..1), off.get(1..3).unwrap_or("").parse::<i64>(), off.get(3..5).unwrap_or("").parse::<i64>()) else {
            return utc();
        };
        let secs = h * 3600 + m * 60;
        Zone { offset: if sign == "-" { -secs } else { secs }, name: name.to_string() }
    })
}

/// Abbreviation of the local timezone, for panel titles.
pub fn zone_name() -> &'static str {
    &zone().name
}

/// Seconds since the Unix epoch, shifted into local time.
fn local_secs() -> i64 {
    let utc = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    utc + zone().offset
}

/// `HH:MM:SS` of a local-time second count.
fn hms(secs: i64) -> String {
    let s = secs.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
}

/// `HH:MM:SS` in local time for the in-app log panel.
pub fn clock() -> String {
    hms(local_secs())
}

/// ISO-8601 local timestamp with its UTC offset, e.g. `2026-10-12T17:45:00+02:00`.
fn timestamp() -> String {
    let now = local_secs(); // read once so the date and the time always agree
    let (y, m, d) = civil_from_days(now.div_euclid(86_400));
    let off = zone().offset;
    let sign = if off < 0 { '-' } else { '+' };
    format!("{y:04}-{m:02}-{d:02}T{}{sign}{:02}:{:02}", hms(now), off.abs() / 3600, off.abs() % 3600 / 60)
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
