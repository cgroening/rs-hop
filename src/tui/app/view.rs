//! What is currently visible: the tab's entries in display order, the
//! selection they resolve to, and the per-kind view settings behind them.

use super::*;

use std::path::PathBuf;

use chrono::Local;

use ratada::shortcut_hints;

use crate::domain::filter::{TabKind, belongs_to_tab, fuzzy_indices};
use crate::domain::repo::{Repo, RepoKind};
use crate::domain::sections;
use crate::domain::sort::{
    SortContext, SortDir, SortMode, StatsLookup, sort_indices,
};
use crate::service::ui_state_service::{self, TabView, UiState};
use crate::tui::columns::ColumnSet;
use crate::tui::git_columns::effective_info;

/// Whether `repo` passes the changes-only filter: non-git entries always pass;
/// a git entry passes only when its (live or example) status is not clean.
pub(super) fn repo_has_change(repo: &Repo, example_mode: bool) -> bool {
    if repo.kind != RepoKind::Git {
        return true;
    }
    effective_info(repo, example_mode).is_some_and(|info| !info.is_clean())
}

impl App {
    /// The service indices belonging to the current tab (unsorted, unfiltered).
    pub(super) fn tab_indices(&self) -> Vec<usize> {
        let repos = self.service.repos();
        (0..repos.len())
            .filter(|&i| belongs_to_tab(&repos[i], self.tab))
            .collect()
    }

    /// The current tab's kind's view settings.
    pub(super) fn view(&self) -> &TabState {
        &self.tab_state[self.tab.kind_index()]
    }

    /// The current tab's kind's view settings, mutably.
    pub(super) fn view_mut(&mut self) -> &mut TabState {
        &mut self.tab_state[self.tab.kind_index()]
    }

    /// The active sort mode for the current tab.
    pub(super) fn sort(&self) -> SortMode {
        self.view().sort
    }

    /// The active sort direction for the current tab.
    pub(super) fn sort_dir(&self) -> SortDir {
        self.view().sort_dir
    }

    /// The active column set for the current tab.
    pub(super) fn columns(&self) -> ColumnSet {
        self.view().columns
    }

    /// Whether the current tab groups entries into sections.
    pub(super) fn grouped(&self) -> bool {
        self.view().grouped
    }

    /// Whether the current tab floats favourites to the top.
    pub(super) fn fav_float(&self) -> bool {
        self.view().fav_float
    }

    /// The ordered service indices visible in the current tab, after the sort
    /// or live fuzzy filter.
    pub(super) fn ordered_view(&self) -> Vec<usize> {
        let repos = self.service.repos();
        let tab_indices = self.tab_indices();
        let query = self.filter.value();
        let mut indices = if self.filtering_active() {
            let subset: Vec<Repo> =
                tab_indices.iter().map(|&i| repos[i].clone()).collect();
            fuzzy_indices(&subset, query)
                .into_iter()
                .map(|pos| tab_indices[pos])
                .collect()
        } else if self.is_sectioned() {
            // Grouped view: entries by section, sorted within each group by the
            // active sort mode (favourites floated per the fav-float toggle).
            sections::flatten(&self.section_groups())
        } else {
            // Flat view: the chosen sort mode over the whole tab.
            let mut indices = tab_indices;
            sort_indices(repos, &mut indices, &self.sort_context());
            indices
        };
        if self.changes_only {
            indices.retain(|&i| self.shows_change(&repos[i]));
        }
        indices
    }

    /// Whether `repo` passes the changes-only filter (see [`repo_has_change`]).
    pub(super) fn shows_change(&self, repo: &Repo) -> bool {
        repo_has_change(repo, self.config.example_mode)
    }

    /// Whether the live fuzzy filter is currently narrowing the list.
    pub(super) fn filtering_active(&self) -> bool {
        self.filtering && !self.filter.value().trim().is_empty()
    }

    /// Whether the current view groups entries into sections (the grouping
    /// toggle is on and no live filter is narrowing the list).
    pub(super) fn is_sectioned(&self) -> bool {
        self.grouped() && !self.filtering_active()
    }

