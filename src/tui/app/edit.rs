//! Mutating entries: the modals that ask and the writers that answer.
//!
//! Every mutation goes through [`RepoService`](crate::service::repo_service),
//! so each one is a single undo frame.

use super::*;

use std::path::PathBuf;

use crate::domain::path_repair::nearest_existing_on_disk;
use crate::domain::repo::{Repo, RepoKind};
use crate::tui::form::{BulkDraft, RepoDraft, RepoForm};
use crate::tui::path_picker::PathPicker;
use crate::tui::section_picker::SectionPicker;
use crate::tui::widgets::{ConfirmModal, SelectModal, TextPrompt};

/// The single index a non-bulk `Save` applies to (`One` -> `Some`, else `None`).
pub(super) fn save_index(target: &EditTarget) -> Option<usize> {
    match target {
        EditTarget::One(index) => Some(*index),
        _ => None,
    }
}

/// The shared value of a field over `repos`: `Some(value)` when all agree, or
/// `None` when they differ (a *mixed* field).
pub(super) fn shared<T: PartialEq>(
    repos: &[&Repo],
    get: impl Fn(&Repo) -> T,
) -> Option<T> {
    let mut values = repos.iter().map(|repo| get(repo));
    let first = values.next()?;
    values.all(|value| value == first).then_some(first)
}

/// Copies the draft's fields onto `repo`, keeping its runtime/example fields.
pub(super) fn apply_draft(repo: &mut Repo, draft: RepoDraft, path: PathBuf) {
    repo.name = draft.name;
    repo.path = path;
    repo.slug = draft.slug;
    repo.section = draft.section;
    repo.kind = draft.kind;
    repo.fav = draft.fav;
    repo.include_in_backup = draft.include_in_backup;
}

impl App {
    /// Reverts the last config mutation, keeping the cursor in range.
    pub(super) fn undo(&mut self) {
        match self.service.undo() {
            Ok(Some(label)) => {
                self.clear_selection();
                let len = self.ordered_view().len();
                self.clamp_cursor(len);
                self.set_status(format!("undid: {label}"));
            }
            Ok(None) => self.set_status("nothing to undo"),
            Err(error) => self.set_status(format!("undo failed: {error}")),
        }
    }

    /// Starts adding an entry: the form opens directly with a kind guessed from
    /// the active tab. The path is a plain text field; `^O` opens the picker.
    pub(super) fn open_add(&mut self) {
        let form = RepoForm::for_add("", self.tab.repo_kind());
        self.overlay = Overlay::Form(Box::new(form), EditTarget::Add);
    }

    /// Opens the path picker to fill the path field of `form`, seeded near the
    /// path typed so far.
    pub(super) fn open_form_path_picker(
        &mut self,
        form: Box<RepoForm>,
        target: EditTarget,
    ) {
        let typed = form.path_value();
        let start = if typed.trim().is_empty() {
            crate::util::paths::home_dir().unwrap_or_else(|| PathBuf::from("/"))
        } else {
            crate::util::paths::expand_tilde(&typed)
        };
        self.overlay = Overlay::Picker(
            PathPicker::new(&start, true),
            PickerIntent::FormPath(form, target),
        );
    }

    /// Opens the fuzzy section picker over a form in progress, seeded with the
    /// form's kind's section list and its current section.
    pub(super) fn open_section_picker(
        &mut self,
        form: Box<RepoForm>,
        target: EditTarget,
    ) {
        let sections = self.service.sections(form.kind()).to_vec();
        let picker = SectionPicker::new(&sections, form.section().as_deref());
        self.overlay = Overlay::SectionPicker(Box::new(picker), form, target);
    }

    /// Re-opens the form after the section picker, applying the chosen section.
    pub(super) fn resume_form_with_section(
        &mut self,
        mut form: Box<RepoForm>,
        target: EditTarget,
        section: Option<String>,
    ) {
        form.set_section(section);
        self.overlay = Overlay::Form(form, target);
    }

    /// Opens the edit form: a bulk form when several entries are targeted, else
    /// the single-entry form for the cursor/selection.
    pub(super) fn open_edit_form(&mut self) {
        let targets = self.targets();
        match targets.len() {
            0 => {}
            1 => self.edit_form_for(targets[0]),
            _ => self.open_bulk_form(targets),
        }
    }

    /// Opens the edit form for the entry at `index`.
    pub(super) fn edit_form_for(&mut self, index: usize) {
        let Some(repo) = self.service.get(index) else {
            return;
        };
        let form = RepoForm::for_edit(repo);
        self.overlay = Overlay::Form(Box::new(form), EditTarget::One(index));
    }

    /// Opens a bulk-edit form over `indices` (all the same kind, since selection
    /// is per-tab): the shared value of each field, or *mixed* when they differ.
    pub(super) fn open_bulk_form(&mut self, indices: Vec<usize>) {
        let repos: Vec<&Repo> = indices
            .iter()
            .filter_map(|&index| self.service.get(index))
            .collect();
        if repos.is_empty() {
            return;
        }
        let kind = repos[0].kind;
        let section = shared(&repos, |repo| repo.section.clone());
        let fav = shared(&repos, |repo| repo.fav);
        let backup = shared(&repos, |repo| repo.include_in_backup);
        let form = RepoForm::for_bulk(repos.len(), kind, section, fav, backup);
        self.overlay = Overlay::Form(Box::new(form), EditTarget::Bulk(indices));
    }

