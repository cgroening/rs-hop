//! The add/edit form for an entry: path, name, slug, section, favourite,
//! backup and kind.
//!
//! The state and key handling live here; the drawing is a sibling
//! `impl RepoForm` block in [`render`].
//!
//! The form opens in normal mode, where letters are commands. `Tab`/`BackTab`
//! (or `Up`/`Down`, `j`/`k`) step between the visible fields; `Enter` (or `e`)
//! edits the focused Name or Slug, `Enter` on Path opens the path picker (`e`
//! types the path), on Section the fuzzy section picker; `Left`/`Right` cycle
//! the kind and `Space` toggles the favourite and backup checkboxes. While a
//! field is edited, letters type; `Enter` or `Esc` end the editing. `s` and
//! `Ctrl+S` save and keep the form open, a second press within a second
//! closes it; `Ctrl+O` puts the focused field back to its value when the form
//! opened or was last saved; `Esc` closes an unchanged form and otherwise
//! asks "Save changes?" (`y` save, `n` discard, `b`/`Esc` back) in the form
//! itself, so hop's event loop never blocks. When the name is still blank it
//! is auto-filled from the path's basename.
//!
//! A **bulk** form (editing several entries at once) hides Path/Name/Slug and
//! shows only Section/Favourite/Backup/Kind. A field whose value differs across
//! the selection renders as *mixed* and is only written when the user touches
//! it, so an untouched field leaves every selected entry as it was.

mod render;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent};
use ratada::double_press::DoublePress;
use ratada::input::InputField;

use crate::domain::repo::{Repo, RepoKind};
use crate::domain::slug::slugify;
/// The placeholder shown for a bulk field whose value differs across entries.
const MIXED: &str = "(mixed)";

/// Which field the form cursor is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// The entry's path (typed, or picked with the path picker).
    Path,
    /// The display name (falls back to the path's basename).
    Name,
    /// The section (picked with the section picker).
    Section,
    /// The slug for `hop <slug>`.
    Slug,
    /// Git repo or file/folder.
    Kind,
    /// The favourite flag.
    Fav,
    /// Whether the backup-all run (`Z`) includes the entry.
    Backup,
}

/// The values captured by a single-entry form on save.
pub struct RepoDraft {
    /// Explicit name, or `None` to fall back to the path's basename.
    pub name: Option<String>,
    /// The entry path as typed.
    pub path: String,
    /// Slug, or `None` when blank.
    pub slug: Option<String>,
    /// Section name, or `None` for Ungrouped.
    pub section: Option<String>,
    /// The entry kind.
    pub kind: RepoKind,
    /// Whether the entry is a favourite.
    pub fav: bool,
    /// Whether the "backup all" (`Z`) run includes this entry.
    pub include_in_backup: bool,
}

/// The changes a bulk form applies. Each field is `Some` only when the user
/// touched it, so untouched fields leave every selected entry unchanged.
pub struct BulkDraft {
    /// The section to set (`Some(None)` = Ungrouped), or `None` to leave as-is.
    pub section: Option<Option<String>>,
    /// The favourite state to set, or `None` to leave as-is.
    pub fav: Option<bool>,
    /// The backup membership to set, or `None` to leave as-is.
    pub include_in_backup: Option<bool>,
    /// The kind to set, or `None` to leave as-is.
    pub kind: Option<RepoKind>,
}

/// Outcome of feeding a key to the form.
pub enum FormResult {
    /// Still editing.
    Pending,
    /// The user saved a single entry's values.
    Save(RepoDraft),
    /// The user saved a bulk edit (only the touched fields).
    SaveBulk(BulkDraft),
    /// The user asked to pick the path with the filesystem picker.
    PickPath,
    /// The user asked to choose the section with the fuzzy picker.
    PickSection,
    /// The user left without saving anything.
    Cancel,
    /// The user closed the form after saving.
    Close,
}

/// Whether letters are commands or type into the focused field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Editing,
}

/// The values a form compares against for its change markers and `Ctrl+O`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Values {
    name: String,
    path: String,
    slug: String,
    section: Option<String>,
    kind: RepoKind,
    fav: bool,
    include_in_backup: bool,
}

