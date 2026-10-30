//! Pulling and pushing git entries from the list: the confirmation for a whole
//! tab, the background worker, and how its result is reported.
//!
//! A run owns the progress bar like a refresh or a ZIP backup does, so only one
//! of them runs at a time. Each repository's status is updated as soon as it is
//! done; skips and failures open a result list once the run ends.

use super::*;
use unicode_width::UnicodeWidthStr;

use std::path::PathBuf;
use std::sync::mpsc::TryRecvError;

use chrono::Local;

use crate::domain::filter::TabKind;
use crate::domain::repo::RepoKind;
use crate::domain::sync::{Summary, SyncOp, SyncOutcome};
use crate::service::sync_service::{
    SyncOptions, SyncPlan, SyncReport, SyncUpdate, spawn_sync,
};

/// Status message for a pull or push key pressed outside the git tabs.
const GIT_TABS_ONLY: &str = "pull/push: git tabs only";

/// Status message for a pull or push asked for while another job runs.
const BUSY: &str = "busy: a refresh, backup, pull or push is running";

/// Columns between the repo name and its outcome in the result list.
const RESULT_GAP: usize = 2;

/// Which entries a run covers.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SyncScope {
    /// The selection, else the cursor entry.
    Targets,
    /// Every git entry of the current tab.
    WholeTab,
}

/// One in-flight pull or push run.
pub(super) struct SyncJob {
    /// Receives the worker's updates.
    rx: Receiver<SyncUpdate>,
    /// The direction, which names the result.
    op: SyncOp,
    /// Which entries the run covers (a whole-tab pull refreshes the global
    /// "remote: fetched" time, since every pull fetches).
    scope: SyncScope,
    /// The finished repositories so far, as (display name, outcome).
    results: Vec<(String, SyncOutcome)>,
}

impl App {
    /// Pulls or pushes the target git entries (selection or cursor) at once.
    pub(super) fn sync_targets(&mut self, op: SyncOp) {
        if !self.can_start_sync() {
            return;
        }
        let paths = self.git_paths_of(&self.targets());
        if paths.is_empty() {
            self.set_status(format!("nothing to {}", op.verb()));
            return;
        }
        self.start_sync(op, paths, SyncScope::Targets);
        self.clear_selection();
    }

    /// Asks before pulling or pushing every git entry of the current tab.
    pub(super) fn confirm_sync_all(&mut self, op: SyncOp) {
        if !self.can_start_sync() {
            return;
        }
        let paths = self.git_paths_of(&self.tab_indices());
        if paths.is_empty() {
            self.set_status(format!("nothing to {}", op.verb()));
            return;
        }
        let noun = if paths.len() == 1 { "repo" } else { "repos" };
        let modal = ConfirmModal::new(
            format!(" {} all ", op.title()),
            format!("{} {} {noun}?", op.title(), paths.len()),
        );
        self.overlay = Overlay::SyncConfirm(modal, op, paths);
    }

    /// Starts the run confirmed in the dialog.
    pub(super) fn sync_confirmed(&mut self, op: SyncOp, paths: Vec<PathBuf>) {
        // The tab may have started a refresh while the dialog was open.
        if self.can_start_sync() {
            self.start_sync(op, paths, SyncScope::WholeTab);
        }
    }

    /// Applies the worker's pending updates without blocking, and reports the
    /// run once the worker has finished.
    pub(super) fn drain_sync(&mut self) {
        let Some(mut job) = self.sync_job.take() else {
            return;
        };
        loop {
            match job.rx.try_recv() {
                Ok(SyncUpdate::Started { path }) => {
                    self.loading_detail = Some(self.name_for_path(&path));
                }
                Ok(SyncUpdate::Done(report)) => {
                    self.apply_sync(&mut job, report)
                }
                Err(TryRecvError::Empty) => {
                    self.sync_job = Some(job);
                    return;
                }
                Err(TryRecvError::Disconnected) => break,
            }
        }
        self.finish_sync(job);
    }

    /// Whether a pull or push is running.
    pub(super) fn is_syncing(&self) -> bool {
        self.sync_job.is_some()
    }

