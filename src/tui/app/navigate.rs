//! Moving around: the cursor, the tabs and the multi-selection.

use super::*;

use ratada::nav::cycle;

use crate::domain::filter::{Tab, TabKind};
use crate::domain::sections::Row;
use crate::domain::sort::SortMode;

impl App {
    /// Selects `kind`: switches to its active view, or toggles between active
    /// and archive when that kind is already showing (the double-press of
    /// `1`/`2`).
    pub(super) fn select_kind(&mut self, kind: TabKind) {
        let target = if self.tab.kind() == kind {
            self.tab.toggle_archived()
        } else {
            kind.active_tab()
        };
        self.switch_tab(target);
    }

    /// Switches to `tab`, remembering the current tab's cursor entry and
    /// restoring the target tab's, clearing the selection and persisting state.
    pub(super) fn switch_tab(&mut self, tab: Tab) {
        if tab == self.tab {
            return;
        }
        self.remember_focus();
        self.tab = tab;
        self.clear_selection();
        // Each tab is a distinct list; drop the previous tab's scroll offset.
        self.list_offset.set(0);
        self.table_offset.set(0);
        self.restore_focus();
        let clamped = self.columns().available_on(tab);
        self.view_mut().columns = clamped;
        self.preview_scroll.reset();
        self.save_ui_state();
        self.start_stats();
        self.refresh_tab_on_first_visit();
    }

    /// Cycles to the next/previous active tab (`Tab`/`Shift+Tab`). Archives are
    /// not part of the cycle; an archive view normalises to its active sibling
    /// first.
    pub(super) fn cycle_tab(&mut self, delta: isize) {
        let base = self.tab.active();
        let current = Tab::ACTIVE.iter().position(|t| *t == base).unwrap_or(0);
        let next = cycle(current, Tab::ACTIVE.len(), delta);
        self.switch_tab(Tab::ACTIVE[next]);
    }

    /// Records the current tab's cursor entry by path, to restore on return.
    pub(super) fn remember_focus(&mut self) {
        if let Some(path) = self.focus_path() {
            self.tab_focus.insert(self.tab, path);
        }
    }

    /// Restores the cursor to the remembered entry for the current tab (by
    /// path), or the top when none is remembered or it is no longer visible.
    pub(super) fn restore_focus(&mut self) {
        self.cursor = 0;
        let remembered = self.tab_focus.get(&self.tab).cloned();
        self.refocus(remembered);
    }

    /// Runs the per-tab first-visit work: the files tabs check that their paths
    /// still exist; the git tabs refresh status (without fetching), mirroring
    /// the startup refresh of the initially active tab. Each runs once per
    /// session; a git refresh is deferred while another is in flight (and
    /// retried when it finishes) so switching tabs never aborts it.
    pub(super) fn refresh_tab_on_first_visit(&mut self) {
        if self.refreshed_tabs.contains(&self.tab) {
            return;
        }
        if self.tab.kind() == TabKind::Files {
            self.refreshed_tabs.insert(self.tab);
            if !self.config.example_mode {
                self.check_files_existence();
            }
            return;
        }
        if !self.auto_refresh || self.is_refreshing() {
            return;
        }
        self.start_refresh(false);
    }

    /// Moves the cursor cyclically within the current view; a plain move drops
    /// the range anchor so the next `Shift`-move re-anchors at the cursor.
    pub(super) fn move_cursor(&mut self, delta: isize) {
        let len = self.row_count();
        self.cursor = cycle(self.cursor, len, delta);
        self.anchor = None;
    }

    /// Moves the cursor by `delta` without wrapping, clamped into the view.
    pub(super) fn move_clamped(&mut self, delta: isize) {
        let len = self.row_count();
        if len == 0 {
            return;
        }
        self.cursor = ratada::nav::step_clamped(self.cursor, len, delta);
        self.anchor = None;
    }

    /// Jumps the cursor to the first (`g`) or last (`G`) entry.
    pub(super) fn cursor_to_edge(&mut self, to_end: bool) {
        let len = self.row_count();
        self.cursor = if to_end { len.saturating_sub(1) } else { 0 };
        self.anchor = None;
    }

    /// Moves the cursor by whole (`pages` != 0) or half pages, using the last
    /// rendered list height.
    pub(super) fn page(&mut self, pages: isize, half: bool) {
        let height = self.list_height.get().max(1) as isize;
        let step = if half { (height / 2).max(1) } else { height };
        self.move_clamped(pages.signum() * step);
    }

    /// Toggles the selection of the cursor entry and re-anchors the range.
    pub(super) fn toggle_select(&mut self) {
        if let Some(index) = self.selected_index() {
            if !self.selected.remove(&index) {
                self.selected.insert(index);
            }
            self.anchor = Some(self.cursor);
        }
    }

    /// Clears the multi-selection.
    pub(super) fn clear_selection(&mut self) {
        self.selected.clear();
        self.anchor = None;
    }

    /// Extends the range selection by moving the cursor (clamped, not cyclic)
    /// and selecting every row between the anchor and the cursor.
    pub(super) fn extend_selection(&mut self, delta: isize) {
        let rows = self.cursor_rows();
        if rows.is_empty() {
            return;
        }
        let anchor = *self.anchor.get_or_insert(self.cursor);
        let last = rows.len() - 1;
        let new =
            (self.cursor as isize + delta).clamp(0, last as isize) as usize;
        self.cursor = new;
        let (lo, hi) = (anchor.min(new).min(last), anchor.max(new).min(last));
        // Only visible entries join the range; a collapsed header is skipped.
        self.selected = rows[lo..=hi]
            .iter()
            .filter_map(|row| match row {
                Row::Entry(index) => Some(*index),
                Row::Collapsed(_) => None,
            })
            .collect();
    }

    /// Extends the range selection by a whole screen page, the `Shift`+page
    /// counterpart to `Shift`+arrow.
    pub(super) fn extend_selection_page(&mut self, pages: isize) {
        let height = self.list_height.get().max(1) as isize;
        self.extend_selection(pages.signum() * height);
    }

    /// Moves the cursor entry within the custom order (only in custom sort).
    pub(super) fn move_entry(&mut self, delta: isize) {
        // Manual reorder only makes sense in custom sort; every other mode
        // orders automatically.
        if self.sort() != SortMode::Custom {
            self.set_status("switch to custom sort (t) to reorder");
            return;
        }
        let rows = self.cursor_rows();
        let neighbor = self.cursor as isize + delta;
        if neighbor < 0 || neighbor as usize >= rows.len() {
            return;
        }
        // Both ends must be entries: a collapsed header never swaps.
        let (Some(Row::Entry(a)), Some(Row::Entry(b))) =
            (rows.get(self.cursor), rows.get(neighbor as usize))
        else {
            return;
        };
        let (a, b) = (*a, *b);
        let repos = self.service.repos();
        // While favourites float, stay within the fav / non-fav segment.
        if self.fav_float() && repos[a].fav != repos[b].fav {
            return;
        }
        // In the grouped view, only reorder within the same section.
        if self.is_sectioned() && repos[a].section != repos[b].section {
            return;
        }
        let moved = repos[a].path.clone();
        if self.service.swap_entries(a, b).is_ok() {
            self.refocus(Some(moved));
        }
    }
}