/// Add/edit form state.
pub struct RepoForm {
    title: String,
    /// Whether this form edits several entries at once (hides Path/Name/Slug and
    /// tracks per-field touched/mixed state).
    bulk: bool,
    name: InputField,
    path: InputField,
    slug: InputField,
    kind: RepoKind,
    kind_touched: bool,
    fav: bool,
    fav_mixed: bool,
    fav_touched: bool,
    include_in_backup: bool,
    backup_mixed: bool,
    backup_touched: bool,
    /// The chosen section, or `None` for Ungrouped.
    section: Option<String>,
    /// In a bulk form, whether the section differs across entries (until picked).
    section_mixed: bool,
    section_touched: bool,
    /// The focused field's position in the currently visible fields.
    focus: usize,
    mode: Mode,
    /// The values when the form opened or was last saved.
    baseline: Values,
    /// A problem with a field the last save reported, shown under it.
    error: Option<(Field, String)>,
    /// A short confirmation shown in the hint rows until the next key.
    notice: Option<String>,
    /// Whether `Esc` on a changed form is asking "Save changes?".
    asking_save: bool,
    /// Whether the form closes once the pending save succeeds.
    close_after_save: bool,
    save_press: DoublePress,
    has_saved: bool,
}

impl RepoForm {
    /// A blank add form, seeded with a `path` and guessed `kind`. The backup
    /// toggle defaults per kind: on for git repos, off for file/folder entries.
    pub fn for_add(path: &str, kind: RepoKind) -> Self {
        Self::single(
            "Add entry",
            "",
            path,
            "",
            kind,
            false,
            kind == RepoKind::Git,
            None,
        )
    }

    /// An edit form seeded from an existing entry's fields.
    pub fn for_edit(repo: &Repo) -> Self {
        Self::single(
            "Edit entry",
            repo.name.as_deref().unwrap_or(""),
            &repo.path.to_string_lossy(),
            repo.slug.as_deref().unwrap_or(""),
            repo.kind,
            repo.fav,
            repo.include_in_backup,
            repo.section.clone(),
        )
    }

    /// A bulk form over `count` entries of the same `kind`. Each of `section`,
    /// `fav` and `backup` is the shared value when all entries agree, or `None`
    /// (mixed) when they differ.
    pub fn for_bulk(
        count: usize,
        kind: RepoKind,
        section: Option<Option<String>>,
        fav: Option<bool>,
        backup: Option<bool>,
    ) -> Self {
        let (section, section_mixed) =
            section.map_or((None, true), |value| (value, false));
        let (fav, fav_mixed) =
            fav.map_or((false, true), |value| (value, false));
        let (include_in_backup, backup_mixed) =
            backup.map_or((false, true), |value| (value, false));
        RepoForm {
            title: format!("Edit {count} entries"),
            bulk: true,
            name: InputField::new(""),
            path: InputField::new(""),
            slug: InputField::new(""),
            kind,
            kind_touched: false,
            fav,
            fav_mixed,
            fav_touched: false,
            include_in_backup,
            backup_mixed,
            backup_touched: false,
            section,
            section_mixed,
            section_touched: false,
            focus: 0,
            mode: Mode::Normal,
            baseline: Values::default_for(kind),
            error: None,
            notice: None,
            asking_save: false,
            close_after_save: false,
            save_press: DoublePress::new(),
            has_saved: false,
        }
        .with_baseline()
    }

    /// Builds a single-entry form from explicit seed values.
    #[allow(clippy::too_many_arguments)]
    fn single(
        title: &str,
        name: &str,
        path: &str,
        slug: &str,
        kind: RepoKind,
        fav: bool,
        include_in_backup: bool,
        section: Option<String>,
    ) -> Self {
        RepoForm {
            title: title.to_string(),
            bulk: false,
            name: InputField::new(name),
            path: InputField::new(path),
            slug: InputField::new(slug),
            kind,
            kind_touched: false,
            fav,
            fav_mixed: false,
            fav_touched: false,
            include_in_backup,
            backup_mixed: false,
            backup_touched: false,
            section,
            section_mixed: false,
            section_touched: false,
            focus: 0,
            mode: Mode::Normal,
            baseline: Values::default_for(kind),
            error: None,
            notice: None,
            asking_save: false,
            close_after_save: false,
            save_press: DoublePress::new(),
            has_saved: false,
        }
        .with_baseline()
    }

