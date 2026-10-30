//! Pulling and pushing git entries: whether a repository is touched at all,
//! how a run went per repository, and the one-line summary of a whole run.
//!
//! Everything here is pure. The git calls live behind the
//! [`GitClient`](crate::storage::git_client::GitClient) port; this module only
//! decides from a freshly gathered [`GitInfo`] and names the outcome.

use std::fmt;

use crate::domain::repo::GitInfo;

/// The branch name `git rev-parse --abbrev-ref HEAD` reports on a detached
/// HEAD.
const DETACHED_HEAD: &str = "HEAD";

/// Separator between the parts of a run summary.
const SUMMARY_SEPARATOR: &str = " · ";

/// Which way a sync run moves commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncOp {
    /// `git pull --ff-only`: bring the remote's commits down.
    Pull,
    /// `git push`: publish the local commits.
    Push,
}

impl SyncOp {
    /// The command verb, as in "pull 3 repos".
    pub fn verb(self) -> &'static str {
        match self {
            SyncOp::Pull => "pull",
            SyncOp::Push => "push",
        }
    }

    /// The verb capitalised, to open a title or a question.
    pub fn title(self) -> &'static str {
        match self {
            SyncOp::Pull => "Pull",
            SyncOp::Push => "Push",
        }
    }

    /// The verb in its progressive form, for the progress bar.
    pub fn progressive(self) -> &'static str {
        match self {
            SyncOp::Pull => "pulling",
            SyncOp::Push => "pushing",
        }
    }

    /// The past tense, for a repository that was updated.
    fn past(self) -> &'static str {
        match self {
            SyncOp::Pull => "pulled",
            SyncOp::Push => "pushed",
        }
    }

    /// What a repository with nothing to move is called.
    fn idle(self) -> &'static str {
        match self {
            SyncOp::Pull => "up to date",
            SyncOp::Push => "nothing to push",
        }
    }
}

/// Why a repository was left alone on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// A pull would mix the remote's commits into uncommitted work.
    Dirty,
    /// No tracking branch (or a detached HEAD), so there is nothing to sync
    /// against.
    NoUpstream,
}

impl SkipReason {
    /// The reason as shown after "skipped: ".
    fn text(self) -> &'static str {
        match self {
            SkipReason::Dirty => "uncommitted changes",
            SkipReason::NoUpstream => "no upstream branch",
        }
    }
}

/// How the sync of one repository ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// Commits were pulled or pushed.
    Updated,
    /// There was nothing to pull or push.
    UpToDate,
    /// Left alone on purpose.
    Skipped(SkipReason),
    /// git ran and failed, or the repository could not be read; the text says
    /// why.
    Failed(String),
}

impl SyncOutcome {
    /// The outcome as a short phrase for `op`, e.g. "pulled" or
    /// "skipped: uncommitted changes".
    pub fn describe(&self, op: SyncOp) -> String {
        match self {
            SyncOutcome::Updated => op.past().to_string(),
            SyncOutcome::UpToDate => op.idle().to_string(),
            SyncOutcome::Skipped(reason) => {
                format!("skipped: {}", reason.text())
            }
            SyncOutcome::Failed(why) => format!("failed: {why}"),
        }
    }

    /// Whether this outcome deserves the user's attention (skipped or failed).
    pub fn is_problem(&self) -> bool {
        matches!(self, SyncOutcome::Skipped(_) | SyncOutcome::Failed(_))
    }
}

/// What to do with a repository before any network call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Run the pull or push.
    Run,
    /// Settled without running git: the outcome is already known.
    Settled(SyncOutcome),
}