    /// Whether a pull or push may start now, reporting why not when it may
    /// not.
    fn can_start_sync(&mut self) -> bool {
        let refusal = if self.tab.kind() != TabKind::Git {
            Some(GIT_TABS_ONLY)
        } else if self.config.example_mode {
            Some("example mode: live status is off")
        } else if self.loading.is_some() || self.is_refreshing() {
            Some(BUSY)
        } else {
            None
        };
        match refusal {
            Some(message) => {
                self.set_status(message);
                false
            }
            None => true,
        }
    }

    /// The paths of the git entries among the service `indices`.
    fn git_paths_of(&self, indices: &[usize]) -> Vec<PathBuf> {
        indices
            .iter()
            .filter_map(|&i| self.service.get(i))
            .filter(|repo| repo.kind == RepoKind::Git)
            .map(|repo| repo.path.clone())
            .collect()
    }

    /// Hands `paths` to the background worker and shows the progress bar.
    fn start_sync(
        &mut self,
        op: SyncOp,
        paths: Vec<PathBuf>,
        scope: SyncScope,
    ) {
        self.set_status(format!("{}…", op.progressive()));
        self.loading = Some((0, paths.len()));
        self.loading_label = op.progressive();
        self.loading_detail = None;
        self.loading_name_width = self.max_name_width(&paths);
        let plan = SyncPlan {
            op,
            options: SyncOptions {
                timeout_secs: self.config.git_timeout_secs,
            },
        };
        self.sync_job = Some(SyncJob {
            rx: spawn_sync(Arc::clone(&self.git_client), paths, plan),
            op,
            scope,
            results: Vec::new(),
        });
    }

    /// Records one finished repository: its fresh status, the bar, the result.
    fn apply_sync(&mut self, job: &mut SyncJob, report: SyncReport) {
        let name = self.name_for_path(&report.path);
        self.service.set_git_info(&report.path, report.info);
        if let Some((done, _)) = &mut self.loading {
            *done += 1;
        }
        job.results.push((name, report.outcome));
    }

    /// Ends a run: clears the bar, persists the fresh status, and reports.
    fn finish_sync(&mut self, job: SyncJob) {
        self.loading = None;
        self.loading_label = REFRESH_LABEL;
        self.loading_detail = None;
        self.loading_name_width = 0;
        let has_fetched =
            job.op == SyncOp::Pull && job.scope == SyncScope::WholeTab;
        self.finish_refresh(has_fetched);
        let summary =
            Summary::of(job.op, job.results.iter().map(|(_, outcome)| outcome));
        self.set_status(summary.to_string());
        if summary.has_problems() {
            self.open_sync_results(job.op, &job.results);
        }
        // A tab visited mid-run deferred its first refresh until now.
        self.refresh_tab_on_first_visit();
    }

    /// Opens the list of every repository of the run with its outcome.
    fn open_sync_results(
        &mut self,
        op: SyncOp,
        results: &[(String, SyncOutcome)],
    ) {
        let width = results
            .iter()
            .map(|(name, _)| UnicodeWidthStr::width(name.as_str()))
            .max()
            .unwrap_or(0);
        let rows = results
            .iter()
            .map(|(name, outcome)| result_row(name, width, op, outcome))
            .collect();
        let title = format!(
            " {} results \u{b7} {} ",
            op.title(),
            Local::now().format("%H:%M")
        );
        self.overlay = Overlay::SyncResults(SelectModal::new(title, rows, 0));
    }
}

/// One result row: the name padded to `width`, then the outcome.
fn result_row(
    name: &str,
    width: usize,
    op: SyncOp,
    outcome: &SyncOutcome,
) -> String {
    let padding = width.saturating_sub(UnicodeWidthStr::width(name));
    format!(
        "{name}{}{}",
        " ".repeat(padding + RESULT_GAP),
        outcome.describe(op)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::sync::SkipReason;

    #[test]
    fn result_rows_align_the_outcomes_after_the_widest_name() {
        let skipped = SyncOutcome::Skipped(SkipReason::Dirty);
        assert_eq!(
            result_row("hop", 6, SyncOp::Pull, &skipped),
            "hop     skipped: uncommitted changes"
        );
        assert_eq!(
            result_row("mdtask", 6, SyncOp::Push, &SyncOutcome::Updated),
            "mdtask  pushed"
        );
    }
}
