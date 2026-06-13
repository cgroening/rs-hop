//! The git-client port and the pure parsing it relies on.
//!
//! [`GitClient`] gathers a [`GitInfo`] for a path; the subprocess backend
//! implements it by shelling out to `git`. The remote-URL parsing is pure and
//! lives here so it can be unit-tested without a repository.

use std::path::Path;

use crate::domain::repo::GitInfo;

/// Gathers git status for repositories. Implementations never error out: a
/// failure is encoded in the returned [`GitInfo`] (`valid = false`).
///
/// The trait is `Send + Sync` so a background thread can hold one.
pub trait GitClient: Send + Sync {
    /// Collects branch, change count, ahead/behind and remote name for `path`.
    fn collect(&self, path: &Path) -> GitInfo;
    /// Fetches the remote for `path` (best-effort; ignores failures).
    fn fetch(&self, path: &Path);
    /// The most recent `max` commits as one-line summaries (best-effort; empty
    /// when `path` is not a repository).
    fn log(&self, path: &Path, max: usize) -> Vec<String>;
}

/// Extracts an `owner/repo` name from a git remote `url`, stripping a trailing
/// `.git`. When `username` matches the owner, only `repo` is returned (the
/// owner is implied). Returns `None` for URLs without a recognisable path.
pub fn parse_github_name(url: &str, username: Option<&str>) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    let tail = remote_tail(trimmed)?;
    let tail = tail.strip_suffix(".git").unwrap_or(tail);
    let (owner, repo) = tail.split_once('/')?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    if username.is_some_and(|name| name.eq_ignore_ascii_case(owner)) {
        return Some(repo.to_string());
    }
    Some(format!("{owner}/{repo}"))
}

/// The `owner/repo` tail of an SSH (`git@host:owner/repo`) or HTTP(S)
/// (`https://host/owner/repo`) remote URL.
fn remote_tail(url: &str) -> Option<&str> {
    if let Some((_, tail)) = url.split_once("://") {
        // https://host/owner/repo -> owner/repo
        return tail.split_once('/').map(|(_, rest)| rest);
    }
    if let Some((_, tail)) = url.split_once('@') {
        // git@host:owner/repo -> owner/repo
        return tail.split_once(':').map(|(_, rest)| rest);
    }
    None
}

/// Builds the unstaged/branch [`GitInfo`] from raw counts (used by the backend).
pub fn git_info_from_counts(
    branch: Option<String>,
    changes: u32,
    ahead: Option<u32>,
    behind: Option<u32>,
    github_repo_name: Option<String>,
) -> GitInfo {
    GitInfo {
        valid: true,
        error: None,
        current_branch_name: branch,
        changes: Some(changes),
        ahead,
        behind,
        github_repo_name,
        raw_status: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ssh_remote() {
        assert_eq!(
            parse_github_name("git@github.com:owner/repo.git", None),
            Some("owner/repo".to_string())
        );
    }

    #[test]
    fn parses_https_remote() {
        assert_eq!(
            parse_github_name("https://github.com/owner/repo.git", None),
            Some("owner/repo".to_string())
        );
    }

    #[test]
    fn strips_matching_username() {
        assert_eq!(
            parse_github_name(
                "git@github.com:cgroening/hop.git",
                Some("cgroening")
            ),
            Some("hop".to_string())
        );
    }

    #[test]
    fn keeps_owner_when_username_differs() {
        assert_eq!(
            parse_github_name("https://github.com/other/hop", Some("me")),
            Some("other/hop".to_string())
        );
    }

    #[test]
    fn none_for_unparseable() {
        assert_eq!(parse_github_name("not-a-url", None), None);
        assert_eq!(parse_github_name("", None), None);
    }
}