    /// Takes the current values as the baseline.
    fn with_baseline(mut self) -> Self {
        self.baseline = self.values();
        self
    }

    /// The current values, for the change markers and `Ctrl+O`.
    fn values(&self) -> Values {
        Values {
            name: self.name.value().to_string(),
            path: self.path.value().to_string(),
            slug: self.slug.value().to_string(),
            section: self.section.clone(),
            kind: self.kind,
            fav: self.fav,
            include_in_backup: self.include_in_backup,
        }
    }

    /// Whether `field` differs from its baseline.
    fn is_changed(&self, field: Field) -> bool {
        let now = self.values();
        let base = &self.baseline;
        match field {
            Field::Path => now.path != base.path,
            Field::Name => now.name != base.name,
            Field::Slug => now.slug != base.slug,
            Field::Section => now.section != base.section,
            Field::Kind => now.kind != base.kind,
            Field::Fav => now.fav != base.fav,
            Field::Backup => now.include_in_backup != base.include_in_backup,
        }
    }

    /// Whether any visible field differs from its baseline.
    pub fn is_dirty(&self) -> bool {
        self.fields()
            .into_iter()
            .any(|field| self.is_changed(field))
    }

    /// Records a successful save: the saved values become the baseline and
    /// the hint rows say so.
    pub fn mark_saved(&mut self) {
        self.baseline = self.values();
        self.has_saved = true;
        self.notice = Some("Saved".to_string());
    }

    /// Whether the form closes now that its save succeeded (it was saved
    /// from the "Save changes?" question).
    pub fn closes_after_save(&self) -> bool {
        self.close_after_save
    }

    /// Reports a problem with `field`: it shows in red under the field,
    /// which takes the focus; the next key clears it.
    pub fn set_error(&mut self, field: Field, message: impl Into<String>) {
        self.close_after_save = false;
        self.focus = self.field_index(field);
        self.error = Some((field, message.into()));
    }

    /// `Ctrl+O`: puts the focused field back to its baseline.
    fn reset_focused(&mut self) {
        let base = self.baseline.clone();
        match self.focused_field() {
            Field::Path => self.path = InputField::new(&base.path),
            Field::Name => self.name = InputField::new(&base.name),
            Field::Slug => self.slug = InputField::new(&base.slug),
            Field::Section => self.section = base.section,
            Field::Kind => self.kind = base.kind,
            Field::Fav => self.fav = base.fav,
            Field::Backup => self.include_in_backup = base.include_in_backup,
        }
    }

    /// The current kind (used to seed the section picker with the right list).
    pub fn kind(&self) -> RepoKind {
        self.kind
    }

    /// The currently chosen section (`None` = Ungrouped), for the picker cursor.
    pub fn section(&self) -> Option<String> {
        self.section.clone()
    }

    /// Sets the section from the picker, marking it touched (so a bulk edit
    /// applies it) and returning focus to the Section field.
    pub fn set_section(&mut self, section: Option<String>) {
        self.section = section;
        self.section_mixed = false;
        self.section_touched = true;
        self.focus = self.field_index(Field::Section);
    }

    /// The fields visible in this form, in focus order. A bulk form hides the
    /// per-entry Path/Name/Slug and only offers the shared fields.
    fn fields(&self) -> Vec<Field> {
        if self.bulk {
            return vec![
                Field::Section,
                Field::Fav,
                Field::Backup,
                Field::Kind,
            ];
        }
        vec![
            Field::Path,
            Field::Name,
            Field::Slug,
            Field::Section,
            Field::Fav,
            Field::Backup,
            Field::Kind,
        ]
    }

