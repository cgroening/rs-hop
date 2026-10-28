//! The sections of the grouped view: jumping between them, folding them down
//! to their header, and the overlay that manages the current kind's list.

use super::*;

use crate::domain::sections::{self, Row};
use crate::tui::sections_modal::{SectionsAction, SectionsModal};
use crate::tui::widgets::{ConfirmModal, SelectModal, TextPrompt};

impl App {
    /// Opens the jump-to-section picker for the Files tab.
    pub(super) fn open_section_jump(&mut self) {
        let groups = self.visible_groups();
        if groups.len() < 2 {
            self.set_status("no other sections");
            return;
        }
        let starts = self.section_row_starts();
        let labels: Vec<String> =
            groups.iter().map(|g| g.label.clone()).collect();
        let current =
            sections::current_section(&starts, self.cursor).unwrap_or(0);
        self.overlay = Overlay::SectionJump(
            SelectModal::new("Jump to section", labels, current),
            starts,
        );
    }

    /// Moves the cursor to the start of the next section, wrapping from the
    /// last to the first.
    pub(super) fn jump_to_next_section(&mut self) {
        let next =
            sections::next_start(&self.section_row_starts(), self.cursor);
        self.jump_to_row(next);
    }

    /// Moves the cursor to the start of its own section, or from there to the
    /// start of the previous one (wrapping from the first to the last).
    pub(super) fn jump_to_prev_section(&mut self) {
        let prev =
            sections::prev_start(&self.section_row_starts(), self.cursor);
        self.jump_to_row(prev);
    }

    /// Collapses the section under the cursor onto its header, or expands it
    /// when the cursor already sits on its collapsed header.
    pub(super) fn toggle_collapse(&mut self) {
        let Some((position, group)) = self.cursor_group() else {
            return;
        };
        let label = group.label;
        if self.is_collapsed(&label) {
            self.view_mut().collapsed.remove(&label);
        } else {
            self.view_mut().collapsed.insert(label);
        }
        self.settle_on_section_start(position);
    }

    /// Expands the collapsed section whose header is under the cursor; the
    /// cursor stays put and so lands on its first entry.
    pub(super) fn expand_at_cursor(&mut self) {
        if let Some((_, group)) = self.cursor_group() {
            self.view_mut().collapsed.remove(&group.label);
            self.save_ui_state();
        }
    }

    /// Collapses every shown section, or expands them all when none is open.
    pub(super) fn toggle_collapse_all(&mut self) {
        let position = self.cursor_group().map_or(0, |(position, _)| position);
        let labels: Vec<String> = self
            .visible_groups()
            .into_iter()
            .map(|group| group.label)
            .collect();
        let has_open = labels.iter().any(|label| !self.is_collapsed(label));
        let collapsed = &mut self.view_mut().collapsed;
        for label in labels {
            if has_open {
                collapsed.insert(label);
            } else {
                collapsed.remove(&label);
            }
        }
        self.settle_on_section_start(position);
    }

    /// Whether the cursor sits on a collapsed section header.
    pub(super) fn is_cursor_on_collapsed(&self) -> bool {
        matches!(self.cursor_row(), Some(Row::Collapsed(_)))
    }

    /// The row at which each shown section starts.
    fn section_row_starts(&self) -> Vec<usize> {
        sections::row_starts(&self.visible_groups(), |label| {
            self.is_collapsed(label)
        })
    }

    /// The shown section holding the cursor row, with its position among the
    /// shown sections.
    fn cursor_group(&self) -> Option<(usize, sections::SectionGroup)> {
        let starts = self.section_row_starts();
        let current = sections::current_section(&starts, self.cursor)?;
        let group = self.visible_groups().into_iter().nth(current)?;
        Some((current, group))
    }

    /// Moves the cursor to `row` (when there is one) and drops the range
    /// anchor, like any plain move.
    fn jump_to_row(&mut self, row: Option<usize>) {
        if let Some(row) = row {
            self.cursor = row;
            self.anchor = None;
        }
    }