    /// Opens the delete confirmation for the target entries (selection/cursor).
    pub(super) fn open_delete_confirm(&mut self) {
        let targets = self.targets();
        if !targets.is_empty() {
            self.confirm_delete(targets);
        }
    }

    /// Opens the delete confirmation for a single entry at `index`.
    pub(super) fn delete_confirm_for(&mut self, index: usize) {
        self.confirm_delete(vec![index]);
    }

    /// Opens a delete confirmation whose message names the count of `targets`.
    pub(super) fn confirm_delete(&mut self, targets: Vec<usize>) {
        let message = if targets.len() == 1 {
            let name = self
                .service
                .get(targets[0])
                .map_or_else(String::new, Repo::display_name);
            format!("Delete \"{name}\" from the list?")
        } else {
            format!("Delete {} entries from the list?", targets.len())
        };
        self.overlay = Overlay::Confirm(
            ConfirmModal::new("Delete entries", message),
            targets,
        );
    }

    /// Opens the slug prompt for the selected entry.
    pub(super) fn open_slug_prompt(&mut self) {
        let Some(index) = self.selected_index() else {
            return;
        };
        let current = self
            .service
            .get(index)
            .and_then(|r| r.slug.clone())
            .unwrap_or_default();
        self.overlay = Overlay::Prompt(
            TextPrompt::new("Set slug", "slug", &current),
            index,
        );
    }

    /// Opens the path picker to repair the selected entry's missing path.
    pub(super) fn open_repair_picker(&mut self) {
        if let Some(index) = self.selected_index() {
            self.repair_picker_for(index);
        }
    }

    /// Opens the repair picker for the entry at `index`, starting at the nearest
    /// existing ancestor of its (missing) path.
    pub(super) fn repair_picker_for(&mut self, index: usize) {
        let Some(repo) = self.service.get(index) else {
            return;
        };
        let start = nearest_existing_on_disk(&repo.path)
            .unwrap_or_else(|| PathBuf::from("/"));
        self.overlay = Overlay::Picker(
            PathPicker::new(&start, false),
            PickerIntent::Repair(index),
        );
    }

    /// Opens the popup listing all entries with a missing or invalid path.
    pub(super) fn open_error_list(&mut self) {
        let repos = self.service.repos();
        let mut indices = Vec::new();
        let mut labels = Vec::new();
        for (index, repo) in repos.iter().enumerate() {
            if let Some(error) = self.path_error(repo) {
                labels.push(format!("{} - {error}", repo.display_name()));
                indices.push(index);
            }
        }
        if indices.is_empty() {
            self.set_status("no errors");
            return;
        }
        self.overlay =
            Overlay::Errors(SelectModal::new("Errors", labels, 0), indices);
    }

    /// Opens the action menu for an errored entry at `index`.
    pub(super) fn open_error_action(&mut self, index: usize) {
        let name = self
            .service
            .get(index)
            .map_or_else(String::new, Repo::display_name);
        let actions = vec![
            "Repair path".to_string(),
            "Edit".to_string(),
            "Delete".to_string(),
        ];
        self.overlay = Overlay::ErrorAction(
            SelectModal::new(format!("Fix \"{name}\""), actions, 0),
            index,
        );
    }

    /// Runs the chosen action menu entry for the errored entry at `index`.
    pub(super) fn run_error_action(&mut self, index: usize, action: usize) {
        match action {
            0 => self.repair_picker_for(index),
            1 => self.edit_form_for(index),
            _ => self.delete_confirm_for(index),
        }
    }

    /// Toggles the favourite flag of the target entries (all on, else all off),
    /// keeping the cursor on the same entry even as favourites re-sort.
    pub(super) fn toggle_fav(&mut self) {
        let targets = self.targets();
        if targets.is_empty() {
            return;
        }
        let focus = self.cursor_path();
        let all_fav = targets
            .iter()
            .all(|&i| self.service.get(i).is_some_and(|r| r.fav));
        if let Err(error) = self.service.set_fav_many(&targets, !all_fav) {
            self.set_status(format!("could not change favourite: {error}"));
        }
        self.clear_selection();
        self.refocus(focus);
    }

    /// Archives or restores the target entries (all archived, else all on) and
    /// keeps the cursor in range.
    pub(super) fn toggle_archive(&mut self) {
        let targets = self.targets();
        if targets.is_empty() {
            return;
        }
        let all_archived = targets
            .iter()
            .all(|&i| self.service.get(i).is_some_and(|r| r.archived));
        if let Err(error) =
            self.service.set_archived_many(&targets, !all_archived)
        {
            self.set_status(format!("could not change archive: {error}"));
        }
        self.clear_selection();
        let len = self.ordered_view().len();
        self.clamp_cursor(len);
    }

