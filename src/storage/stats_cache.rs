//! TOML-backed cache of the per-entry statistics, keyed by entry path.
//!
//! Purely a display accelerator: switching to a statistics column set shows the
//! last known numbers at once while the background workers recompute and
//! overwrite them. Nothing here decides whether a value is stale, so there is
//! no invalidation to get wrong.
//!
//! The two families are stored in two independent tables, because they are
//! gathered by two independent workers: a user who never opens the `Code`
//! columns never writes a line count. Loading is best-effort - a missing or
//! corrupt cache yields empty maps.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::domain::error::{Error, Result};
use crate::domain::stats::{
    CodeEntry, CodeStats, DiskStats, GitStats, LangCount,
};
use crate::util::fs::write_atomic;

/// Code and size statistics keyed by entry path.
pub type CodeCache = HashMap<PathBuf, CodeEntry>;

/// Git history statistics keyed by entry path.
pub type GitCache = HashMap<PathBuf, GitStats>;

/// Both halves of the cached statistics.
#[derive(Debug, Default, Clone)]
pub struct StatsCache {
    /// Lines of code and size on disk, per entry.
    pub code: CodeCache,
    /// Repository history, per git entry.
    pub git: GitCache,
}

/// The on-disk cache document: two independent tables.
#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheDoc {
    #[serde(default)]
    code: Vec<CodeRow>,
    #[serde(default)]
    git: Vec<GitRow>,
}

/// One entry's code and disk statistics.
#[derive(Debug, Serialize, Deserialize)]
struct CodeRow {
    path: String,
    #[serde(default)]
    languages: Vec<LangRow>,
    #[serde(default)]
    total_bytes: u64,
    #[serde(default)]
    git_bytes: u64,
    #[serde(default)]
    excluded_bytes: u64,
    #[serde(default)]
    files: u64,
}

/// One language's counts within an entry.
#[derive(Debug, Serialize, Deserialize)]
struct LangRow {
    name: String,
    code: usize,
    comments: usize,
    blanks: usize,
    files: usize,
}

/// One entry's git history statistics. Every count fits in a TOML integer
/// (signed 64-bit), so none of them needs the hex encoding `zip_cache` uses.
#[derive(Debug, Serialize, Deserialize)]
struct GitRow {
    path: String,
    #[serde(default)]
    commits: u64,
    #[serde(default)]
    contributors: u64,
    #[serde(default)]
    branches: u64,
    #[serde(default)]
    tags: u64,
    #[serde(default)]
    first_commit: Option<i64>,
    #[serde(default)]
    last_commit: Option<i64>,
    #[serde(default)]
    commits_recent: u64,
}

/// Loads the cache from `path`, returning empty maps when it is missing or
/// cannot be parsed.
pub fn load(path: &Path) -> StatsCache {
    let Ok(text) = fs::read_to_string(path) else {
        return StatsCache::default();
    };
    let Ok(doc) = toml::from_str::<CacheDoc>(&text) else {
        return StatsCache::default();
    };
    StatsCache {
        code: doc.code.into_iter().map(into_code).collect(),
        git: doc.git.into_iter().map(into_git).collect(),
    }
}

/// Writes `cache` to `path`, creating the state directory if needed.
///
/// # Errors
///
/// Returns an error if the directory or file cannot be written.
pub fn save(path: &Path, cache: &StatsCache) -> Result<()> {
    let mut code: Vec<CodeRow> = cache.code.iter().map(from_code).collect();
    let mut git: Vec<GitRow> = cache.git.iter().map(from_git).collect();
    // Stable on-disk order for clean diffs.
    code.sort_by(|a, b| a.path.cmp(&b.path));
    git.sort_by(|a, b| a.path.cmp(&b.path));
    let text = toml::to_string_pretty(&CacheDoc { code, git })
        .map_err(|e| Error::invalid(format!("serialise stats cache: {e}")))?;
    write_atomic(path, &text, "stats cache")
}

