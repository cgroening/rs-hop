//! The background git-status workers and the path-error state they feed.
//!
//! Several refreshes may be in flight at once, so starting one never cancels
//! another; the run loop drains them all each pass.

use super::progress::REFRESH_LABEL;
use super::*;
use unicode_width::UnicodeWidthStr;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::TryRecvError;
use std::time::Instant;

use chrono::Local;

use ratada::spinner::Spinner;

use crate::domain::repo::{self, Repo, RepoKind};
use crate::service::status_service::{self, StatusUpdate, spawn_refresh};

impl App {
    /// Starts a background refresh over the current tab's entries (with the
    /// progress bar).
    pub(super) fn start_refresh(&mut self, fetch: bool) {
        self.refreshed_tabs.insert(self.tab);
        let paths: Vec<PathBuf> = self
            .tab_indices()
            .iter()
            .filter_map(|&i| self.service.get(i).map(|r| r.path.clone()))
            .collect();
        self.refresh_paths(paths, fetch, true);
    }

    /// Starts a background refresh over `paths`, optionally fetching first.
    /// `show_bar` drives the full-width progress bar (and the global
    /// `fetched_at` update); a subset refresh passes `false`.
    pub(super) fn refresh_paths(
        &mut self,
        paths: Vec<PathBuf>,
        fetch: bool,
        show_bar: bool,
    ) {
        if paths.is_empty() {
            return;
        }
        // Start the spinner only when nothing is already animating, so a running
        // refresh keeps its rhythm rather than jumping back to frame zero.
        if self.refreshing.is_empty() {
            self.spinner = Spinner::new();
            self.spinner_at = Instant::now();
        }
        self.refreshing.extend(paths.iter().cloned());
        if show_bar {
            self.loading_label = REFRESH_LABEL;
            self.loading_detail = None;
            self.loading = Some(match self.loading {
                Some((done, total)) => (done, total + paths.len()),
                None => (0, paths.len()),
            });
            self.loading_name_width =
                self.loading_name_width.max(self.max_name_width(&paths));
        }
        let remaining: HashSet<PathBuf> = paths.iter().cloned().collect();
        self.status_jobs.push(RefreshJob {
            rx: spawn_refresh(Arc::clone(&self.git_client), paths, fetch),
            // Only a full refresh updates the global "remote: fetched …" time.
            fetched: fetch && show_bar,
            bar: show_bar,
            remaining,
        });
    }

    /// Applies any pending background status updates without blocking. Drains
    /// every concurrent refresh job and drops the ones whose worker has ended.
    pub(super) fn drain_status(&mut self) {
        if self.status_jobs.is_empty() {
            return;
        }
        let mut jobs = std::mem::take(&mut self.status_jobs);
        let mut any_finished = false;
        let mut fetched_finished = false;
        jobs.retain_mut(|job| {
            loop {
                match job.rx.try_recv() {
                    Ok(StatusUpdate::Started { path }) => {
                        self.loading_detail = Some(self.name_for_path(&path));
                    }
                    Ok(StatusUpdate::Done { path, info }) => {
                        self.service.set_git_info(&path, info);
                        self.refreshing.remove(&path);
                        job.remaining.remove(&path);
                        if job.bar
                            && let Some((done, _)) = &mut self.loading
                        {
                            *done += 1;
                        }
                    }
                    Err(TryRecvError::Empty) => return true,
                    Err(TryRecvError::Disconnected) => {
                        // Clear any spinners the worker never reported on.
                        for path in job.remaining.drain() {
                            self.refreshing.remove(&path);
                        }
                        any_finished = true;
                        fetched_finished |= job.fetched;
                        return false;
                    }
                }
            }
        });
        self.status_jobs = jobs;
        if any_finished {
            self.finish_refresh(fetched_finished);
        }
        // The bar lives as long as any bar-owning job runs.
        if !self.status_jobs.iter().any(|job| job.bar) {
            self.loading = None;
            self.loading_detail = None;
            self.loading_name_width = 0;
        }
        if self.status_jobs.is_empty() {
            self.refreshing.clear();
            // A tab switched to mid-refresh deferred its first-visit refresh;
            // run it now that every refresh has drained.
            self.refresh_tab_on_first_visit();
        }
    }

    /// Persists the full status cache after a refresh job finishes.
    pub(super) fn finish_refresh(&mut self, fetched: bool) {
        let now = Local::now();
        self.cache_generated_at = Some(now);
        if fetched {
            self.last_fetched = Some(now);
        }
        // Persist the full current state (not just the finished job's paths),
        // so a single-entry refresh never drops the other entries' cache.
        let infos: Vec<(PathBuf, crate::domain::repo::GitInfo)> = self
            .service
            .repos()
            .iter()
            .filter_map(|repo| {
                repo.git_info.clone().map(|info| (repo.path.clone(), info))
            })
            .collect();
        let _ = status_service::save_cache(
            &self.cache_path,
            &infos,
            self.last_fetched,
        );
    }

    /// Whether any background status refresh is currently running.
    pub(super) fn is_refreshing(&self) -> bool {
        !self.status_jobs.is_empty()
    }

    /// The display name of the entry at `path`, or its basename as a fallback.
    pub(super) fn name_for_path(&self, path: &Path) -> String {
        self.service
            .repos()
            .iter()
            .find(|repo| repo.path == path)
            .map_or_else(|| repo::basename(path), Repo::display_name)
    }

