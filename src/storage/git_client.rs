//! The git-client port and the pure parsing it relies on.
//!
//! [`GitClient`] gathers a [`GitInfo`] for a path; the subprocess backend
//! implements it by shelling out to `git`. The remote-URL parsing is pure and
//! lives here so it can be unit-tested without a repository.

use std::path::Path;

use crate::domain::repo::GitInfo;
use crate::domain::stats::GitStats;

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
    /// The repository history behind `path`: commit and author counts, the
    /// first and last commit, branches and tags. A path that is not a
    /// repository, or one without commits, yields the default (all zero).
    fn stats(&self, path: &Path) -> GitStats;
}

/// Counts commits and distinct authors from `git shortlog -sne HEAD` output,
/// whose lines read `<count>\t<name> <email>`.
///
/// `git shortlog` reads standard input when given no revision, so the caller
/// must always pass `HEAD` - otherwise it blocks forever in a worker thread.
pub fn parse_shortlog(text: &str) -> (u64, u64) {
    let mut commits = 0;
    let mut authors = 0;
    for line in text.lines() {
        let Some((count, _)) = line.trim_start().split_once('\t') else {
            continue;
        };
        let Ok(count) = count.trim().parse::<u64>() else {
            continue;
        };
        commits += count;
        authors += 1;
    }
    (commits, authors)
}

/// The earliest unix timestamp in `text`, one per line.
///
/// Used for the first commit, which `git log --max-parents=0 --format=%ct`
/// reports once per root commit - a grafted or merged history has several, and
/// the earliest is the one the repository started at. (`git log --reverse -1`
/// does *not* work: `-1` limits before `--reverse` reverses, so it returns the
/// newest commit.)
pub fn parse_timestamps(text: &str) -> Option<i64> {
    text.lines()
        .filter_map(|line| line.trim().parse::<i64>().ok())
        .min()
}

/// Counts the branches and tags in `git for-each-ref --format=%(refname)`
/// output, which lists `refs/heads/...` and `refs/tags/...` lines.
pub fn parse_refs(text: &str) -> (u64, u64) {
    let mut branches = 0;
    let mut tags = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("refs/heads/") {
            branches += 1;
        } else if line.starts_with("refs/tags/") {
            tags += 1;
        }
    }
    (branches, tags)
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
    fn parse_shortlog_sums_commits_and_counts_authors() {
        let text =
            "    42\tAda <ada@example.com>\n     7\tLin <lin@example.com>\n";
        assert_eq!(parse_shortlog(text), (49, 2));
        assert_eq!(parse_shortlog(""), (0, 0));
        // A malformed line is skipped rather than counted.
        assert_eq!(parse_shortlog("no tab here\n"), (0, 0));
    }

    #[test]
    fn parse_timestamps_takes_the_earliest_root_commit() {
        // A grafted history reports several root commits; the repo began at
        // the earliest of them.
        assert_eq!(
            parse_timestamps("1700000000\n1600000000\n"),
            Some(1_600_000_000)
        );
        assert_eq!(parse_timestamps("1700000000"), Some(1_700_000_000));
        assert_eq!(parse_timestamps(""), None);
        assert_eq!(parse_timestamps("not a number"), None);
    }

    #[test]
    fn parse_refs_separates_branches_from_tags() {
        let text = "refs/heads/main\nrefs/heads/dev\nrefs/tags/v1.0\n";
        assert_eq!(parse_refs(text), (2, 1));
        assert_eq!(parse_refs(""), (0, 0));
        // A ref that is neither is ignored.
        assert_eq!(parse_refs("refs/remotes/origin/main\n"), (0, 0));
    }

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