/// Rebuilds a cached [`CodeEntry`] from its on-disk row.
fn into_code(row: CodeRow) -> (PathBuf, CodeEntry) {
    let languages = row
        .languages
        .into_iter()
        .map(|lang| {
            (
                lang.name,
                LangCount {
                    code: lang.code,
                    comments: lang.comments,
                    blanks: lang.blanks,
                    files: lang.files,
                },
            )
        })
        .collect();
    let entry = CodeEntry {
        code: CodeStats { languages },
        disk: DiskStats {
            total_bytes: row.total_bytes,
            git_bytes: row.git_bytes,
            excluded_bytes: row.excluded_bytes,
            files: row.files,
        },
    };
    (PathBuf::from(row.path), entry)
}

/// Rebuilds a cached [`GitStats`] from its on-disk row.
fn into_git(row: GitRow) -> (PathBuf, GitStats) {
    let stats = GitStats {
        commits: row.commits,
        contributors: row.contributors,
        branches: row.branches,
        tags: row.tags,
        first_commit: row.first_commit,
        last_commit: row.last_commit,
        commits_recent: row.commits_recent,
    };
    (PathBuf::from(row.path), stats)
}

/// Flattens a [`CodeEntry`] into its on-disk row.
fn from_code((path, entry): (&PathBuf, &CodeEntry)) -> CodeRow {
    CodeRow {
        path: path.to_string_lossy().into_owned(),
        languages: entry
            .code
            .languages
            .iter()
            .map(|(name, count)| LangRow {
                name: name.clone(),
                code: count.code,
                comments: count.comments,
                blanks: count.blanks,
                files: count.files,
            })
            .collect(),
        total_bytes: entry.disk.total_bytes,
        git_bytes: entry.disk.git_bytes,
        excluded_bytes: entry.disk.excluded_bytes,
        files: entry.disk.files,
    }
}

/// Flattens a [`GitStats`] into its on-disk row.
fn from_git((path, stats): (&PathBuf, &GitStats)) -> GitRow {
    GitRow {
        path: path.to_string_lossy().into_owned(),
        commits: stats.commits,
        contributors: stats.contributors,
        branches: stats.branches,
        tags: stats.tags,
        first_commit: stats.first_commit,
        last_commit: stats.last_commit,
        commits_recent: stats.commits_recent,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn temp(tag: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("hop-stats-cache-{tag}-{}.toml", std::process::id()))
    }

    fn sample() -> StatsCache {
        let languages = BTreeMap::from([(
            "Rust".to_string(),
            LangCount {
                code: 100,
                comments: 10,
                blanks: 5,
                files: 3,
            },
        )]);
        StatsCache {
            code: HashMap::from([(
                PathBuf::from("/code/hop"),
                CodeEntry {
                    code: CodeStats { languages },
                    disk: DiskStats {
                        total_bytes: 2048,
                        git_bytes: 1024,
                        excluded_bytes: 512,
                        files: 9,
                    },
                },
            )]),
            git: HashMap::from([(
                PathBuf::from("/code/hop"),
                GitStats {
                    commits: 418,
                    contributors: 3,
                    branches: 4,
                    tags: 7,
                    first_commit: Some(1_600_000_000),
                    last_commit: Some(1_700_000_000),
                    commits_recent: 12,
                },
            )]),
        }
    }

    #[test]
    fn save_then_load_round_trips_both_tables() {
        let path = temp("round");
        save(&path, &sample()).unwrap();
        let loaded = load(&path);
        assert_eq!(loaded.code, sample().code);
        assert_eq!(loaded.git, sample().git);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn the_two_tables_are_independent() {
        // A user who never opens the Code columns writes no line counts, and
        // the git half must still load.
        let path = temp("half");
        let mut cache = sample();
        cache.code.clear();
        save(&path, &cache).unwrap();
        let loaded = load(&path);
        assert!(loaded.code.is_empty());
        assert_eq!(loaded.git.len(), 1);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_missing_or_corrupt_file_yields_an_empty_cache() {
        let missing = temp("missing");
        let _ = fs::remove_file(&missing);
        assert!(load(&missing).code.is_empty());

        let corrupt = temp("corrupt");
        fs::write(&corrupt, "this is not toml {{{").unwrap();
        assert!(load(&corrupt).git.is_empty());
        let _ = fs::remove_file(&corrupt);
    }
}