    /// The widest display name among `paths`, used to reserve a stable block in
    /// the progress text so the `XX %` column does not move as names change.
    pub(super) fn max_name_width(&self, paths: &[PathBuf]) -> usize {
        paths
            .iter()
            .map(|path| {
                UnicodeWidthStr::width(self.name_for_path(path).as_str())
            })
            .max()
            .unwrap_or(0)
    }

    /// The current spinner frame glyph, if a refresh is running.
    pub(super) fn spinner_frame(&self) -> Option<&'static str> {
        if self.refreshing.is_empty() {
            return None;
        }
        Some(self.spinner.frame(self.config.appearance.glyphs))
    }

    /// Restarts the full background refresh, optionally fetching first. When the
    /// preview is visible, the current tab's git logs are reloaded too.
    pub(super) fn reload_status(&mut self, fetch: bool) {
        if self.config.example_mode {
            self.set_status("example mode: live status is off");
            return;
        }
        self.set_status(if fetch {
            "fetching and reloading status…"
        } else {
            "reloading status…"
        });
        self.start_refresh(fetch);
        if self.preview.visible {
            let paths = self.tab_git_paths();
            self.invalidate_logs(&paths);
            self.fetch_logs(paths);
        }
    }

    /// The current tab's git entry paths (for reloading preview logs).
    pub(super) fn tab_git_paths(&self) -> Vec<PathBuf> {
        self.tab_indices()
            .iter()
            .filter_map(|&i| self.service.get(i))
            .filter(|repo| repo.kind == RepoKind::Git)
            .map(|repo| repo.path.clone())
            .collect()
    }

    /// Refreshes the target entries in the background, optionally fetching
    /// first. The global "remote: fetched …" line is left untouched.
    pub(super) fn refresh_targets(&mut self, fetch: bool) {
        if self.config.example_mode {
            self.set_status("example mode: live status is off");
            return;
        }
        let targets = self.targets();
        let paths: Vec<PathBuf> = targets
            .iter()
            .filter_map(|&i| self.service.get(i).map(|r| r.path.clone()))
            .collect();
        if paths.is_empty() {
            return;
        }
        let message = if targets.len() == 1 {
            let name = self
                .service
                .get(targets[0])
                .map_or_else(String::new, Repo::display_name);
            format!("refreshing {name}…")
        } else {
            format!("refreshing {} entries…", targets.len())
        };
        self.set_status(message);
        self.refresh_paths(paths, fetch, false);
        if self.preview.visible {
            let git_paths: Vec<PathBuf> = targets
                .iter()
                .filter_map(|&i| self.service.get(i))
                .filter(|repo| repo.kind == RepoKind::Git)
                .map(|repo| repo.path.clone())
                .collect();
            self.invalidate_logs(&git_paths);
            self.fetch_logs(git_paths);
        }
        self.clear_selection();
    }

    /// The number of current-tab entries flagged with a path error.
    pub(super) fn error_count(&self) -> usize {
        let repos = self.service.repos();
        self.tab_indices()
            .iter()
            .filter(|&&i| self.path_error(&repos[i]).is_some())
            .count()
    }

    /// The path error for `repo`, if any. A git entry reports a missing or
    /// invalid repository from its gathered git info (set by the background
    /// refresh, so no filesystem stat happens here); a file/folder entry only
    /// reports a missing path once the on-demand existence check (`r` on the
    /// Files tab) has flagged it.
    pub(super) fn path_error(&self, repo: &Repo) -> Option<String> {
        if self.config.example_mode {
            // Example mode shows curated demo data, so a git entry's error comes
            // from its example info; path entries are never checked on disk.
            return match repo.kind {
                RepoKind::Git => repo.example_error(),
                RepoKind::Path => None,
            };
        }
        match repo.kind {
            RepoKind::Git => repo.entry_error(),
            RepoKind::Path => self
                .files_missing
                .contains(&repo.path)
                .then(|| repo::PATH_NOT_FOUND.to_string()),
        }
    }

    /// Clears the error state of a just-repaired entry, whichever tab it lives
    /// on. The error list spans every tab, so a tab-wide refresh would miss an
    /// entry repaired from another tab: re-stat the file/folder entries (clears
    /// the missing marker) and refresh just the repaired path's git status.
    pub(super) fn clear_repaired_error(&mut self, path: PathBuf) {
        self.recheck_files();
        self.refresh_paths(vec![path], false, false);
    }

    /// Re-stats every file/folder entry, recording the ones whose path is
    /// missing so the marker and error count reflect the current filesystem.
    pub(super) fn recheck_files(&mut self) {
        self.files_missing = self
            .service
            .repos()
            .iter()
            .filter(|repo| repo.kind == RepoKind::Path && !repo.path.exists())
            .map(|repo| repo.path.clone())
            .collect();
    }

    /// Checks on disk which file/folder entries are missing, recording them so
    /// the marker and the error count reflect the result. Triggered by `r` on
    /// the Files tab; never on start. Reports a transient summary.
    pub(super) fn check_files_existence(&mut self) {
        self.recheck_files();
        let missing = self.files_missing.len();
        self.set_status(if missing == 0 {
            "checked paths: all exist".to_string()
        } else if missing == 1 {
            "checked paths: 1 missing".to_string()
        } else {
            format!("checked paths: {missing} missing")
        });
    }
}
