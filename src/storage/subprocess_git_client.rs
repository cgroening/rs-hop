//! [`GitClient`] backed by the `git` executable.
//!
//! Each query runs `git -C <path> ...` with explicit arguments (never a shell
//! string), so paths with spaces are safe and there is no injection surface.

use std::path::Path;
use std::process::Command;

use chrono::{Duration, Local};

use crate::domain::repo::{GitInfo, PATH_NOT_FOUND};
use crate::domain::stats::{GitStats, RECENT_DAYS};
use crate::storage::git_client::{
    GitClient, git_info_from_counts, parse_github_name, parse_refs,
    parse_shortlog, parse_timestamps,
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
                error: Some(PATH_NOT_FOUND.to_string()),
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

    fn log(&self, path: &Path, max: usize) -> Vec<String> {
        if !path.exists() {
            return Vec::new();
        }
        let arg = format!("-{max}");
        git_output(path, &["log", "--oneline", "--no-color", &arg])
            .map(|out| out.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    fn stats(&self, path: &Path) -> GitStats {
        if !path.exists() {
            return GitStats::default();
        }
        // `shortlog` reads stdin without a revision, which would hang the
        // worker thread; `HEAD` is required, not decoration.
        let (commits, contributors) =
            git_output(path, &["shortlog", "-sne", "HEAD"])
                .map(|out| parse_shortlog(&out))
                .unwrap_or_default();
        let (branches, tags) = git_output(
            path,
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/heads",
                "refs/tags",
            ],
        )
        .map(|out| parse_refs(&out))
        .unwrap_or_default();
        GitStats {
            commits,
            contributors,
            branches,
            tags,
            // `--max-parents=0` lists the root commits: a grafted history has
            // several, and the repository began at the earliest.
            first_commit: git_output(
                path,
                &["log", "--max-parents=0", "--format=%ct", "HEAD"],
            )
            .and_then(|out| parse_timestamps(&out)),
            last_commit: git_output(
                path,
                &["log", "-1", "--format=%ct", "HEAD"],
            )
            .and_then(|out| parse_timestamps(&out)),
            commits_recent: recent_commits(path),
        }
    }
}

/// Commits within the last [`RECENT_DAYS`] days. The cutoff is computed here
/// rather than handed to git as a relative phrase, so the boundary is ours.
fn recent_commits(path: &Path) -> u64 {
    let cutoff = Local::now() - Duration::days(RECENT_DAYS);
    let since = format!("--since={}", cutoff.format("%Y-%m-%dT%H:%M:%S%z"));
    git_output(path, &["rev-list", "--count", &since, "HEAD"])
        .and_then(|out| out.trim().parse::<u64>().ok())
        .unwrap_or(0)
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
