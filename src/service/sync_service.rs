//! Pulls or pushes git entries, one after another: synchronously for the CLI,
//! or on a background worker for the TUI.
//!
//! Each repository is gathered fresh first, so the decision whether to touch
//! it ([`sync::decide`]) never rests on a stale cache; then the pull or push
//! runs, and the status is gathered again so the caller can show the result
//! without a separate refresh.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use crate::domain::repo::GitInfo;
use crate::domain::sync::{self, Decision, SyncOp, SyncOutcome};
use crate::storage::git_client::GitClient;

pub use crate::storage::git_client::SyncOptions;

/// What one pull or push run is: its direction and how it reaches the remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncPlan {
    /// Pull or push.
    pub op: SyncOp,
    /// The network settings handed to git.
    pub options: SyncOptions,
}

/// The finished sync of one repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    /// The entry path that was synced.
    pub path: PathBuf,
    /// How it went.
    pub outcome: SyncOutcome,
    /// The status gathered after the sync, so the list is current at once.
    pub info: GitInfo,
}

/// A message from the background worker.
pub enum SyncUpdate {
    /// The worker has begun on this path (names it in the progress bar).
    Started {
        /// The entry path now being synced.
        path: PathBuf,
    },
    /// This path is done.
    Done(SyncReport),
}

/// What [`sync_one`] would do with `path`, without running a pull or push
/// (for `--dry-run`).
pub fn preview_one(
    client: &dyn GitClient,
    path: &Path,
    op: SyncOp,
) -> Decision {
    sync::decide(op, &client.collect(path))
}

/// Syncs one repository: gather, decide, run, gather again.
pub fn sync_one(
    client: &dyn GitClient,
    path: PathBuf,
    plan: SyncPlan,
) -> SyncReport {
    let before = client.collect(&path);
    let outcome = match sync::decide(plan.op, &before) {
        Decision::Settled(outcome) => {
            return SyncReport {
                path,
                outcome,
                info: before,
            };
        }
        Decision::Run => match plan.op {
            SyncOp::Pull => client.pull(&path, plan.options),
            SyncOp::Push => client.push(&path, plan.options),
        },
    };
    let info = client.collect(&path);
    SyncReport {
        path,
        outcome,
        info,
    }
}

/// Spawns a worker that syncs each path in order and streams the results.
///
/// The receiver yields a `Started` and a `Done` per path, then closes. When it
/// is dropped (the app quit), the worker stops before the next repository.
pub fn spawn_sync(
    client: Arc<dyn GitClient>,
    paths: Vec<PathBuf>,
    plan: SyncPlan,
) -> Receiver<SyncUpdate> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for path in paths {
            if sender
                .send(SyncUpdate::Started { path: path.clone() })
                .is_err()
            {
                break;
            }
            let report = sync_one(client.as_ref(), path, plan);
            if sender.send(SyncUpdate::Done(report)).is_err() {
                break;
            }
        }
    });
    receiver
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Mutex;

    use super::*;
    use crate::domain::stats::GitStats;
    use crate::domain::sync::SkipReason;

    const PLAN_PULL: SyncPlan = SyncPlan {
        op: SyncOp::Pull,
        options: SyncOptions { timeout_secs: 1 },
    };

    /// A client whose status is fixed per path and that records each network
    /// call. A synced path reads as level with its upstream afterwards.
    struct FakeClient {
        dirty: Vec<PathBuf>,
        calls: Mutex<Vec<String>>,
        synced: Mutex<HashSet<PathBuf>>,
    }

    impl FakeClient {
        fn new(dirty: &[&str]) -> Self {
            FakeClient {
                dirty: dirty.iter().map(PathBuf::from).collect(),
                calls: Mutex::new(Vec::new()),
                synced: Mutex::new(HashSet::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls
                .lock()
                .expect("test lock is never poisoned")
                .clone()
        }

        fn record(&self, call: &str, path: &Path) {
            self.calls
                .lock()
                .expect("test lock is never poisoned")
                .push(format!("{call} {}", path.display()));
            self.synced
                .lock()
                .expect("test lock is never poisoned")
                .insert(path.to_path_buf());
        }
    }

    impl GitClient for FakeClient {
        fn collect(&self, path: &Path) -> GitInfo {
            let is_synced = self
                .synced
                .lock()
                .expect("test lock is never poisoned")
                .contains(path);
            let is_dirty = self.dirty.iter().any(|dirty| dirty == path);
            GitInfo {
                valid: true,
                current_branch_name: Some("main".to_string()),
                changes: Some(u32::from(is_dirty)),
                ahead: Some(u32::from(!is_synced)),
                behind: Some(u32::from(!is_synced)),
                ..GitInfo::default()
            }
        }
        fn fetch(&self, _path: &Path) {}
        fn pull(&self, path: &Path, _options: SyncOptions) -> SyncOutcome {
            self.record("pull", path);
            SyncOutcome::Updated
        }
        fn push(&self, path: &Path, _options: SyncOptions) -> SyncOutcome {
            self.record("push", path);
            SyncOutcome::Updated
        }
        fn log(&self, _path: &Path, _max: usize) -> Vec<String> {
            Vec::new()
        }
        fn stats(&self, _path: &Path) -> GitStats {
            GitStats::default()
        }
    }

    #[test]
    fn a_synced_repo_reports_its_status_after_the_run() {
        let client = FakeClient::new(&[]);
        let report = sync_one(&client, PathBuf::from("/a"), PLAN_PULL);
        assert_eq!(report.outcome, SyncOutcome::Updated);
        assert_eq!(client.calls(), vec!["pull /a"]);
        // Gathered again after the pull, so it no longer reads "behind".
        assert_eq!(report.info.behind, Some(0));
    }

    #[test]
    fn a_settled_repo_is_never_handed_to_git() {
        let client = FakeClient::new(&["/dirty"]);
        let report = sync_one(&client, PathBuf::from("/dirty"), PLAN_PULL);
        assert_eq!(report.outcome, SyncOutcome::Skipped(SkipReason::Dirty));
        assert!(client.calls().is_empty());
    }

    #[test]
    fn a_preview_decides_without_touching_git() {
        let client = FakeClient::new(&["/dirty"]);
        assert_eq!(
            preview_one(&client, Path::new("/clean"), SyncOp::Pull),
            Decision::Run
        );
        assert_eq!(
            preview_one(&client, Path::new("/dirty"), SyncOp::Pull),
            Decision::Settled(SyncOutcome::Skipped(SkipReason::Dirty))
        );
        assert!(client.calls().is_empty());
    }

    #[test]
    fn the_worker_starts_and_finishes_every_path_in_order() {
        let client = Arc::new(FakeClient::new(&["/b"]));
        let paths = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        let plan = SyncPlan {
            op: SyncOp::Push,
            ..PLAN_PULL
        };
        let rx = spawn_sync(client.clone(), paths, plan);
        let events: Vec<String> = rx
            .iter()
            .map(|update| match update {
                SyncUpdate::Started { path } => {
                    format!("start {}", path.display())
                }
                SyncUpdate::Done(report) => {
                    format!("done {}", report.path.display())
                }
            })
            .collect();
        assert_eq!(events, ["start /a", "done /a", "start /b", "done /b"]);
        // Dirty work (`/b`) does not block a push.
        assert_eq!(client.calls(), vec!["push /a", "push /b"]);
    }
}
