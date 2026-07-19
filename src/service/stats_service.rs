//! Gathers project statistics in the background, mirroring `status_service`.
//!
//! Two workers, not one, because the two column sets need different halves: the
//! `Code` columns want a source walk and a size measurement, the `Activity`
//! columns want only `git`. Bundling them would make a switch to `Activity`
//! pay for a full tokei pass over every source file whose numbers no column
//! ever shows.
//!
//! Progress is reported per entry rather than per file: tokei's `for_each_fn`
//! is a plain function pointer and cannot capture a channel to send from.
//!
//! Each worker stops as soon as its receiver is dropped, so switching column
//! sets mid-run simply abandons the old pass.
//!
//! The service also owns the on-disk statistics cache, which is a display
//! accelerator: it seeds the columns at once and a finished worker overwrites
//! it. There is no invalidation.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use tokei::{Config as TokeiConfig, Languages};

use crate::domain::error::Result;
use crate::domain::stats::{CodeEntry, CodeStats, GitStats, LangCount};
use crate::storage::git_client::GitClient;
use crate::storage::stats_cache;
use crate::util::diskusage;

pub use crate::storage::stats_cache::StatsCache;

/// Loads the cached statistics from `path`, so the columns show numbers before
/// a worker reports. A missing or corrupt cache yields an empty one.
pub fn load_cache(path: &Path) -> StatsCache {
    stats_cache::load(path)
}

/// Writes the gathered statistics to the cache at `path`.
///
/// # Errors
///
/// Returns an error if the cache file cannot be written.
pub fn save_cache(path: &Path, cache: &StatsCache) -> Result<()> {
    stats_cache::save(path, cache)
}

/// A message from the code-statistics worker.
pub enum CodeUpdate {
    /// The worker has begun counting this path.
    Started {
        /// The entry path now being counted.
        path: PathBuf,
    },
    /// The line counts and size for this path have been gathered.
    Done {
        /// The entry path the statistics belong to.
        path: PathBuf,
        /// The gathered code and disk statistics.
        stats: Box<CodeEntry>,
    },
}

/// A message from the git-statistics worker.
pub enum GitStatsUpdate {
    /// The worker has begun querying this path.
    Started {
        /// The entry path now being queried.
        path: PathBuf,
    },
    /// The history statistics for this path have been gathered.
    Done {
        /// The entry path the statistics belong to.
        path: PathBuf,
        /// The gathered history statistics.
        stats: GitStats,
    },
}

/// Spawns a worker that counts lines and measures size for each path, streaming
/// one [`CodeUpdate::Started`] and one [`CodeUpdate::Done`] per entry.
///
/// `exclude_dirs` are the configured build-directory name prefixes; they are
/// applied on top of the `.gitignore` rules tokei honours by default.
pub fn spawn_code_stats(
    paths: Vec<PathBuf>,
    exclude_dirs: Vec<String>,
) -> Receiver<CodeUpdate> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for path in paths {
            let started = CodeUpdate::Started { path: path.clone() };
            if tx.send(started).is_err() {
                return;
            }
            let stats = Box::new(CodeEntry {
                code: count_lines(&path, &exclude_dirs),
                disk: diskusage::measure(&path, &exclude_dirs),
            });
            if tx.send(CodeUpdate::Done { path, stats }).is_err() {
                return;
            }
        }
    });
    rx
}

/// Spawns a worker that queries the repository history for each path.
pub fn spawn_git_stats(
    client: Arc<dyn GitClient>,
    paths: Vec<PathBuf>,
) -> Receiver<GitStatsUpdate> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for path in paths {
            let started = GitStatsUpdate::Started { path: path.clone() };
            if tx.send(started).is_err() {
                return;
            }
            let stats = client.stats(&path);
            if tx.send(GitStatsUpdate::Done { path, stats }).is_err() {
                return;
            }
        }
    });
    rx
}

/// Counts the lines under `path` per language.
///
/// tokei's defaults already give the wanted semantics: `hidden` is off, so
/// `.git` and other dot directories are skipped, and `no_ignore_vcs` is off, so
/// `.gitignore` is honoured. `no_ignore_parent` is turned on so an ignoring
/// parent directory cannot make a repository look empty. `exclude_dirs` become
/// directory globs on top of that.
fn count_lines(path: &PathBuf, exclude_dirs: &[String]) -> CodeStats {
    let globs = exclude_globs(exclude_dirs);
    let ignored: Vec<&str> = globs.iter().map(String::as_str).collect();
    let config = TokeiConfig {
        no_ignore_parent: Some(true),
        ..TokeiConfig::default()
    };
    let mut languages = Languages::new();
    languages.get_statistics(std::slice::from_ref(path), &ignored, &config);
    // Sum the per-file reports away immediately: on a monorepo, keeping them is
    // the memory hotspot, and no caller ever looks at an individual file.
    let counts = languages
        .into_iter()
        .map(|(kind, language)| {
            (
                kind.name().to_string(),
                LangCount {
                    code: language.code,
                    comments: language.comments,
                    blanks: language.blanks,
                    files: language.reports.len(),
                },
            )
        })
        .filter(|(_, count)| count.files > 0)
        .collect();
    CodeStats { languages: counts }
}

/// Translates hop's name-prefix exclusions into the directory globs tokei's
/// ignore layer expects: `target` becomes `target*/`, which matches at any
/// depth (no leading slash), keeps the prefix rule (`*`) and applies only to
/// directories (the trailing slash), just as `archive::is_excluded` does.
fn exclude_globs(exclude_dirs: &[String]) -> Vec<String> {
    exclude_dirs
        .iter()
        .filter(|dir| !dir.is_empty())
        .map(|dir| format!("{dir}*/"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclude_globs_keep_the_prefix_rule_and_target_directories() {
        assert_eq!(
            exclude_globs(&["target".to_string(), "node_modules".to_string()]),
            vec!["target*/".to_string(), "node_modules*/".to_string()]
        );
        // An empty entry must not become `*/`, which would exclude everything.
        assert!(exclude_globs(&[String::new()]).is_empty());
    }
}