/// Decides from a freshly gathered `info` whether `op` should run.
///
/// An unreadable repository fails with its error; a branch without upstream
/// (or a detached HEAD) is skipped; a pull skips uncommitted work; a push with
/// no commits ahead has nothing to do. Everything else runs - a pull always
/// runs, because the stored "behind" count may predate the last fetch.
pub fn decide(op: SyncOp, info: &GitInfo) -> Decision {
    if !info.valid {
        let why = info.error.clone().unwrap_or_else(|| "unreadable".into());
        return Decision::Settled(SyncOutcome::Failed(why));
    }
    if !has_upstream(info) {
        return skip(SkipReason::NoUpstream);
    }
    match op {
        SyncOp::Pull if info.changes.unwrap_or(0) > 0 => {
            skip(SkipReason::Dirty)
        }
        SyncOp::Push if info.ahead.unwrap_or(0) == 0 => {
            Decision::Settled(SyncOutcome::UpToDate)
        }
        SyncOp::Pull | SyncOp::Push => Decision::Run,
    }
}

/// A settled skip for `reason`.
fn skip(reason: SkipReason) -> Decision {
    Decision::Settled(SyncOutcome::Skipped(reason))
}

/// Whether `info` names a branch that tracks an upstream. The ahead/behind
/// counts are only gathered against an upstream, so their absence means none.
fn has_upstream(info: &GitInfo) -> bool {
    let is_on_branch = info
        .current_branch_name
        .as_deref()
        .is_some_and(|branch| branch != DETACHED_HEAD);
    is_on_branch && info.ahead.is_some() && info.behind.is_some()
}

/// The counts of a finished (or running) sync run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    /// The direction of the run, which names the counts.
    pub op: SyncOp,
    /// Repositories that were pulled or pushed.
    pub updated: usize,
    /// Repositories with nothing to move.
    pub up_to_date: usize,
    /// Repositories left alone on purpose.
    pub skipped: usize,
    /// Repositories whose sync failed.
    pub failed: usize,
}

impl Summary {
    /// Counts `outcomes` of an `op` run.
    pub fn of<'a>(
        op: SyncOp,
        outcomes: impl IntoIterator<Item = &'a SyncOutcome>,
    ) -> Self {
        let mut summary = Summary {
            op,
            updated: 0,
            up_to_date: 0,
            skipped: 0,
            failed: 0,
        };
        for outcome in outcomes {
            match outcome {
                SyncOutcome::Updated => summary.updated += 1,
                SyncOutcome::UpToDate => summary.up_to_date += 1,
                SyncOutcome::Skipped(_) => summary.skipped += 1,
                SyncOutcome::Failed(_) => summary.failed += 1,
            }
        }
        summary
    }

    /// Whether any repository failed (drives the CLI's exit code).
    pub fn has_failures(&self) -> bool {
        self.failed > 0
    }

    /// Whether any repository was skipped or failed (opens the result list).
    pub fn has_problems(&self) -> bool {
        self.skipped > 0 || self.failed > 0
    }
}