    /// Handles a key, returning a save draft, a picker request, a cancel, or
    /// pending.
    pub fn handle_key(&mut self, key: KeyEvent) -> FormResult {
        // A problem or a notice reflects the last key; any key clears it.
        self.error = None;
        self.notice = None;
        if self.asking_save {
            return self.answer_save_question(key);
        }
        // `is_command`, not a bare CONTROL check: AltGr is Control+Alt and must
        // keep typing into the form's fields.
        let ctrl = ratada::input::is_command(key);
        if ctrl && key.code == KeyCode::Char('s') {
            return self.press_save();
        }
        if ratada::form::field::is_reset_key(key) {
            self.reset_focused();
            return FormResult::Pending;
        }
        match key.code {
            KeyCode::Tab => self.move_focus(1),
            KeyCode::BackTab => self.move_focus(self.fields().len() - 1),
            _ if self.mode == Mode::Editing => self.edit_key(key),
            _ => return self.normal_key(key),
        }
        FormResult::Pending
    }

    /// A key while a text field is typed into: `Enter`/`Esc` end the editing.
    fn edit_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Esc => self.mode = Mode::Normal,
            _ => self.edit_focused(key),
        }
    }

    /// A key in normal mode, where letters are commands.
    fn normal_key(&mut self, key: KeyEvent) -> FormResult {
        let bare = ratada::input::is_bare_character(key);
        let field = self.focused_field();
        match key.code {
            KeyCode::Esc => return self.leave(),
            KeyCode::Down => self.move_focus(1),
            KeyCode::Up => self.move_focus(self.fields().len() - 1),
            KeyCode::Char('j') if bare => self.move_focus(1),
            KeyCode::Char('k') if bare => {
                self.move_focus(self.fields().len() - 1);
            }
            KeyCode::Char('s') if bare => return self.press_save(),
            KeyCode::Enter if field == Field::Path => {
                return FormResult::PickPath;
            }
            KeyCode::Enter if field == Field::Section => {
                return FormResult::PickSection;
            }
            KeyCode::Enter | KeyCode::Char('e')
                if is_typed(field) && (bare || key.code == KeyCode::Enter) =>
            {
                self.mode = Mode::Editing;
            }
            _ => self.change_focused(key),
        }
        FormResult::Pending
    }

    /// The keys that change a kind or a checkbox in place.
    fn change_focused(&mut self, key: KeyEvent) {
        match self.focused_field() {
            Field::Kind => self.edit_kind(key),
            Field::Fav if toggles(key) => self.toggle_fav(),
            Field::Backup if toggles(key) => self.toggle_backup(),
            _ => {}
        }
    }

    /// `s`/`Ctrl+S`: asks the app to save, keeping the form open; a second
    /// press within a second of a save that left nothing changed closes it.
    fn press_save(&mut self) -> FormResult {
        let doubled = self.save_press.register(Instant::now());
        if doubled && self.has_saved && !self.is_dirty() {
            return FormResult::Close;
        }
        self.mode = Mode::Normal;
        self.save()
    }

    /// `Esc` in normal mode: closes an unchanged form, asks for a changed one.
    fn leave(&mut self) -> FormResult {
        if self.is_dirty() {
            self.asking_save = true;
            return FormResult::Pending;
        }
        if self.has_saved {
            FormResult::Close
        } else {
            FormResult::Cancel
        }
    }

    /// The answer to "Save changes?": `y` saves and closes, `n` discards,
    /// `b`/`Esc` go back to the form.
    fn answer_save_question(&mut self, key: KeyEvent) -> FormResult {
        match key.code {
            KeyCode::Char('y' | 'Y') => {
                self.asking_save = false;
                self.close_after_save = true;
                self.save()
            }
            KeyCode::Char('n' | 'N') => FormResult::Cancel,
            KeyCode::Char('b' | 'B') | KeyCode::Esc => {
                self.asking_save = false;
                FormResult::Pending
            }
            _ => FormResult::Pending,
        }
    }

    /// Inserts a bracketed paste into the focused text field, if any. The
    /// non-text fields (Section, Kind, Fav, Backup) hold no caret to paste at.
    pub fn paste(&mut self, text: &str) {
        match self.focused_field() {
            Field::Path => self.path.paste(text),
            Field::Name => self.name.paste(text),
            Field::Slug => self.slug.paste(text),
            Field::Section | Field::Kind | Field::Fav | Field::Backup => {}
        }
    }

    /// The save result for this form's mode.
    fn save(&self) -> FormResult {
        if self.bulk {
            FormResult::SaveBulk(self.bulk_draft())
        } else {
            FormResult::Save(self.draft())
        }
    }

    /// Moves focus by `delta` (wrapping) and ends any editing, auto-filling
    /// the name on arrival.
    fn move_focus(&mut self, delta: usize) {
        self.mode = Mode::Normal;
        let fields = self.fields();
        self.focus =
            ratada::nav::cycle(self.focus, fields.len(), delta as isize);
        if fields[self.focus] == Field::Name {
            self.autofill_name();
        }
    }

    /// Routes an editing key to the focused text field.
    fn edit_focused(&mut self, key: KeyEvent) {
        match self.focused_field() {
            Field::Path => {
                self.path.handle_key(key);
            }
            Field::Name => {
                self.name.handle_key(key);
            }
            Field::Slug => {
                self.slug.handle_key(key);
            }
            Field::Section | Field::Kind | Field::Fav | Field::Backup => {
                self.mode = Mode::Normal;
            }
        }
    }

    /// Toggles the favourite checkbox; a mixed field flips to on first.
    fn toggle_fav(&mut self) {
        self.fav = if self.fav_mixed { true } else { !self.fav };
        self.fav_mixed = false;
        self.fav_touched = true;
    }

    /// Toggles the backup checkbox; a mixed field flips to on first.
    fn toggle_backup(&mut self) {
        self.include_in_backup = if self.backup_mixed {
            true
        } else {
            !self.include_in_backup
        };
        self.backup_mixed = false;
        self.backup_touched = true;
    }

    /// Cycles the kind on `Left`/`Right`, marking it touched and keeping focus on
    /// the Kind field even though the visible field set can change with the kind.
    fn edit_kind(&mut self, key: KeyEvent) {
        let changed = match key.code {
            KeyCode::Left => {
                self.kind = prev_kind(self.kind);
                true
            }
            KeyCode::Right => {
                self.kind = next_kind(self.kind);
                true
            }
            _ => false,
        };
        if changed {
            self.kind_touched = true;
            self.focus = self.field_index(Field::Kind);
        }
    }

    /// The field currently focused.
    fn focused_field(&self) -> Field {
        self.fields()[self.focus]
    }

    /// The position of `field` in the current layout, or 0 when absent.
    fn field_index(&self, field: Field) -> usize {
        self.fields()
            .iter()
            .position(|candidate| *candidate == field)
            .unwrap_or(0)
    }

    /// The path field's current value (used to seed the path picker).
    pub fn path_value(&self) -> String {
        self.path.value().to_string()
    }

    /// Replaces the path field (after picking a path) and auto-fills the name
    /// from its basename when the name is still blank.
    pub fn set_path(&mut self, path: &str) {
        self.path = InputField::new(path);
        self.autofill_name();
        self.focus = 0;
    }

    /// Fills the name from the path's basename when the name is still blank.
    fn autofill_name(&mut self) {
        if !self.name.value().trim().is_empty() {
            return;
        }
        if let Some(base) = basename(self.path.value()) {
            self.name = InputField::new(&base);
        }
    }

    /// Builds the single-entry draft from the current field values.
    fn draft(&self) -> RepoDraft {
        RepoDraft {
            name: non_empty(self.name.value().to_string()),
            path: self.path.value().to_string(),
            slug: non_empty(slugify(self.slug.value())),
            section: self.section.clone(),
            kind: self.kind,
            fav: self.fav,
            include_in_backup: self.include_in_backup,
        }
    }

    /// Builds the bulk draft: only the fields the user actually touched.
    fn bulk_draft(&self) -> BulkDraft {
        BulkDraft {
            section: self.section_touched.then(|| self.section.clone()),
            fav: self.fav_touched.then_some(self.fav),
            include_in_backup: self
                .backup_touched
                .then_some(self.include_in_backup),
            kind: self.kind_touched.then_some(self.kind),
        }
    }
}

