//! Minimal `log` backend writing to an optional file.
//!
//! A TUI owns the terminal, so diagnostics must not go to stderr (it would
//! corrupt the alternate screen). When a log file is configured, records are
//! appended there; otherwise logging is a no-op.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};

/// A file logger; silent when no file is configured.
struct FileLogger {
    file: Option<Mutex<File>>,
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Debug
    }

    fn log(&self, record: &Record) {
        let Some(file) = &self.file else {
            return;
        };
        if let Ok(mut handle) = file.lock() {
            let _ = writeln!(
                handle,
                "[{:<5}] {}: {}",
                record.level(),
                record.target(),
                record.args()
            );
        }
    }

    fn flush(&self) {
        if let Some(file) = &self.file
            && let Ok(mut handle) = file.lock()
        {
            let _ = handle.flush();
        }
    }
}

/// Installs the logger at `level`, appending to `file` when given.
///
/// Best-effort: a failure to open the file is reported via the returned result,
/// but the app should treat logging as non-fatal.
///
/// # Errors
///
/// Returns an error if the log file cannot be opened or the logger cannot be
/// installed.
pub fn init(level: LevelFilter, file: Option<&Path>) -> Result<(), String> {
    let sink = match file {
        Some(path) => {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let handle = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|e| format!("cannot open log file: {e}"))?;
            Some(Mutex::new(handle))
        }
        None => None,
    };
    let logger = FileLogger { file: sink };
    log::set_boxed_logger(Box::new(logger))
        .map_err(|e| format!("cannot install logger: {e}"))?;
    log::set_max_level(level);
    Ok(())
}
