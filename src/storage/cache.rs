//! TOML-backed cache of gathered git status, keyed by path.
//!
//! Reading it on start lets the list show last-known status immediately while a
//! fresh refresh runs in the background. Loading is best-effort: a missing or
//! corrupt cache yields an empty result rather than an error.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::domain::error::{Error, Result};
use crate::domain::repo::GitInfo;

/// The on-disk cache document.
#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheDoc {
    generated_at: Option<String>,
    fetched_at: Option<String>,
    #[serde(default)]
    entries: Vec<CacheEntry>,
}

/// One cached status, with the structured counts (the display string is rebuilt
/// at render time).
#[derive(Debug, Serialize, Deserialize)]
struct CacheEntry {
    path: String,
    valid: bool,
    error: Option<String>,
    current_branch_name: Option<String>,
    changes: Option<u32>,
    ahead: Option<u32>,
    behind: Option<u32>,
    github_repo_name: Option<String>,
}

/// A loaded cache: when it was generated, when the remote was last fetched, and
/// the status per path.
#[derive(Debug, Default)]
pub struct GitInfoCache {
    /// When the cache was last written, if recorded.
    pub generated_at: Option<DateTime<Local>>,
    /// When `git fetch` was last run for the entries, if ever.
    pub fetched_at: Option<DateTime<Local>>,
    /// Gathered status keyed by entry path.
    pub infos: HashMap<PathBuf, GitInfo>,
}

/// Loads the cache from `path`, returning an empty cache when it is missing or
/// cannot be parsed.
pub fn load(path: &Path) -> GitInfoCache {
    let Ok(text) = fs::read_to_string(path) else {
        return GitInfoCache::default();
    };
    let Ok(doc) = toml::from_str::<CacheDoc>(&text) else {
        return GitInfoCache::default();
    };
    let generated_at = doc.generated_at.as_deref().and_then(parse_timestamp);
    let fetched_at = doc.fetched_at.as_deref().and_then(parse_timestamp);
    let infos = doc
        .entries
        .into_iter()
        .map(|entry| (PathBuf::from(&entry.path), entry.into_info()))
        .collect();
    GitInfoCache {
        generated_at,
        fetched_at,
        infos,
    }
}

/// Writes the gathered `infos` to `path`, stamping the current time as the
/// generation time and recording `fetched_at` (when the remote was last
/// fetched, preserved across non-fetching refreshes by the caller).
///
/// # Errors
/// Returns an error if the directory or file cannot be written.
pub fn save(
    path: &Path,
    infos: &[(PathBuf, GitInfo)],
    fetched_at: Option<DateTime<Local>>,
) -> Result<()> {
    let doc = CacheDoc {
        generated_at: Some(Local::now().to_rfc3339()),
        fetched_at: fetched_at.map(|at| at.to_rfc3339()),
        entries: infos
            .iter()
            .map(|(path, info)| CacheEntry::from_info(path, info))
            .collect(),
    };
    let text = toml::to_string_pretty(&doc)
        .map_err(|e| Error::invalid(format!("serialise cache: {e}")))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| Error::io("create state directory", e))?;
    }
    fs::write(path, text).map_err(|e| Error::io("write git-info cache", e))
}

/// Parses an RFC 3339 timestamp into local time.
fn parse_timestamp(text: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|dt| dt.with_timezone(&Local))
}

impl CacheEntry {
    /// Builds a cache entry from a path and gathered info.
    fn from_info(path: &Path, info: &GitInfo) -> Self {
        CacheEntry {
            path: path.to_string_lossy().into_owned(),
            valid: info.valid,
            error: info.error.clone(),
            current_branch_name: info.current_branch_name.clone(),
            changes: info.changes,
            ahead: info.ahead,
            behind: info.behind,
            github_repo_name: info.github_repo_name.clone(),
        }
    }

    /// Rebuilds a [`GitInfo`] (without a verbatim status string).
    fn into_info(self) -> GitInfo {
        GitInfo {
            valid: self.valid,
            error: self.error,
            current_branch_name: self.current_branch_name,
            changes: self.changes,
            ahead: self.ahead,
            behind: self.behind,
            github_repo_name: self.github_repo_name,
            raw_status: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_then_load_round_trips() {
        let dir = std::env::temp_dir()
            .join(format!("hop-cache-test-{}", std::process::id()));
        let file = dir.join("cache.toml");
        let info = GitInfo {
            valid: true,
            current_branch_name: Some("main".to_string()),
            changes: Some(3),
            ahead: Some(1),
            behind: Some(0),
            github_repo_name: Some("owner/repo".to_string()),
            ..GitInfo::default()
        };
        let path = PathBuf::from("/code/hop");
        let fetched = Local::now();
        save(&file, &[(path.clone(), info.clone())], Some(fetched)).unwrap();
        let loaded = load(&file);
        assert!(loaded.generated_at.is_some());
        assert!(loaded.fetched_at.is_some());
        assert_eq!(loaded.infos.get(&path), Some(&info));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_yields_empty_cache() {
        let loaded = load(Path::new("/nonexistent/hop-cache.toml"));
        assert!(loaded.infos.is_empty());
        assert!(loaded.generated_at.is_none());
    }
}
