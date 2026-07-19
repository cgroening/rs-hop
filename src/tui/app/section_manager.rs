//! The section overlays: jumping to a section and managing the current kind's
//! section list.

use super::*;

use crate::domain::sections;
use crate::tui::sections_modal::{SectionsAction, SectionsModal};
use crate::tui::widgets::{ConfirmModal, SelectModal, TextPrompt};

impl App {
    /// Opens the jump-to-section picker for the Files tab.
    pub(super) fn open_section_jump(&mut self) {
        let groups = self.section_groups();
        if groups.len() < 2 {
            self.set_status("no other sections");
            return;
        }
        let starts = sections::section_starts(&groups);
        let labels: Vec<String> =
            groups.iter().map(|g| g.label.clone()).collect();
        let current =
            sections::current_section(&starts, self.cursor).unwrap_or(0);
        self.overlay = Overlay::SectionJump(
            SelectModal::new("Jump to section", labels, current),
            starts,
        );
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
            SectionPromptKind::Rename(old) => {
                self.service.rename_section(repo_kind, old, &value)
            }
        };
        if let Err(error) = result {
            self.set_status(format!("{error}"));
        }
        self.open_sections_manager();
    }
}