    /// Deletes the confirmed target entries.
    pub(super) fn do_delete(&mut self, targets: Vec<usize>) {
        match self.service.delete_many(&targets) {
            Ok(()) => {
                self.clear_selection();
                let len = self.ordered_view().len();
                self.clamp_cursor(len);
                let count = targets.len();
                self.set_status(if count == 1 {
                    "deleted entry".to_string()
                } else {
                    format!("deleted {count} entries")
                });
            }
            Err(error) => self.set_status(format!("delete failed: {error}")),
        }
    }

    /// Sets or clears the slug of the entry at `index`.
    pub(super) fn do_set_slug(&mut self, index: usize, value: String) {
        let slug = if value.trim().is_empty() {
            None
        } else {
            Some(value)
        };
        match self.service.set_slug(index, slug) {
            Ok(()) => self.set_status("slug updated"),
            Err(error) => self.set_status(format!("{error}")),
        }
    }

    /// Saves the add or edit form into a new or existing entry, registering a
    /// newly typed section name.
    pub(super) fn do_save_form(
        &mut self,
        index: Option<usize>,
        draft: RepoDraft,
    ) {
        let path = crate::util::paths::expand_tilde(draft.path.trim());
        if draft.path.trim().is_empty() {
            self.set_status("path must not be empty");
            return;
        }
        let section = draft.section.clone();
        let kind = draft.kind;
        // A folder needs a trailing slash to be recognised before it exists.
        let assumed_file = kind == RepoKind::Path
            && !draft.path.trim().ends_with('/')
            && !path.exists();
        // A git entry refreshes its own status only when its path changed (or
        // when it is newly added); other edits never touch git.
        let path_changed = match index {
            Some(index) => {
                self.service.get(index).map(|r| &r.path) != Some(&path)
            }
            None => true,
        };
        let new_path = path.clone();
        let (result, ok_message) = match index {
            Some(index) => {
                let Some(mut repo) = self.service.get(index).cloned() else {
                    return;
                };
                apply_draft(&mut repo, draft, path);
                (self.service.update(index, repo), "entry updated")
            }
            None => {
                let mut repo = Repo::new(path.clone());
                apply_draft(&mut repo, draft, path);
                (self.service.add(repo), "entry added")
            }
        };
        let saved = result.is_ok();
        if saved && let Some(name) = section {
            let _ = self.service.ensure_section(kind, &name);
        }
        self.report(result, ok_message);
        if saved && assumed_file {
            self.set_status(
                "no trailing / - treated as a file (end with / for a folder)",
            );
        }
        if saved
            && kind == RepoKind::Git
            && path_changed
            && !self.config.example_mode
        {
            self.refresh_paths(vec![new_path], false, false);
        }
    }

    /// Applies a bulk edit: writes each touched field onto every target entry
    /// (one undo frame), registering a newly typed section and refocusing.
    pub(super) fn do_save_bulk(
        &mut self,
        target: EditTarget,
        draft: BulkDraft,
    ) {
        let EditTarget::Bulk(indices) = target else {
            return;
        };
        // The kind after the edit decides which namespace a new section joins.
        let kind_after = draft.kind.unwrap_or_else(|| self.tab.repo_kind());
        let result = self.service.update_many(&indices, |repo| {
            if let Some(section) = &draft.section {
                repo.section = section.clone();
            }
            if let Some(fav) = draft.fav {
                repo.fav = fav;
            }
            if let Some(backup) = draft.include_in_backup {
                repo.include_in_backup = backup;
            }
            if let Some(kind) = draft.kind {
                repo.kind = kind;
            }
        });
        if result.is_ok()
            && let Some(Some(name)) = &draft.section
        {
            let _ = self.service.ensure_section(kind_after, name);
        }
        let count = indices.len();
        self.report(result, &format!("updated {count} entries"));
        self.clear_selection();
        let len = self.ordered_view().len();
        self.clamp_cursor(len);
        self.start_stats();
    }

    /// Applies a picked path to its intent (repair an entry, or fill a form).
    pub(super) fn do_picked(&mut self, intent: PickerIntent, path: PathBuf) {
        match intent {
            PickerIntent::Repair(index) => {
                let repaired = path.clone();
                match self.service.set_path(index, path) {
                    Ok(()) => {
                        self.set_status("path repaired");
                        if !self.config.example_mode {
                            self.clear_repaired_error(repaired);
                        }
                    }
                    Err(error) => {
                        self.set_status(format!("repair failed: {error}"))
                    }
                }
            }
            PickerIntent::FormPath(mut form, target) => {
                form.set_path(&path.to_string_lossy());
                self.overlay = Overlay::Form(form, target);
            }
        }
    }

    /// Reports a service result as a transient status message.
    pub(super) fn report(
        &mut self,
        result: crate::domain::error::Result<()>,
        ok_message: &str,
    ) {
        match result {
            Ok(()) => self.set_status(ok_message),
            Err(error) => self.set_status(format!("{error}")),
        }
    }
}