    /// Puts the cursor on the start of the shown section at `position` (the
    /// one it was in before a fold changed the rows) and persists the fold.
    fn settle_on_section_start(&mut self, position: usize) {
        let starts = self.section_row_starts();
        self.cursor = starts.get(position).copied().unwrap_or(0);
        self.anchor = None;
        self.save_ui_state();
    }

    /// Opens the manage-sections overlay at `cursor` over the current kind's
    /// sections.
    pub(super) fn open_sections_manager_at(&mut self, cursor: usize) {
        let names = self.service.sections(self.tab.repo_kind()).to_vec();
        self.overlay = Overlay::Sections(SectionsModal::new(names, cursor));
    }

    /// Opens the manage-sections overlay at the first section.
    pub(super) fn open_sections_manager(&mut self) {
        self.open_sections_manager_at(0);
    }

    /// Runs a section-manager action, reporting errors and re-opening the
    /// manager (or a sub-prompt) afterwards.
    pub(super) fn run_sections_action(
        &mut self,
        modal: SectionsModal,
        action: SectionsAction,
    ) {
        match action {
            SectionsAction::Pending => {
                self.overlay = Overlay::Sections(modal);
            }
            SectionsAction::Close => {}
            SectionsAction::New => {
                self.overlay = Overlay::SectionPrompt(
                    TextPrompt::new("New section", "name", ""),
                    SectionPromptKind::New,
                );
            }
            SectionsAction::Rename(old) => {
                self.overlay = Overlay::SectionPrompt(
                    TextPrompt::new("Rename section", "name", &old),
                    SectionPromptKind::Rename(old),
                );
            }
            SectionsAction::Delete(name) => {
                let message = format!(
                    "Delete section \"{name}\"? Entries become Ungrouped."
                );
                self.overlay = Overlay::SectionDelete(
                    ConfirmModal::new("Delete section", message),
                    name,
                );
            }
            SectionsAction::Move { from, to } => {
                let repo_kind = self.tab.repo_kind();
                if let Err(error) =
                    self.service.move_section(repo_kind, from, to)
                {
                    self.set_status(format!("{error}"));
                }
                self.open_sections_manager_at(to);
            }
        }
    }

    /// Applies a submitted section prompt (new or rename) and re-opens the
    /// manager.
    pub(super) fn submit_section_prompt(
        &mut self,
        kind: SectionPromptKind,
        value: String,
    ) {
        let repo_kind = self.tab.repo_kind();
        let result = match &kind {
            SectionPromptKind::New => {
                self.service.add_section(repo_kind, &value)
            }
            SectionPromptKind::Rename(old) => self
                .service
                .rename_section(repo_kind, old, &value)
                .map(|()| self.rename_collapsed(old, value.trim())),
        };
        if let Err(error) = result {
            self.set_status(format!("{error}"));
        }
        self.open_sections_manager();
    }

    /// Deletes the confirmed section `name` (its entries become Ungrouped)
    /// and re-opens the manager.
    pub(super) fn delete_section(&mut self, name: &str) {
        let repo_kind = self.tab.repo_kind();
        match self.service.delete_section(repo_kind, name) {
            Ok(()) => self.forget_collapsed(name),
            Err(error) => self.set_status(format!("{error}")),
        }
        self.open_sections_manager();
    }

    /// Carries a renamed section's fold over to its new name.
    fn rename_collapsed(&mut self, old: &str, new: &str) {
        if self.view_mut().collapsed.remove(old) {
            self.view_mut().collapsed.insert(new.to_string());
            self.save_ui_state();
        }
    }

    /// Drops a deleted section's fold, so a later section of the same name
    /// starts expanded.
    fn forget_collapsed(&mut self, name: &str) {
        if self.view_mut().collapsed.remove(name) {
            self.save_ui_state();
        }
    }
}
