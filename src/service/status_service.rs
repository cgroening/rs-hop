//! Gathers git status, synchronously for the CLI or in the background for the
//! TUI, and owns the on-disk status cache that seeds the list on start.
//!
//! The background refresh runs on one worker thread that walks the paths and
//! streams a [`StatusUpdate`] per entry, so the list stays responsive while
//! `git` runs. The synchronous variant is used by `hop list` and to seed the
//! cache.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use chrono::{DateTime, Local};

use crate::domain::error::Result;
use crate::domain::repo::GitInfo;
use crate::storage::cache;
use crate::storage::git_client::GitClient;

pub use crate::storage::cache::GitInfoCache;

/// A message from the background refresh: a path's work either started or
/// finished. `Started` lets the progress bar name the entry while git runs.
pub enum StatusUpdate {
    /// The worker has begun processing this path (before fetch/collect).
    Started {
        /// The entry path now being refreshed.
        path: PathBuf,
    },
    /// The status for this path has been gathered.
    Done {
        /// The entry path the status belongs to.
        path: PathBuf,
        /// The gathered git info.
        info: GitInfo,
    },
}

/// Loads the last gathered status from the cache at `path`, so the list can be
/// drawn before the background refresh reports. A missing or corrupt cache
/// yields an empty one.
pub fn load_cache(path: &Path) -> GitInfoCache {
    cache::load(path)
}

/// Writes the gathered `infos` to the cache at `path`, recording when the remote
/// was last fetched.
///
/// # Errors
/// Returns an error if the cache file cannot be written.
pub fn save_cache(
    path: &Path,
    infos: &[(PathBuf, GitInfo)],
    fetched_at: Option<DateTime<Local>>,
) -> Result<()> {
    cache::save(path, infos, fetched_at)
}

/// Gathers status for every path in order, optionally fetching first. Used by
/// the CLI and to build the initial cache.
pub fn collect_all(
    client: &dyn GitClient,
    paths: &[PathBuf],
    fetch: bool,
) -> Vec<(PathBuf, GitInfo)> {
    paths
        .iter()
        .map(|path| {
            if fetch {
                client.fetch(path);
            }
            (path.clone(), client.collect(path))
        })
        .collect()
}

/// Spawns a worker that gathers status for each path and streams the results.
/// The returned receiver yields one [`StatusUpdate`] per path, then closes when
/// the worker finishes. Dropping the receiver lets the worker run to completion
/// harmlessly.
pub fn spawn_refresh(
    client: Arc<dyn GitClient>,
    paths: Vec<PathBuf>,
    fetch: bool,
) -> Receiver<StatusUpdate> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for path in paths {
            // The receiver may have been dropped (app exited); stop quietly.
            if sender
                .send(StatusUpdate::Started { path: path.clone() })
                .is_err()
            {
                break;
            }
            if fetch {
                client.fetch(&path);
            }
            let info = client.collect(&path);
            if sender.send(StatusUpdate::Done { path, info }).is_err() {
                break;
            }
        }
    });
    receiver
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// A fake client returning a fixed branch, recording fetches.
    use crate::domain::stats::GitStats;

    struct FakeClient;

    impl GitClient for FakeClient {
        fn collect(&self, _path: &Path) -> GitInfo {
            GitInfo {
                valid: true,
                current_branch_name: Some("main".to_string()),
                changes: Some(0),
                ..GitInfo::default()
            }
        }
        fn fetch(&self, _path: &Path) {}
        fn log(&self, _path: &Path, _max: usize) -> Vec<String> {
            Vec::new()
        }
        fn stats(&self, _path: &Path) -> GitStats {
            GitStats::default()
        }
    }

    #[test]
    fn collect_all_returns_info_per_path() {
        let paths = vec![
            PathBuf::from("/a"),
            PathBuf::from("/b"),
            PathBuf::from("/c"),
        ];
        let results = collect_all(&FakeClient, &paths, false);
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|(_, info)| info.valid));
    }

    #[test]
    fn spawn_refresh_streams_every_path() {
        let paths = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        let rx = spawn_refresh(Arc::new(FakeClient), paths, false);
        let mut done = 0;
        while let Ok(update) = rx.recv() {
            if matches!(update, StatusUpdate::Done { .. }) {
                done += 1;
            }
        }
        assert_eq!(done, 2);
    }
}
