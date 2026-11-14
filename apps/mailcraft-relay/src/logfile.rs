// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The relay's log: `relay.log` in its folder (the one before is kept as `relay.1.log` once it
//! passes a megabyte), and the console too when there is one. The relay's own records from
//! `info` up, the rest from `warn`. No secret is ever written: the network crate hides them from
//! what it logs, and the relay logs addresses and counts.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// When the log is moved aside.
const MOST: u64 = 1 << 20;

struct FileLog {
    path: PathBuf,
    file: Mutex<Option<std::fs::File>>,
    console: bool,
}

impl log::Log for FileLog {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        let ours = m.target().starts_with("mailcraft_relay");
        m.level() <= if ours { log::Level::Info } else { log::Level::Warn }
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let line = format!("{} {:<5} {}\n", stamp(now), record.level(), record.args());
        if self.console {
            eprint!("{line}");
        }
        let Ok(mut file) = self.file.lock() else { return };
        if file.as_ref().and_then(|f| f.metadata().ok()).is_some_and(|m| m.len() > MOST) {
            *file = None;
            let _ = std::fs::rename(&self.path, self.path.with_file_name("relay.1.log"));
        }
        if file.is_none() {
            *file = std::fs::OpenOptions::new().create(true).append(true).open(&self.path).ok();
        }
        if let Some(f) = file.as_mut() {
            let _ = f.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {}
}

/// `YYYY-MM-DD hh:mm:ss` (UTC) for `secs` since 1970.
pub fn stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// Start the log in `folder`; `console`: the console gets it too.
pub fn start(folder: &Path, console: bool) {
    let _ = std::fs::create_dir_all(folder);
    static LOGGER: std::sync::OnceLock<FileLog> = std::sync::OnceLock::new();
    let logger = LOGGER.get_or_init(|| FileLog { path: folder.join("relay.log"), file: Mutex::new(None), console });
    if log::set_logger(logger).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

/// The last `n` lines of the log in `folder`.
pub fn tail(folder: &Path, n: usize) -> Vec<String> {
    let text = std::fs::read_to_string(folder.join("relay.log")).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines.iter().skip(lines.len().saturating_sub(n)).map(|l| l.to_string()).collect()
}
