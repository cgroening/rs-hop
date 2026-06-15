//! Gathers `git log` excerpts for the preview panel in the background.
//!
//! The TUI shows a short `git log` for the cursor entry. Running `git` on the
//! main thread stalls cursor movement, so the work is handed to a worker thread
//! that streams a [`PreviewLog`] per path over a shared channel. The caller
//! keeps the sender alive and drains results without blocking.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread;

use crate::storage::git_client::GitClient;

/// A gathered `git log` excerpt for one path, streamed from the worker.
pub struct PreviewLog {
    /// The entry path the log belongs to.
    pub path: PathBuf,
    /// The `git log --oneline` lines (empty when the path is not a repo).
    pub lines: Vec<String>,
}

/// Spawns a worker that gathers the log for each path and streams the results.
/// Sends one [`PreviewLog`] per path into `sender`, stopping quietly if the
/// receiver was dropped (app exited) or superseded by a newer request.
pub fn spawn_logs(
    client: Arc<dyn GitClient>,
    paths: Vec<PathBuf>,
    max: usize,
    sender: Sender<PreviewLog>,
) {
    thread::spawn(move || {
        for path in paths {
            let lines = client.log(&path, max);
            if sender.send(PreviewLog { path, lines }).is_err() {
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::mpsc;

    use super::*;
    use crate::domain::repo::GitInfo;

    /// A fake client returning a fixed log line per path.
    struct FakeClient;

    impl GitClient for FakeClient {
        fn collect(&self, _path: &Path) -> GitInfo {
            GitInfo::default()
        }
        fn fetch(&self, _path: &Path) {}
        fn log(&self, _path: &Path, _max: usize) -> Vec<String> {
            vec!["abc123 commit".to_string()]
        }
    }

    #[test]
    fn spawn_logs_streams_every_path() {
        let paths = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        let (tx, rx) = mpsc::channel();
        spawn_logs(Arc::new(FakeClient), paths, 5, tx);
        let mut count = 0;
        while let Ok(log) = rx.recv() {
            assert_eq!(log.lines, vec!["abc123 commit".to_string()]);
            count += 1;
        }
        assert_eq!(count, 2);
    }
}