impl Values {
    /// The values of an empty form of `kind` (replaced by the seeded ones).
    fn default_for(kind: RepoKind) -> Self {
        Values {
            name: String::new(),
            path: String::new(),
            slug: String::new(),
            section: None,
            kind,
            fav: false,
            include_in_backup: false,
        }
    }
}

/// Whether `field` is typed into (rather than picked or toggled).
fn is_typed(field: Field) -> bool {
    matches!(field, Field::Path | Field::Name | Field::Slug)
}

/// Whether `key` toggles a checkbox: a bare `Space` or `Enter`.
fn toggles(key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char(' ') => ratada::input::is_bare_character(key),
        KeyCode::Enter => true,
        _ => false,
    }
}

/// The final path component of `path`, or `None` when it has none.
fn basename(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    std::path::Path::new(trimmed)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

/// Returns `Some(value)` when non-empty after trimming, else `None`.
fn non_empty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

/// The next kind in the cycle (git <-> file/folder).
fn next_kind(kind: RepoKind) -> RepoKind {
    toggle_kind(kind)
}

/// The previous kind in the cycle (git <-> file/folder).
fn prev_kind(kind: RepoKind) -> RepoKind {
    toggle_kind(kind)
}

/// Toggles between the two kinds.
fn toggle_kind(kind: RepoKind) -> RepoKind {
    match kind {
        RepoKind::Git => RepoKind::Path,
        RepoKind::Path => RepoKind::Git,
    }
}

/// The display label for a kind.
fn kind_label(kind: RepoKind) -> &'static str {
    match kind {
        RepoKind::Git => "git",
        RepoKind::Path => "file/folder",
    }
}

