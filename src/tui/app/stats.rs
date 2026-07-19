//! The code and history statistics workers and the cache they fill.
//!
//! The two families are independent: the `Code` columns never need git, and
//! `Activity` never needs a source walk.

use super::*;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self};

use crate::service::stats_service::{
    self, CodeUpdate, GitStatsUpdate, spawn_code_stats, spawn_git_stats,
};

impl App {
    /// Applies any pending background ZIP-backup progress without blocking.
    /// Starts the worker the active column set needs, over the paths currently
    /// shown. `Standard` starts nothing at all, so a user who never opens the
    /// statistics never pays for a source walk.
    ///
    /// In example mode no worker ever runs; the cells fall back to a dash
    /// rather than spinning forever.
    pub(super) fn start_stats(&mut self) {
        self.code_rx = None;
        self.git_stats_rx = None;
        self.computing.clear();
        if !self.columns().is_statistics() || self.config.example_mode {
            return;
        }
        let paths: Vec<PathBuf> = self
            .ordered_view()
            .iter()
            .filter_map(|&i| self.service.get(i))
            .map(|repo| repo.path.clone())
            .collect();
        if paths.is_empty() {
            return;
        }
        self.computing = paths.iter().cloned().collect();
        if self.columns().needs_code_stats() {
            self.code_rx = Some(spawn_code_stats(
                paths,
                self.config.zip_exclude_dirs.clone(),
            ));
        } else if self.columns().needs_git_stats() {
            self.git_stats_rx =
                Some(spawn_git_stats(Arc::clone(&self.git_client), paths));
        }
    }

    /// Drains the background code statistics into the cache.
    pub(super) fn drain_code_stats(&mut self) {
        let Some(rx) = self.code_rx.take() else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(CodeUpdate::Started { .. }) => {}
                Ok(CodeUpdate::Done { path, stats }) => {
                    self.computing.remove(&path);
                    self.stats.code.insert(path, *stats);
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.code_rx = Some(rx);
                    return;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.finish_stats();
                    return;
                }
            }
        }
    }

    /// Drains the background history statistics into the cache.
    pub(super) fn drain_git_stats(&mut self) {
        let Some(rx) = self.git_stats_rx.take() else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(GitStatsUpdate::Started { .. }) => {}
                Ok(GitStatsUpdate::Done { path, stats }) => {
                    self.computing.remove(&path);
                    self.stats.git.insert(path, stats);
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.git_stats_rx = Some(rx);
                    return;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.finish_stats();
                    return;
                }
            }
        }
    }

    /// Persists the statistics cache once a worker has finished.
    pub(super) fn finish_stats(&mut self) {
        self.computing.clear();
        if let Err(error) =
            stats_service::save_cache(&self.stats_path, &self.stats)
        {
            log::warn!("could not write the stats cache: {error}");
        }
    }

    /// Whether a statistics worker is still running.
    pub(super) fn is_computing(&self) -> bool {
        self.code_rx.is_some() || self.git_stats_rx.is_some()
    }

    /// The summed statistics of the entries currently shown, so the totals row
    /// follows the filter.
    pub(super) fn visible_totals(&self) -> crate::domain::stats::Totals {
        let entries = self
            .ordered_view()
            .into_iter()
            .filter_map(|i| self.service.get(i))
            .filter_map(|repo| self.stats.code.get(&repo.path));
        crate::domain::stats::totals(entries)
    }
}
