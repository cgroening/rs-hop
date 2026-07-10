//! Per-entry usage tracking (last-used time, open count) and the selected-repo
//! handoff file.
//!
//! Usage lives in the state directory, separate from the user's config, so the
//! hand-maintained `config.toml` stays clean. The selected-repo file is the
//! single line the shell wrapper reads to `cd` after hop exits.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::domain::error::{Error, Result};
use crate::util::fs::write_atomic;

/// Usage counters for one entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    /// Last-used time as unix seconds.
    pub last_used: Option<i64>,
    /// How often the entry was opened.
    pub open_count: u64,
}

/// The on-disk usage document.
#[derive(Debug, Default, Serialize, Deserialize)]
struct UsageDoc {
    #[serde(default)]
    entries: Vec<UsageEntry>,
}

/// One stored usage row.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct UsageEntry {
    path: String,
    last_used: Option<i64>,
    #[serde(default)]
    open_count: u64,
}

/// Loads usage keyed by path, returning empty on a missing or corrupt file.
pub fn load(path: &Path) -> HashMap<PathBuf, Usage> {
    let Ok(text) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(doc) = toml::from_str::<UsageDoc>(&text) else {
        return HashMap::new();
    };
    doc.entries
        .into_iter()
        .map(|entry| {
            (
                PathBuf::from(entry.path),
                Usage {
                    last_used: entry.last_used,
                    open_count: entry.open_count,
                },
            )
        })
        .collect()
}

/// Records an open of `repo_path`: stamps the current time and bumps the open
/// count in the usage file at `path`.
///
/// # Errors
/// Returns an error if the usage file cannot be written.
pub fn record(path: &Path, repo_path: &Path) -> Result<()> {
    let mut usage = load(path);
    let entry = usage.entry(repo_path.to_path_buf()).or_default();
    entry.last_used = Some(Utc::now().timestamp());
    entry.open_count += 1;
    save(path, &usage)
}

/// Writes the whole usage map back to `path`.
fn save(path: &Path, usage: &HashMap<PathBuf, Usage>) -> Result<()> {
    let mut entries: Vec<UsageEntry> = usage
        .iter()
        .map(|(path, usage)| UsageEntry {
            path: path.to_string_lossy().into_owned(),
            last_used: usage.last_used,
            open_count: usage.open_count,
        })
        .collect();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    let doc = UsageDoc { entries };
    let text = toml::to_string_pretty(&doc)
        .map_err(|e| Error::invalid(format!("serialise usage: {e}")))?;
    write_atomic(path, &text, "usage state")
}

/// Writes `repo_path` as the single-line selected-repo handoff at `dest`.
///
/// # Errors
/// Returns an error if the file cannot be written.
pub fn write_selected_repo(dest: &Path, repo_path: &Path) -> Result<()> {
    let line = format!("{}\n", repo_path.to_string_lossy());
    write_atomic(dest, &line, "selected-repo file")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_bumps_count_and_stamps_time() {
        let dir = std::env::temp_dir()
            .join(format!("hop-usage-test-{}", std::process::id()));
        let file = dir.join("usage.toml");
        let repo = PathBuf::from("/code/hop");
        record(&file, &repo).unwrap();
        record(&file, &repo).unwrap();
        let usage = load(&file);
        let entry = usage.get(&repo).unwrap();
        assert_eq!(entry.open_count, 2);
        assert!(entry.last_used.is_some());
        let _ = fs::remove_dir_all(&dir);
    }
}