#[cfg(test)]
mod tests {
    use super::render::checkbox;
    use super::*;
    use crate::domain::sections::UNGROUPED;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn edit_form() -> RepoForm {
        RepoForm::single(
            "Edit entry",
            "hop",
            "/code/hop",
            "",
            RepoKind::Git,
            false,
            true,
            None,
        )
    }

    #[test]
    fn letters_are_commands_until_e_starts_typing() {
        let mut form = edit_form();
        form.focus = form.field_index(Field::Name);
        form.handle_key(key(KeyCode::Char('x')));
        assert_eq!(form.name.value(), "hop");
        form.handle_key(key(KeyCode::Char('e')));
        form.handle_key(key(KeyCode::Char('x')));
        assert_eq!(form.name.value(), "hopx");
        form.handle_key(key(KeyCode::Esc));
        assert_eq!(form.mode, Mode::Normal);
    }

    #[test]
    fn enter_on_the_path_opens_the_picker() {
        let mut form = edit_form();
        assert!(matches!(
            form.handle_key(key(KeyCode::Enter)),
            FormResult::PickPath
        ));
    }

    #[test]
    fn a_save_keeps_the_form_and_a_second_press_closes_it() {
        let mut form = edit_form();
        assert!(matches!(
            form.handle_key(key(KeyCode::Char('s'))),
            FormResult::Save(_)
        ));
        form.mark_saved();
        assert!(matches!(
            form.handle_key(key(KeyCode::Char('s'))),
            FormResult::Close
        ));
    }

    #[test]
    fn esc_on_a_changed_form_asks_and_each_answer_does_its_part() {
        let mut form = edit_form();
        form.focus = form.field_index(Field::Fav);
        form.handle_key(key(KeyCode::Char(' ')));
        assert!(form.is_dirty());
        assert!(matches!(
            form.handle_key(key(KeyCode::Esc)),
            FormResult::Pending
        ));
        assert!(form.is_asking());
        form.handle_key(key(KeyCode::Char('b')));
        assert!(!form.is_asking());
        form.handle_key(key(KeyCode::Esc));
        assert!(matches!(
            form.handle_key(key(KeyCode::Char('y'))),
            FormResult::Save(_)
        ));
        assert!(form.closes_after_save());
        let mut form = edit_form();
        form.focus = form.field_index(Field::Fav);
        form.handle_key(key(KeyCode::Char(' ')));
        form.handle_key(key(KeyCode::Esc));
        assert!(matches!(
            form.handle_key(key(KeyCode::Char('n'))),
            FormResult::Cancel
        ));
    }

