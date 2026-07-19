//! The view toggles the user flips - grouping, favourite floating, slugs, the
//! changes filter, the column set and the sort picker.

use super::*;

use crate::domain::sort::{SortDir, SortMode};
use crate::tui::widgets::SelectModal;

impl App {
    /// Toggles grouping for the current kind (grouped <-> flat) and persists it.
    pub(super) fn toggle_grouping(&mut self) {
        let grouped = self.view().grouped;
        self.view_mut().grouped = !grouped;
        let len = self.ordered_view().len();
        self.clamp_cursor(len);
        self.save_ui_state();
    }

    /// Toggles floating favourites for the current kind and persists it.
    pub(super) fn toggle_fav_float(&mut self) {
        let fav_float = self.view().fav_float;
        self.view_mut().fav_float = !fav_float;
        let len = self.ordered_view().len();
        self.clamp_cursor(len);
        self.save_ui_state();
    }

    /// The modes the sort picker offers: the four general ones, then the
    /// columns of the active set - so a user only sorts by what is on screen.
    pub(super) fn sort_modes(&self) -> Vec<SortMode> {
        let mut modes = vec![
            SortMode::Name,
            SortMode::Recent,
            SortMode::Frecency,
            SortMode::Custom,
        ];
        modes.extend_from_slice(self.columns().sort_modes());
        modes
    }

    /// Opens the sort picker, with the cursor on the active mode.
    pub(super) fn open_sort_picker(&mut self) {
        let (sort, dir) = (self.sort(), self.sort_dir());
        let modes = self.sort_modes();
        let cursor = modes.iter().position(|m| *m == sort).unwrap_or(0);
        let items: Vec<String> = modes
            .iter()
            .map(|mode| {
                if *mode == sort {
                    format!("{}  {}", mode.title(), dir.arrow())
                } else {
                    mode.title().to_string()
                }
            })
            .collect();
        self.overlay =
            Overlay::Sort(SelectModal::new(" Sort by ", items, cursor), modes);
    }

    /// Applies a picked sort mode. Re-picking the active column flips the
    /// direction; a fresh statistics column starts descending, because "which
    /// is the biggest" is the question it answers.
    pub(super) fn apply_sort(&mut self, mode: SortMode) {
        let dir = if mode == self.sort() {
            self.sort_dir().flip()
        } else if mode.is_statistic() {
            SortDir::Desc
        } else {
            SortDir::Asc
        };
        let view = self.view_mut();
        view.sort = mode;
        view.sort_dir = dir;
        self.save_ui_state();
    }

    /// Cycles the table's column set and starts the worker the new set needs.
    pub(super) fn cycle_columns(&mut self) {
        let next = self.columns().next(self.tab);
        self.view_mut().columns = next;
        self.save_ui_state();
        self.start_stats();
    }

    /// Toggles the Slug column display and persists it for the next run.
    pub(super) fn toggle_slugs(&mut self) {
        self.show_slugs = !self.show_slugs;
        self.save_ui_state();
    }

    /// Toggles the changes-only filter (git entries with a status change),
    /// keeping the cursor in range. Session-only (not persisted).
    pub(super) fn toggle_changes_only(&mut self) {
        self.changes_only = !self.changes_only;
        self.clear_selection();
        let len = self.ordered_view().len();
        self.clamp_cursor(len);
        self.set_status(if self.changes_only {
            "showing changed repos only"
        } else {
            "showing all entries"
        });
    }
}