    /// The display-ordered sections for the current tab: entries grouped by
    /// section (in the kind's stored section order), each group sorted by the
    /// active sort mode, with Ungrouped last. Sorting the flat index list first
    /// carries the order into each bucket (grouping keeps input order).
    pub(super) fn section_groups(&self) -> Vec<sections::SectionGroup> {
        let repos = self.service.repos();
        let mut indices = self.tab_indices();
        sort_indices(repos, &mut indices, &self.sort_context());
        sections::group(
            self.service.sections(self.tab.repo_kind()),
            &indices,
            |i| repos[i].section.clone(),
        )
    }

    /// The selected service index, if the view is non-empty.
    pub(super) fn selected_index(&self) -> Option<usize> {
        self.ordered_view().get(self.cursor).copied()
    }

    /// Clamps the cursor into the current view length.
    pub(super) fn clamp_cursor(&mut self, view_len: usize) {
        if view_len == 0 {
            self.cursor = 0;
        } else if self.cursor >= view_len {
            self.cursor = view_len - 1;
        }
    }

    /// Everything a sort needs, borrowed from the statistics caches.
    pub(super) fn sort_context(&self) -> SortContext<'_> {
        SortContext {
            mode: self.sort(),
            dir: self.sort_dir(),
            float_favs: self.fav_float(),
            now: Local::now().timestamp(),
            stats: StatsLookup {
                code: &self.stats.code,
                git: &self.stats.git,
            },
        }
    }

    /// The persisted per-kind view block for tab-state index `i`.
    pub(super) fn tab_view_of(&self, i: usize) -> TabView {
        let state = &self.tab_state[i];
        TabView {
            sort: state.sort,
            sort_dir: state.sort_dir,
            columns: state.columns.as_key().to_string(),
            grouped: state.grouped,
            fav_float: state.fav_float,
        }
    }

    /// Persists the per-kind view settings, active tab, slug display, preview
    /// mode and whether the hint footer is shown.
    pub(super) fn save_ui_state(&self) {
        let _ = ui_state_service::save(
            &self.ui_state_path,
            &UiState {
                git: self.tab_view_of(TabKind::Git.index()),
                files: self.tab_view_of(TabKind::Files.index()),
                tab: self.tab,
                show_slugs: self.show_slugs,
                preview: self.preview.as_key().to_string(),
                preview_width_pct: self.preview.width_pct,
                preview_height_rows: self.preview.height_rows,
                hints_visible: shortcut_hints::visible(),
            },
        );
    }

    /// The paths of the target entries (the selection, or the cursor entry when
    /// nothing is selected), in list order.
    pub(super) fn target_paths(&self) -> Vec<String> {
        self.targets()
            .into_iter()
            .filter_map(|index| self.service.get(index))
            .map(|repo| repo.path.to_string_lossy().into_owned())
            .collect()
    }

    /// The entries an action applies to: the multi-selection, or the cursor
    /// entry when nothing is selected. Sorted ascending.
    pub(super) fn targets(&self) -> Vec<usize> {
        if self.selected.is_empty() {
            return self.selected_index().into_iter().collect();
        }
        let mut indices: Vec<usize> = self.selected.iter().copied().collect();
        indices.sort_unstable();
        indices
    }

    /// The path of the entry under the cursor, if any.
    pub(super) fn cursor_path(&self) -> Option<PathBuf> {
        self.selected_index()
            .and_then(|index| self.service.get(index))
            .map(|repo| repo.path.clone())
    }

    /// Moves the cursor onto the entry with `path`, if it is still visible.
    pub(super) fn refocus(&mut self, path: Option<PathBuf>) {
        let Some(path) = path else {
            return;
        };
        let view = self.ordered_view();
        let repos = self.service.repos();
        if let Some(pos) = view.iter().position(|&i| repos[i].path == path) {
            self.cursor = pos;
        }
    }
}