    #[test]
    fn ctrl_o_puts_the_focused_field_back() {
        let mut form = edit_form();
        form.focus = form.field_index(Field::Kind);
        form.handle_key(key(KeyCode::Right));
        assert!(form.is_changed(Field::Kind));
        form.handle_key(ctrl('o'));
        assert_eq!(form.kind, RepoKind::Git);
        assert!(!form.is_dirty());
    }

    #[test]
    fn an_unchanged_form_closes_on_esc() {
        let mut form = edit_form();
        assert!(matches!(
            form.handle_key(key(KeyCode::Esc)),
            FormResult::Cancel
        ));
    }

    #[test]
    fn a_field_problem_takes_the_focus() {
        let mut form = edit_form();
        form.set_error(Field::Slug, "taken");
        assert_eq!(form.focused_field(), Field::Slug);
    }

    /// Draws `form` on an 80x24 screen; trailing blanks trimmed per row.
    fn drawn(form: &RepoForm) -> String {
        use ratatui::{Terminal, backend::TestBackend};
        let skin = crate::config::Config::default().skin();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| form.render(frame, frame.area(), &skin))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..24)
            .map(|y| {
                let row: String =
                    (0..80).map(|x| buffer[(x, y)].symbol()).collect();
                row.trim_end().to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The whole form: frame, label column, picker fields, stepper, the
    /// change marker and the two hint rows.
    #[test]
    fn the_form_renders_as_a_whole() {
        let mut form = edit_form();
        form.focus = form.field_index(Field::Fav);
        form.handle_key(key(KeyCode::Char(' ')));
        insta::assert_snapshot!(drawn(&form));
    }

    /// A field problem stands in red under its field.
    #[test]
    fn a_field_problem_renders_under_its_field() {
        let mut form = edit_form();
        form.set_error(Field::Path, "path must not be empty");
        let text = drawn(&form);
        let rows: Vec<&str> = text.lines().collect();
        let path = rows.iter().position(|row| row.contains("Path")).unwrap();
        assert!(rows[path + 1].contains("path must not be empty"), "{text}");
    }

    #[test]
    fn basename_takes_final_component() {
        assert_eq!(basename("/code/hop").as_deref(), Some("hop"));
        assert_eq!(basename("/code/hop/").as_deref(), Some("hop"));
        assert_eq!(basename("  ").as_deref(), None);
    }

    #[test]
    fn single_form_shows_all_fields_for_both_kinds() {
        for kind in [RepoKind::Git, RepoKind::Path] {
            let form = RepoForm::for_add("/p", kind);
            for field in [
                Field::Path,
                Field::Name,
                Field::Slug,
                Field::Section,
                Field::Fav,
                Field::Backup,
                Field::Kind,
            ] {
                assert!(form.fields().contains(&field));
            }
        }
    }

    #[test]
    fn field_order_puts_slug_before_section() {
        let form = RepoForm::for_add("/p", RepoKind::Git);
        let fields = form.fields();
        let slug = fields.iter().position(|f| *f == Field::Slug).unwrap();
        let section = fields.iter().position(|f| *f == Field::Section).unwrap();
        assert!(slug < section);
    }

    #[test]
    fn kind_cycle_marks_it_touched_and_keeps_the_section() {
        let mut form = RepoForm::for_edit(&{
            let mut repo = Repo::new("/p".into());
            repo.section = Some("Backend".to_string());
            repo
        });
        assert!(!form.kind_touched);
        form.edit_kind(KeyEvent::from(KeyCode::Right));
        assert_eq!(form.kind, RepoKind::Path);
        assert!(form.kind_touched);
        // The typed section survives a kind change (re-registered on save).
        assert_eq!(form.section.as_deref(), Some("Backend"));
    }

    #[test]
    fn for_add_seeds_backup_toggle_per_kind() {
        let git = RepoForm::for_add("/p", RepoKind::Git);
        assert!(git.draft().include_in_backup);
        let folder = RepoForm::for_add("/p", RepoKind::Path);
        assert!(!folder.draft().include_in_backup);
    }

    #[test]
    fn paste_lands_in_the_focused_text_field() {
        let mut form = RepoForm::for_add("", RepoKind::Git);
        form.focus = form.field_index(Field::Path);
        form.paste("/code/hop");
        assert_eq!(form.path.value(), "/code/hop");

        form.focus = form.field_index(Field::Name);
        form.paste("hop");
        assert_eq!(form.name.value(), "hop");
        // The earlier field keeps its own value.
        assert_eq!(form.path.value(), "/code/hop");
    }

    #[test]
    fn paste_on_a_non_text_field_changes_nothing() {
        let mut form = RepoForm::for_add("/p", RepoKind::Git);
        form.focus = form.field_index(Field::Fav);
        form.paste("/code/hop");
        assert_eq!(form.path.value(), "/p");
        assert_eq!(form.name.value(), "");
        assert_eq!(form.slug.value(), "");
    }

    #[test]
    fn set_section_marks_it_touched() {
        let mut form = RepoForm::for_add("/p", RepoKind::Git);
        assert!(!form.section_touched);
        form.set_section(Some("Work".to_string()));
        assert_eq!(form.section.as_deref(), Some("Work"));
        assert!(form.section_touched);
    }

    #[test]
    fn bulk_form_hides_per_entry_fields() {
        let form =
            RepoForm::for_bulk(3, RepoKind::Git, Some(None), Some(true), None);
        let fields = form.fields();
        assert_eq!(
            fields,
            vec![Field::Section, Field::Fav, Field::Backup, Field::Kind]
        );
        assert!(!fields.contains(&Field::Path));
        assert!(!fields.contains(&Field::Name));
        assert!(!fields.contains(&Field::Slug));
    }

    #[test]
    fn bulk_draft_only_carries_touched_fields() {
        // Section shared, fav shared, backup mixed, kind untouched.
        let mut form = RepoForm::for_bulk(
            2,
            RepoKind::Git,
            Some(Some("A".to_string())),
            Some(false),
            None,
        );
        // Nothing touched yet: an all-None draft.
        let draft = form.bulk_draft();
        assert!(draft.section.is_none());
        assert!(draft.fav.is_none());
        assert!(draft.include_in_backup.is_none());
        assert!(draft.kind.is_none());

        // Touch fav (flips false -> true) and set a section.
        form.toggle_fav();
        form.set_section(Some("B".to_string()));
        let draft = form.bulk_draft();
        assert_eq!(draft.fav, Some(true));
        assert_eq!(draft.section, Some(Some("B".to_string())));
        // Backup was mixed and untouched: still left as-is.
        assert!(draft.include_in_backup.is_none());
        assert!(draft.kind.is_none());
    }

    #[test]
    fn a_mixed_bulk_checkbox_toggles_to_on_first() {
        let mut form = RepoForm::for_bulk(2, RepoKind::Git, None, None, None);
        // A mixed checkbox renders indeterminate until touched.
        assert_eq!(checkbox(form.fav, form.fav_mixed, form.fav_touched), "[-]");
        form.toggle_fav();
        assert!(form.fav);
        assert_eq!(checkbox(form.fav, form.fav_mixed, form.fav_touched), "[x]");
    }

    #[test]
    fn a_mixed_section_shows_the_placeholder_until_picked() {
        let mut form =
            RepoForm::for_bulk(2, RepoKind::Git, None, Some(true), None);
        assert_eq!(form.section_label(), MIXED);
        form.set_section(None);
        assert_eq!(form.section_label(), UNGROUPED);
    }
}
