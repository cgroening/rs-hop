//! [`GitClient`] backed by the `git` executable.
//!
//! Each query runs `git -C <path> ...` with explicit arguments (never a shell
//! string), so paths with spaces are safe and there is no injection surface.

use std::path::Path;
use std::process::{Command, Output, Stdio};

use chrono::{Duration, Local};

use crate::domain::repo::{GitInfo, PATH_NOT_FOUND};
use crate::domain::stats::{GitStats, RECENT_DAYS};
use crate::domain::sync::SyncOutcome;
use crate::storage::git_client::{
    GitClient, SyncOptions, classify_git_failure, git_info_from_counts,
    parse_github_name, parse_refs, parse_shortlog, parse_timestamps,
};

/// What `git pull` prints when there was nothing to fetch in.
const PULL_UP_TO_DATE: &str = "Already up to date";

/// What `git push` prints when the remote already has every commit.
const PUSH_UP_TO_DATE: &str = "Everything up-to-date";

/// The ssh program git runs when nothing else is configured.
const DEFAULT_SSH: &str = "ssh";

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

    fn pull(&self, path: &Path, options: SyncOptions) -> SyncOutcome {
        run_sync(path, options, &["pull", "--ff-only"], PULL_UP_TO_DATE)
    }

    fn push(&self, path: &Path, options: SyncOptions) -> SyncOutcome {
        run_sync(path, options, &["push"], PUSH_UP_TO_DATE)
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

/// Runs the network command `args` (a pull or a push) in `path` and reads its
/// outcome: `up_to_date` in the output means there was nothing to move.
fn run_sync(
    path: &Path,
    options: SyncOptions,
    args: &[&str],
    up_to_date: &str,
) -> SyncOutcome {
    if !path.exists() {
        return SyncOutcome::Failed(PATH_NOT_FOUND.to_string());
    }
    let output = match sync_command(path, options).args(args).output() {
        Ok(output) => output,
        Err(error) => {
            return SyncOutcome::Failed(format!("cannot run git: {error}"));
        }
    };
    let text = combined_output(&output);
    if !output.status.success() {
        return SyncOutcome::Failed(classify_git_failure(&text));
    }
    if text.contains(up_to_date) {
        return SyncOutcome::UpToDate;
    }
    SyncOutcome::Updated
}

/// A `git -C <path>` command that never waits on a person and gives up on a
/// stalled connection.
///
/// No prompt may reach the terminal: the TUI owns it, and a CLI run in a
/// pipeline would hang. So credential prompts are switched off
/// (`GIT_TERMINAL_PROMPT`, `GCM_INTERACTIVE`), ssh runs in batch mode, and
/// stdin is closed. A connection that stalls for `timeout_secs` is dropped by
/// ssh (`ConnectTimeout`, `ServerAliveInterval`) or curl (`lowSpeedTime`).
/// `LC_ALL=C` keeps git's messages in English, so they can be classified.
fn sync_command(path: &Path, options: SyncOptions) -> Command {
    let secs = options.timeout_secs;
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(path)
        .args(["-c", "http.lowSpeedLimit=1"])
        .arg("-c")
        .arg(format!("http.lowSpeedTime={secs}"))
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("LC_ALL", "C")
        .stdin(Stdio::null());
    // `GIT_SSH` names a wrapper program that takes no options; leave it be
    // rather than override it with a command it would not run through.
    if std::env::var_os("GIT_SSH").is_none() {
        let base = configured_ssh(path);
        command.env("GIT_SSH_COMMAND", batch_ssh_command(&base, secs));
    }
    command
}

/// The ssh command git would run for `path`: `GIT_SSH_COMMAND`, else
/// `core.sshCommand`, else plain `ssh`. Reading it first keeps a custom ssh
/// setup working while the batch options are appended to it.
fn configured_ssh(path: &Path) -> String {
    if let Ok(command) = std::env::var("GIT_SSH_COMMAND")
        && !command.trim().is_empty()
    {
        return command;
    }
    git_output(path, &["config", "--get", "core.sshCommand"])
        .filter(|command| !command.is_empty())
        .unwrap_or_else(|| DEFAULT_SSH.to_string())
}

/// `base` with the options that make ssh fail instead of asking, and give up
/// on a connection that stalls for `secs` seconds.
fn batch_ssh_command(base: &str, secs: u64) -> String {
    format!(
        "{base} -o BatchMode=yes -o ConnectTimeout={secs} \
         -o ServerAliveInterval={secs} -o ServerAliveCountMax=1"
    )
}

/// stdout and stderr together: git splits its messages between the two
/// depending on the command, so both are searched.
fn combined_output(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_batch_ssh_command_extends_the_configured_one() {
        assert_eq!(
            batch_ssh_command("ssh -i ~/.ssh/work", 15),
            "ssh -i ~/.ssh/work -o BatchMode=yes -o ConnectTimeout=15 \
             -o ServerAliveInterval=15 -o ServerAliveCountMax=1"
        );
    }

    #[test]
    fn syncing_a_missing_path_fails_without_running_git() {
        let outcome = run_sync(
            Path::new("/definitely/not/here/hop-sync"),
            SyncOptions { timeout_secs: 1 },
            &["pull"],
            PULL_UP_TO_DATE,
        );
        assert_eq!(outcome, SyncOutcome::Failed(PATH_NOT_FOUND.to_string()));
    }
}
