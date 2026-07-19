//! Key routing: which use case a key press reaches, and nothing else.
//!
//! Overlay keys are handled first, then the live filter, then the keymap's
//! [`Action`](crate::keymap::Action) dispatch. The two structural keys the
//! keymap deliberately does not own (`Tab`/`BackTab` and `Esc`) are consumed
//! here too.

use super::edit::save_index;
use super::*;

use crossterm::event::{KeyCode, KeyEvent};

use ratada::input::InputField;
use ratada::shortcut_hints;

use crate::domain::filter::TabKind;
use crate::keymap::Action;
use crate::tui::form::FormResult;
use crate::tui::path_picker::PickerResult;
use crate::tui::section_picker::SectionPickResult;
use crate::tui::widgets::{ConfirmResult, PromptResult, SelectResult};

impl App {
    /// Handles a key, returning an outcome when the loop should end.
    ///
    /// The toolkit's hints toggle is consumed first, so it works in every
    /// state; `Ctrl+Q` never reaches here (the `Tui` turns it into
    /// [`TuiEvent::Quit`]).
    pub(super) fn handle_key(&mut self, key: KeyEvent) -> Option<RunOutcome> {
        if shortcut_hints::consume_toggle(key) {
            self.save_ui_state();
            return None;
        }
        match &mut self.overlay {
            Overlay::None => self.handle_list_key(key),
            _ => {
                self.handle_overlay_key(key);
                None
            }
        }
    }

    /// Routes a bracketed paste to the focused text field: the live filter, or
    /// the open overlay's own. Anything else has no caret to paste at, so the
    /// paste is dropped.
    pub(super) fn handle_paste(&mut self, text: &str) {
        if self.filtering {
            self.filter.paste(text);
            // The wider query can drop the entry under the cursor, as after
            // any other filter edit.
            self.cursor = 0;
            return;
        }
        match &mut self.overlay {
            Overlay::Form(form, _) => form.paste(text),
            Overlay::Prompt(prompt, _) => prompt.paste(text),
            Overlay::SectionPrompt(prompt, _) => prompt.paste(text),
            Overlay::SectionPicker(picker, ..) => picker.paste(text),
            Overlay::Picker(picker, _) => picker.paste(text),
            _ => {}
        }
    }