impl fmt::Display for Summary {
    /// Writes only the non-zero counts, e.g.
    /// `pulled 5 · up to date 3 · failed 1`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = [
            (self.op.past(), self.updated),
            (self.op.idle(), self.up_to_date),
            ("skipped", self.skipped),
            ("failed", self.failed),
        ]
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .map(|(label, count)| format!("{label} {count}"))
        .collect();
        if parts.is_empty() {
            return write!(f, "nothing to {}", self.op.verb());
        }
        write!(f, "{}", parts.join(SUMMARY_SEPARATOR))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A readable repository on `main` that tracks an upstream.
    fn tracked(changes: u32, ahead: u32, behind: u32) -> GitInfo {
        GitInfo {
            valid: true,
            current_branch_name: Some("main".to_string()),
            changes: Some(changes),
            ahead: Some(ahead),
            behind: Some(behind),
            ..GitInfo::default()
        }
    }

    #[test]
    fn a_clean_tracked_repo_is_always_pulled() {
        // Even "behind 0" pulls: the count may predate the last fetch.
        assert_eq!(decide(SyncOp::Pull, &tracked(0, 0, 0)), Decision::Run);
        assert_eq!(decide(SyncOp::Pull, &tracked(0, 2, 3)), Decision::Run);
    }

    #[test]
    fn a_pull_skips_uncommitted_work() {
        assert_eq!(
            decide(SyncOp::Pull, &tracked(4, 0, 1)),
            Decision::Settled(SyncOutcome::Skipped(SkipReason::Dirty))
        );
    }

    #[test]
    fn a_push_runs_only_with_commits_ahead() {
        assert_eq!(decide(SyncOp::Push, &tracked(0, 2, 0)), Decision::Run);
        // Dirty work does not block a push; it only publishes commits.
        assert_eq!(decide(SyncOp::Push, &tracked(3, 1, 0)), Decision::Run);
        // Behind as well as ahead: git rejects it, which is reported.
        assert_eq!(decide(SyncOp::Push, &tracked(0, 1, 1)), Decision::Run);
        assert_eq!(
            decide(SyncOp::Push, &tracked(0, 0, 5)),
            Decision::Settled(SyncOutcome::UpToDate)
        );
    }

    #[test]
    fn a_branch_without_upstream_is_skipped_both_ways() {
        let info = GitInfo {
            ahead: None,
            behind: None,
            ..tracked(0, 0, 0)
        };
        for op in [SyncOp::Pull, SyncOp::Push] {
            assert_eq!(
                decide(op, &info),
                Decision::Settled(SyncOutcome::Skipped(SkipReason::NoUpstream))
            );
        }
    }

    #[test]
    fn a_detached_head_counts_as_no_upstream() {
        let info = GitInfo {
            current_branch_name: Some(DETACHED_HEAD.to_string()),
            ..tracked(0, 1, 0)
        };
        assert_eq!(
            decide(SyncOp::Push, &info),
            Decision::Settled(SyncOutcome::Skipped(SkipReason::NoUpstream))
        );
    }

    #[test]
    fn an_unreadable_repo_fails_with_its_error() {
        let info = GitInfo {
            valid: false,
            error: Some("path not found".to_string()),
            ..GitInfo::default()
        };
        assert_eq!(
            decide(SyncOp::Pull, &info),
            Decision::Settled(SyncOutcome::Failed(
                "path not found".to_string()
            ))
        );
    }

    #[test]
    fn outcomes_are_described_per_direction() {
        assert_eq!(SyncOutcome::Updated.describe(SyncOp::Pull), "pulled");
        assert_eq!(SyncOutcome::Updated.describe(SyncOp::Push), "pushed");
        assert_eq!(SyncOutcome::UpToDate.describe(SyncOp::Pull), "up to date");
        assert_eq!(
            SyncOutcome::UpToDate.describe(SyncOp::Push),
            "nothing to push"
        );
        assert_eq!(
            SyncOutcome::Skipped(SkipReason::Dirty).describe(SyncOp::Pull),
            "skipped: uncommitted changes"
        );
        assert_eq!(
            SyncOutcome::Failed("timeout".to_string()).describe(SyncOp::Push),
            "failed: timeout"
        );
    }

    #[test]
    fn the_summary_lists_only_non_zero_counts() {
        let outcomes = [
            SyncOutcome::Updated,
            SyncOutcome::Updated,
            SyncOutcome::UpToDate,
            SyncOutcome::Failed("x".to_string()),
        ];
        let summary = Summary::of(SyncOp::Pull, &outcomes);
        assert_eq!(summary.to_string(), "pulled 2 · up to date 1 · failed 1");
        assert!(summary.has_failures());
        assert!(summary.has_problems());
    }

    #[test]
    fn a_skip_is_a_problem_but_not_a_failure() {
        let outcomes = [SyncOutcome::Skipped(SkipReason::NoUpstream)];
        let summary = Summary::of(SyncOp::Push, &outcomes);
        assert_eq!(summary.to_string(), "skipped 1");
        assert!(!summary.has_failures());
        assert!(summary.has_problems());
    }

    #[test]
    fn an_empty_run_says_there_was_nothing_to_do() {
        let summary = Summary::of(SyncOp::Push, &[]);
        assert_eq!(summary.to_string(), "nothing to push");
        assert!(!summary.has_problems());
    }
}
