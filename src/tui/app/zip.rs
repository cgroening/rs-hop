//! The background ZIP-backup worker and how its result is reported.

use super::progress::{REFRESH_LABEL, ZIP_LABEL};
use super::*;
use unicode_width::UnicodeWidthStr;

use std::path::{Path, PathBuf};
use std::sync::mpsc::TryRecvError;

use chrono::{DateTime, Local};

use crate::domain::backup;
use crate::domain::repo::Repo;
use crate::service::zip_service::{ZipJob, spawn_zip};
use crate::util::paths::expand_tilde;

/// The destination archive path for `repo` in `folder`: a unique, slugified
/// file name (see [`crate::domain::backup::backup_filename`]).
pub(super) fn backup_dest(
    folder: &Path,
    repo: &Repo,
    repos: &[Repo],
) -> PathBuf {
    folder.join(backup::backup_filename(repo, repos))
}

impl App {
    /// Zips the target git entries (selection or cursor) into the backup folder.
    /// Non-git entries are ignored; a lone non-git cursor reports a hint.
    pub(super) fn zip_targets(&mut self) {
        // An explicit single/selection backup ignores the include flag: the
        // user is targeting these entries on purpose. Git repos and folders
        // both qualify; missing/non-folder paths are dropped in `start_zip`.
        let targets = self.targets();
        if targets.is_empty() {
            self.set_status("nothing to zip");
            return;
        }
        self.start_zip(&targets);
        self.clear_selection();
    }

    /// Zips every entry (across all tabs) that opts into the "backup all" run
    /// into the backup folder: all git repos plus the file/folder entries whose
    /// backup toggle is on.
    pub(super) fn zip_all(&mut self) {
        let indices: Vec<usize> = self
            .service
            .repos()
            .iter()
            .enumerate()
            .filter(|(_, repo)| repo.include_in_backup)
            .map(|(index, _)| index)
            .collect();
        if indices.is_empty() {
            self.set_status("nothing to zip");
            return;
        }
        self.start_zip(&indices);
    }

    /// Starts a background ZIP backup of the repos at `indices`, showing the
    /// progress bar. Refuses to start while another refresh or backup runs.
    pub(super) fn start_zip(&mut self, indices: &[usize]) {
        if self.loading.is_some() {
            self.set_status("busy: a refresh or backup is running");
            return;
        }
        let Some(folder) = self.config.zip_backup_folder.as_deref() else {
            self.set_status("no zip_backup_folder configured");
            return;
        };
        let folder = expand_tilde(folder);
        if let Err(error) = std::fs::create_dir_all(&folder) {
            self.set_status(format!("could not create backup folder: {error}"));
            return;
        }
        let repos = self.service.repos();
        let jobs: Vec<ZipJob> = indices
            .iter()
            .filter_map(|&i| repos.get(i))
            // Skip entries whose directory is gone (broken or offline drives),
            // so no empty archive is written for them.
            .filter(|repo| repo.path.is_dir())
            .map(|repo| ZipJob {
                src: repo.path.clone(),
                dest: backup_dest(&folder, repo, repos),
                name: repo.display_name(),
            })
            .collect();
        if jobs.is_empty() {
            self.set_status("nothing to zip (paths missing?)");
            return;
        }
        let count = jobs.len();
        self.set_status(if count == 1 {
            "creating backup…".to_string()
        } else {
            format!("creating {count} backups…")
        });
        self.loading_name_width = jobs
            .iter()
            .map(|job| UnicodeWidthStr::width(job.name.as_str()))
            .max()
            .unwrap_or(0);
        self.loading = Some((0, 0));
        self.loading_label = ZIP_LABEL;
        self.loading_detail = None;
        self.zip_rx = Some(spawn_zip(
            jobs,
            self.config.zip_exclude_dirs.clone(),
            self.zip_cache_path.clone(),
        ));
    }

    pub(super) fn drain_zip(&mut self) {
        let Some(rx) = self.zip_rx.take() else {
            return;
        };
        let mut summary = None;
        let mut disconnected = false;
        loop {
            match rx.try_recv() {
                Ok(update) => {
                    self.loading = Some((update.done, update.total));
                    if update.label.is_some() {
                        self.loading_detail = update.label;
                    }
                    if update.finished {
                        summary = Some((
                            update.archives,
                            update.unchanged,
                            update.errors,
                        ));
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if disconnected || summary.is_some() {
            self.loading = None;
            self.loading_label = REFRESH_LABEL;
            self.loading_detail = None;
            self.reload_zip_backups();
            if let Some((archives, unchanged, errors)) = summary {
                self.report_zip_done(archives, unchanged, errors);
            }
        } else {
            self.zip_rx = Some(rx);
        }
    }

    /// Reports the outcome of a finished ZIP-backup run.
    pub(super) fn report_zip_done(
        &mut self,
        archives: usize,
        unchanged: usize,
        errors: usize,
    ) {
        // Nothing written and nothing failed: every target was already current.
        if archives == 0 && errors == 0 && unchanged > 0 {
            self.set_status("backup up to date (no changes)");
            return;
        }
        let folder =
            self.config.zip_backup_folder.as_deref().unwrap_or_default();
        let mut message = if archives == 1 {
            format!("backed up 1 archive to {folder}")
        } else {
            format!("backed up {archives} archives to {folder}")
        };
        if unchanged > 0 {
            message.push_str(&format!(" ({unchanged} unchanged)"));
        }
        if errors > 0 {
            message.push_str(&format!(" ({errors} failed)"));
        }
        self.set_status(message);
    }

    /// Whether a background ZIP backup is currently running.
    pub(super) fn is_zipping(&self) -> bool {
        self.zip_rx.is_some()
    }

    /// Rebuilds the per-repo last-backup times from the backup folder (once at
    /// start and after each backup; never per frame).
    pub(super) fn reload_zip_backups(&mut self) {
        self.zip_backups.clear();
        let Some(folder) = self.config.zip_backup_folder.as_deref() else {
            return;
        };
        let folder = expand_tilde(folder);
        let repos = self.service.repos();
        // Both git repos and file/folder entries can have a backup archive.
        for repo in repos.iter() {
            let dest = backup_dest(&folder, repo, repos);
            if let Ok(meta) = std::fs::metadata(&dest)
                && let Ok(modified) = meta.modified()
            {
                self.zip_backups.insert(
                    repo.path.clone(),
                    DateTime::<Local>::from(modified),
                );
            }
        }
    }
}
