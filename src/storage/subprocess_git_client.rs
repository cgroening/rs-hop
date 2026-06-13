//! [`GitClient`] backed by the `git` executable.
//!
//! Each query runs `git -C <path> ...` with explicit arguments (never a shell
//! string), so paths with spaces are safe and there is no injection surface.

use std::path::Path;
use std::process::Command;

use crate::domain::repo::GitInfo;
use crate::storage::git_client::{
    GitClient, git_info_from_counts, parse_github_name,
};

/// Gathers git status by shelling out to `git`.
pub struct SubprocessGitClient {
    github_username: Option<String>,
}

impl SubprocessGitClient {
    /// Creates a client that strips `github_username` from remote names.
    pub fn new(github_username: Option<String>) -> Self {
        SubprocessGitClient { github_username }
    }
}

impl GitClient for SubprocessGitClient {
    fn collect(&self, path: &Path) -> GitInfo {
        if !path.exists() {
            return GitInfo {
                valid: false,
                error: Some("path not found".to_string()),
                ..GitInfo::default()
            };
        }
        if git_output(path, &["rev-parse", "--is-inside-work-tree"]).as_deref()
            != Some("true")
        {
            return GitInfo {
                valid: false,
                error: Some("not a git repository".to_string()),
                ..GitInfo::default()
            };
        }
        let branch = git_output(path, &["rev-parse", "--abbrev-ref", "HEAD"]);
        let changes = git_output(path, &["status", "--porcelain"])
            .map(|out| out.lines().filter(|line| !line.is_empty()).count())
            .unwrap_or(0) as u32;
        let (ahead, behind) = ahead_behind(path);
        let github = git_output(path, &["remote", "get-url", "origin"])
            .and_then(|url| {
                parse_github_name(&url, self.github_username.as_deref())
            });
        git_info_from_counts(branch, changes, ahead, behind, github)
    }

    fn fetch(&self, path: &Path) {
        // Skip missing paths and capture all output: `.status()` would inherit
        // git's stderr (e.g. "fatal: cannot change to ...") into the TUI's
        // alternate screen and corrupt the display.
        if !path.exists() {
            return;
        }
        let _ = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["fetch", "--quiet"])
            .output();
    }
}

/// Runs `git -C <path> <args>` and returns trimmed stdout on success.
fn git_output(path: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The commits ahead of and behind the upstream, or `(None, None)` when no
/// upstream is configured. `git rev-list --left-right --count HEAD...@{u}`
/// prints "<ahead>\t<behind>" (left = HEAD-only commits).
fn ahead_behind(path: &Path) -> (Option<u32>, Option<u32>) {
    let Some(out) = git_output(
        path,
        &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
    ) else {
        return (None, None);
    };
    let mut parts = out.split_whitespace();
    let ahead = parts.next().and_then(|n| n.parse().ok());
    let behind = parts.next().and_then(|n| n.parse().ok());
    (ahead, behind)
}