    /// Handles a key for an open overlay, transitioning state as needed.
    pub(super) fn handle_overlay_key(&mut self, key: KeyEvent) {
        let overlay = std::mem::replace(&mut self.overlay, Overlay::None);
        match overlay {
            Overlay::Help => self.handle_help_key(key),
            Overlay::Confirm(modal, targets) => match modal.handle_key(key) {
                ConfirmResult::Yes => self.do_delete(targets),
                ConfirmResult::No => {}
                ConfirmResult::Pending => {
                    self.overlay = Overlay::Confirm(modal, targets);
                }
            },
            Overlay::Prompt(mut prompt, index) => {
                match prompt.handle_key(key) {
                    PromptResult::Submit(value) => {
                        self.do_set_slug(index, value)
                    }
                    PromptResult::Cancel => {}
                    PromptResult::Pending => {
                        self.overlay = Overlay::Prompt(prompt, index);
                    }
                }
            }
            Overlay::Form(mut form, target) => match form.handle_key(key) {
                FormResult::Save(draft) => {
                    self.do_save_form(save_index(&target), draft);
                }
                FormResult::SaveBulk(bulk) => self.do_save_bulk(target, bulk),
                FormResult::PickPath => {
                    self.open_form_path_picker(form, target);
                }
                FormResult::PickSection => {
                    self.open_section_picker(form, target);
                }
                FormResult::Cancel => {}
                FormResult::Pending => {
                    self.overlay = Overlay::Form(form, target);
                }
            },
            Overlay::SectionPicker(mut picker, form, target) => {
                match picker.handle_key(key) {
                    SectionPickResult::Picked(section) => {
                        self.resume_form_with_section(form, target, section);
                    }
                    SectionPickResult::Cancel => {
                        self.overlay = Overlay::Form(form, target);
                    }
                    SectionPickResult::Pending => {
                        self.overlay =
                            Overlay::SectionPicker(picker, form, target);
                    }
                }
            }
            Overlay::Picker(mut picker, intent) => {
                match picker.handle_key(key) {
                    PickerResult::Selected(path) => {
                        self.do_picked(intent, path);
                    }
                    PickerResult::Cancel => {}
                    PickerResult::Pending => {
                        self.overlay = Overlay::Picker(picker, intent);
                    }
                }
            }
            Overlay::Errors(mut modal, indices) => {
                match modal.handle_key(key) {
                    SelectResult::Selected(row) => {
                        if let Some(&index) = indices.get(row) {
                            self.open_error_action(index);
                        }
                    }
                    SelectResult::Cancel => {}
                    SelectResult::Pending => {
                        self.overlay = Overlay::Errors(modal, indices);
                    }
                }
            }
            Overlay::ErrorAction(mut modal, index) => {
                match modal.handle_key(key) {
                    SelectResult::Selected(action) => {
                        self.run_error_action(index, action);
                    }
                    SelectResult::Cancel => {}
                    SelectResult::Pending => {
                        self.overlay = Overlay::ErrorAction(modal, index);
                    }
                }
            }
            Overlay::Sort(mut modal, modes) => match modal.handle_key(key) {
                SelectResult::Selected(row) => {
                    if let Some(&mode) = modes.get(row) {
                        self.apply_sort(mode);
                    }
                }
                SelectResult::Cancel => {}
                SelectResult::Pending => {
                    self.overlay = Overlay::Sort(modal, modes);
                }
            },
            Overlay::SectionJump(mut modal, starts) => {
                match modal.handle_key(key) {
                    SelectResult::Selected(row) => {
                        if let Some(&pos) = starts.get(row) {
                            self.cursor = pos;
                        }
                    }
                    SelectResult::Cancel => {}
                    SelectResult::Pending => {
                        self.overlay = Overlay::SectionJump(modal, starts);
                    }
                }
            }
            Overlay::Sections(mut modal) => {
                let action = modal.handle_key(key);
                self.run_sections_action(modal, action);
            }
            Overlay::SectionPrompt(mut prompt, kind) => {
                match prompt.handle_key(key) {
                    PromptResult::Submit(value) => {
                        self.submit_section_prompt(kind, value);
                    }
                    PromptResult::Cancel => self.open_sections_manager(),
                    PromptResult::Pending => {
                        self.overlay = Overlay::SectionPrompt(prompt, kind);
                    }
                }
            }
            Overlay::SectionDelete(confirm, name) => {
                match confirm.handle_key(key) {
                    ConfirmResult::Yes => {
                        let repo_kind = self.tab.repo_kind();
                        if let Err(error) =
                            self.service.delete_section(repo_kind, &name)
                        {
                            self.set_status(format!("{error}"));
                        }
                        self.open_sections_manager();
                    }
                    ConfirmResult::No => self.open_sections_manager(),
                    ConfirmResult::Pending => {
                        self.overlay = Overlay::SectionDelete(confirm, name);
                    }
                }
            }
            Overlay::None => {}
        }
    }

    /// Routes a key inside the help overlay.
    ///
    /// `?` closes it. `Esc` clears a typed filter first and only closes once
    /// the filter is empty, so narrowing down and backing out are one key.
    /// A plain character types into the fuzzy filter; everything else scrolls.
    fn handle_help_key(&mut self, key: KeyEvent) {
        self.overlay = Overlay::Help;
        match key.code {
            KeyCode::Char('?') => self.close_help(),
            KeyCode::Esc if self.help_query.value().is_empty() => {
                self.close_help();
            }
            KeyCode::Esc => self.help_query = InputField::new(""),
            KeyCode::Backspace => {
                self.help_query.handle_key(key);
                self.help_scroll.reset();
            }
            _ if ratada::input::is_bare_character(key) => {
                self.help_query.handle_key(key);
                self.help_scroll.reset();
            }
            _ => self.help_scroll.handle_key(key),
        }
    }

    /// Closes the help overlay and forgets its filter.
    fn close_help(&mut self) {
        self.overlay = Overlay::None;
        self.help_query = InputField::new("");
        self.help_scroll.reset();
    }

    /// Handles a key for the list view (no overlay open).
    ///
    /// Keys the keymap cannot express are consumed first (see
    /// [`App::handle_untracked_key`]); everything else resolves to an
    /// [`Action`], so a `[keys]` override actually rebinds it.
    pub(super) fn handle_list_key(
        &mut self,
        key: KeyEvent,
    ) -> Option<RunOutcome> {
        if self.filtering {
            return self.handle_filter_key(key);
        }
        if self.handle_untracked_key(key) {
            return None;
        }
        let action = self.keymap.action_for(&key)?;
        self.run_action(action)
    }

