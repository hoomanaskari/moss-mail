//! A trace of each panic, so a background task that died leaves a clue: when, where in the code,
//! and its message when that is fixed text. A message built from values (which could hold mail,
//! an address or a secret) is left out. The log is `crash.log` in the data folder, only you can
//! read it, and it keeps the last 256 KB.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

/// The log's file name in the data folder.
pub const FILE: &str = "crash.log";

/// How much of the log is kept.
const KEEP: usize = 256 * 1024;

/// Logs every panic from now on to `path`, then does what was done with panics before.
pub fn install(path: PathBuf) {
    // Two threads that panic at once take turns, so neither entry is lost.
    static WRITING: Mutex<()> = Mutex::new(());
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let place = info.location().map_or_else(String::new, |l| {
            format!("{}:{}:{}", l.file(), l.line(), l.column())
        });
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .unwrap_or("(message left out: it may hold mail)");
        let writing = WRITING.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = append(&path, &format!("{} panic at {place}: {message}\n", utc(at)));
        drop(writing);
        previous(info);
    }));
}

/// Adds `entry` to the log at `path`, dropping its oldest whole entries past [`KEEP`].
fn append(path: &Path, entry: &str) -> io::Result<()> {
    let mut log = match std::fs::read(path) {
        Ok(log) => log,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error),
    };
    log.extend_from_slice(entry.as_bytes());
    if log.len() > KEEP {
        let cut = log.len() - KEEP;
        let start = match log[cut - 1] {
            b'\n' => cut,
            _ => log[cut..]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(log.len(), |end| cut + end + 1),
        };
        log.drain(..start);
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    io::Write::write_all(&mut options.open(path)?, &log)
}

/// `secs` since 1970 as a UTC date and time: "2026-11-15 08:30:05 UTC".
fn utc(secs: u64) -> String {
    // Howard Hinnant's days-to-civil, for days since 1970.
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let time = secs % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        time / 3600,
        time % 3600 / 60,
        time % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_leaves_when_and_where_but_not_what_it_was_made_of() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        install(path.clone());
        let _ = std::panic::catch_unwind(|| panic!("the outbox stopped"));
        let address = "dana@acme.co";
        let _ = std::panic::catch_unwind(|| panic!("couldn't send to {address}"));
        // Back to the panics every other test expects.
        drop(std::panic::take_hook());
        let log = std::fs::read_to_string(&path).unwrap();
        let ours: Vec<_> = log
            .lines()
            .filter(|l| l.contains("crash_log.rs:"))
            .collect();
        assert_eq!(ours.len(), 2, "{log}");
        assert!(ours[0].ends_with(": the outbox stopped"), "{log}");
        assert!(
            ours[0].contains(" UTC panic at ") && ours[0].contains("src/crash_log.rs:"),
            "{log}"
        );
        assert!(
            ours[1].ends_with(": (message left out: it may hold mail)"),
            "{log}"
        );
        assert!(!log.contains(address), "{log}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn the_log_keeps_its_last_whole_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        append(&path, "first\n").unwrap();
        append(&path, "second\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\nsecond\n");
        let old = "an old entry\n".repeat(KEEP / 13 + 1);
        std::fs::write(&path, &old).unwrap();
        append(&path, "the newest\n").unwrap();
        let log = std::fs::read_to_string(&path).unwrap();
        assert!(log.len() <= KEEP && log.len() > KEEP - 30, "{}", log.len());
        assert!(log.starts_with("an old entry\n") && log.ends_with("an old entry\nthe newest\n"));
        // Cut right after an entry, every entry left is kept.
        std::fs::write(&path, "x\n".repeat(KEEP / 2)).unwrap();
        append(&path, "y\n").unwrap();
        let log = std::fs::read_to_string(&path).unwrap();
        assert_eq!(log.len(), KEEP);
        assert!(log.starts_with("x\nx\n") && log.ends_with("x\ny\n"));
        // A log it can't read isn't replaced.
        let folder = dir.path().join("folder");
        std::fs::create_dir(&folder).unwrap();
        assert!(append(&folder, "x\n").is_err());
    }

    #[test]
    fn times_are_utc() {
        assert_eq!(utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(utc(1_709_210_096), "2024-02-29 12:34:56 UTC");
        assert_eq!(utc(1_763_197_205), "2025-11-15 09:00:05 UTC");
    }
}
