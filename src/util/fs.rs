//! Atomic file writes for the config and the state files.
//!
//! A plain `fs::write` truncates the file first, so a crash or a full disk
//! midway leaves it half-written. hop keeps its settings *and* every `[[repos]]`
//! entry in one `config.toml`, so that failure mode loses the user's entries.
//! [`write_atomic`] builds a sibling temp file and renames it over the target,
//! which is atomic on the same filesystem: readers see either the old file or
//! the new one, never a truncated one.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::domain::error::{Error, Result};

/// Writes `contents` to `path` by way of a sibling temp file and a rename,
/// creating the parent directory when it is missing. `what` names the file in
/// the error, so a failure says which write went wrong.
///
/// # Errors
/// Returns an error when the parent directory, the temp file or the rename
/// fails.
pub fn write_atomic(path: &Path, contents: &str, what: &str) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            Error::io(format!("create {what} directory"), error)
        })?;
    }
    let temp = temp_path(path);
    // A failed write leaves the temp file behind rather than the target
    // truncated; the next successful write replaces it.
    fs::write(&temp, contents)
        .map_err(|error| Error::io(format!("write {what}"), error))?;
    replace_atomically(&temp, path)
        .map_err(|error| Error::io(format!("replace {what}"), error))
}

/// The sibling temp path used while writing, in the same directory so the final
/// rename stays on one filesystem and is therefore atomic.
fn temp_path(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_os_string();
    name.push(".tmp");
    PathBuf::from(name)
}

/// Renames `tmp` over `dest`. On Unix this atomically replaces an existing file;
/// where rename refuses an existing target, it removes it first.
fn replace_atomically(tmp: &Path, dest: &Path) -> io::Result<()> {
    match fs::rename(tmp, dest) {
        Ok(()) => Ok(()),
        Err(_) if dest.exists() => {
            fs::remove_file(dest)?;
            fs::rename(tmp, dest)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory for one test, removed by the caller.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("hop-fs-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn writes_a_new_file_and_creates_its_directory() {
        let dir = temp_dir("new");
        let file = dir.join("nested").join("config.toml");
        write_atomic(&file, "a = 1", "config").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "a = 1");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn replaces_an_existing_file_and_leaves_no_temp_behind() {
        let dir = temp_dir("replace");
        let file = dir.join("config.toml");
        write_atomic(&file, "old", "config").unwrap();
        write_atomic(&file, "new", "config").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "new");
        assert!(!temp_path(&file).exists(), "the temp file is renamed away");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_previous_content_survives_a_failed_write() {
        // The temp file takes the hit, not the target: writing into a path whose
        // temp sibling cannot be created leaves the old file intact.
        let dir = temp_dir("survive");
        let file = dir.join("config.toml");
        write_atomic(&file, "keep me", "config").unwrap();
        // A directory at the temp path makes `fs::write` to it fail.
        fs::create_dir_all(temp_path(&file)).unwrap();
        assert!(write_atomic(&file, "lost", "config").is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "keep me");
        let _ = fs::remove_dir_all(&dir);
    }
}