    /// Handles the keys that have no [`Action`], returning whether one matched.
    ///
    /// Tab cycling and `Esc` are structural rather than user-facing actions, so
    /// they stay out of the keymap and are not rebindable. `Shift`+arrow used
    /// to be caught here too, because the old `KeyChord` ignored the shift
    /// modifier and would have turned a shifted arrow into a plain cursor move;
    /// `ratada::keymap` compares shift for a non-character key, so extending is
    /// now the ordinary `extend_up`/`extend_down` binding.
    pub(super) fn handle_untracked_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Tab => self.cycle_tab(1),
            KeyCode::BackTab => self.cycle_tab(-1),
            KeyCode::Esc => self.clear_selection(),
            _ => return false,
        }
        true
    }

    /// Runs `action` on the list view, returning an outcome when the loop should
    /// end.
    ///
    /// The context-dependent actions branch on the active tab here rather than
    /// on the key, so a rebound key keeps its meaning: `SectionJump` only jumps
    /// where the list is sectioned, and `Reload`/`ReloadFetch` re-check paths on
    /// the files tabs but refresh git status on the git tabs.
    pub(super) fn run_action(&mut self, action: Action) -> Option<RunOutcome> {
        match action {
            Action::Up => self.move_cursor(-1),
            Action::Down => self.move_cursor(1),
            Action::Top => self.cursor_to_edge(false),
            Action::Bottom => self.cursor_to_edge(true),
            Action::PageUp => self.page(-1, false),
            Action::PageDown => self.page(1, false),
            Action::HalfPageUp => self.page(-1, true),
            Action::HalfPageDown => self.page(1, true),
            Action::TabGit => self.select_kind(TabKind::Git),
            Action::TabFiles => self.select_kind(TabKind::Files),
            Action::ToggleSelect => self.toggle_select(),
            Action::ExtendUp => self.extend_selection(-1),
            Action::ExtendDown => self.extend_selection(1),
            Action::ExtendPageUp => self.extend_selection_page(-1),
            Action::ExtendPageDown => self.extend_selection_page(1),
            Action::Jump | Action::JumpCd => return self.open_selected(false),
            Action::Open => return self.open_selected(true),
            Action::GitTool => return self.open_git_inline(),
            Action::OpenApp => return self.force_open_with(),
            Action::Filter => self.filtering = true,
            Action::ChangesFilter => self.toggle_changes_only(),
            Action::Github => self.open_on_github(),
            Action::Preview => self.toggle_preview(),
            Action::PreviewPosition => self.flip_preview_position(),
            Action::PreviewScrollUp => self.scroll_preview(-1),
            Action::PreviewScrollDown => self.scroll_preview(1),
            Action::PreviewPageUp => self.page_preview(-1),
            Action::PreviewPageDown => self.page_preview(1),
            Action::PreviewShrink => self.resize_preview(-1),
            Action::PreviewGrow => self.resize_preview(1),
            Action::Columns => self.cycle_columns(),
            Action::Sort => self.open_sort_picker(),
            Action::ToggleGrouping => self.toggle_grouping(),
            Action::ToggleFavFloat => self.toggle_fav_float(),
            Action::SectionJump if self.is_sectioned() => {
                self.open_section_jump();
            }
            Action::SectionJump => {}
            Action::ManageSections => self.open_sections_manager(),
            Action::ReorderUp => self.move_entry(-1),
            Action::ReorderDown => self.move_entry(1),
            Action::Add => self.open_add(),
            Action::Edit => self.open_edit_form(),
            Action::Delete => self.open_delete_confirm(),
            Action::Undo => self.undo(),
            Action::ToggleFav => self.toggle_fav(),
            Action::Zip => self.zip_targets(),
            Action::ZipAll => self.zip_all(),
            Action::Archive => self.toggle_archive(),
            Action::Slug => self.open_slug_prompt(),
            Action::ToggleSlugs => self.toggle_slugs(),
            Action::CopyPath => self.copy_path(),
            Action::RepairPath => self.open_repair_picker(),
            Action::Errors => self.open_error_list(),
            Action::Reload | Action::ReloadFetch
                if self.tab.kind() == TabKind::Files =>
            {
                self.check_files_existence();
            }
            Action::Reload => self.reload_status(false),
            Action::ReloadFetch => self.reload_status(true),
            Action::RefreshOne => self.refresh_targets(false),
            Action::RefreshOneFetch => self.refresh_targets(true),
            Action::Help => {
                self.help_scroll.reset();
                self.help_query = InputField::new("");
                self.overlay = Overlay::Help;
            }
            Action::Quit => return Some(RunOutcome::Quit),
        }
        None
    }

    /// Handles a key while the live filter is active.
    pub(super) fn handle_filter_key(
        &mut self,
        key: KeyEvent,
    ) -> Option<RunOutcome> {
        match key.code {
            KeyCode::Esc => {
                self.filtering = false;
                self.filter = InputField::new("");
                self.cursor = 0;
            }
            KeyCode::Up => self.move_cursor(-1),
            KeyCode::Down => self.move_cursor(1),
            KeyCode::Enter => return self.open_selected(false),
            _ => {
                if self.filter.handle_key(key) {
                    self.cursor = 0;
                }
            }
        }
        None
    }
}
