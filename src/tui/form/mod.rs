//! The add/edit form for an entry: path, name, slug, section, favourite,
//! backup and kind, on the toolkit's non-blocking form core
//! (`ratada::form::FormState`), so hop's event loop never blocks.
//!
//! The keys are the core's: normal mode, where letters are commands, and
//! editing mode; `Enter` on Path opens hop's path picker (`e` types the
//! path), on Section the fuzzy section picker; `Left`/`Right` step the kind
//! and `Space` toggles the favourite and backup checkboxes. `s`/`Ctrl+S`
//! save and keep the form open, a second press within a second closes it,
//! `Ctrl+O` puts the focused field back, and `Esc` closes an unchanged form
//! or asks "Save changes?" inside the form. When the name is still blank it
//! is auto-filled from the path's basename.
//!
//! A **bulk** form (editing several entries at once) shows only Section,
//! Favourite, Backup and Kind. A field whose value differs across the
//! selection renders as *mixed* and is only written when the user changes
//! it, so an untouched field leaves every selected entry as it was.

use crossterm::event::KeyEvent;
use ratada::form::{FormEvent, FormField, FormState, is_yes};
use ratada::theme::Skin;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::domain::repo::{Repo, RepoKind};
use crate::domain::sections::UNGROUPED;
use crate::domain::slug::slugify;

/// Which field a problem belongs to.
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

impl Field {
    /// The key of the field in the form core.
    fn key(self) -> &'static str {
        match self {
            Field::Path => PATH,
            Field::Name => NAME,
            Field::Section => SECTION,
            Field::Slug => SLUG,
            Field::Kind => KIND,
            Field::Fav => FAV,
            Field::Backup => BACKUP,
        }
    }
}

const PATH: &str = "path";
const NAME: &str = "name";
const SLUG: &str = "slug";
const SECTION: &str = "section";
const FAV: &str = "fav";
const BACKUP: &str = "backup";
const KIND: &str = "kind";

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
/// changed it, so the other fields leave every selected entry unchanged.
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
    /// The user saved a bulk edit (only the changed fields).
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

/// Add/edit form state: the form core, and whether it edits several
/// entries at once.
pub struct RepoForm {
    state: FormState,
    bulk: bool,
}

impl RepoForm {
    /// A blank add form, seeded with a `path` and guessed `kind`. The backup
    /// toggle defaults per kind: on for git repos, off for file/folder entries.
    pub fn for_add(path: &str, kind: RepoKind) -> Self {
        let mut repo = Repo::new(path.into());
        repo.kind = kind;
        repo.include_in_backup = kind == RepoKind::Git;
        Self::single("Add entry", &repo)
    }

    /// An edit form seeded from an existing entry's fields.
    pub fn for_edit(repo: &Repo) -> Self {
        Self::single("Edit entry", repo)
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
        let section_field = FormField::picker(
            SECTION,
            "Section",
            section_label(section.as_ref().and_then(Option::as_deref)),
        );
        let fav_field = FormField::toggle(FAV, "Fav", fav.unwrap_or(false));
        let backup_field =
            FormField::toggle(BACKUP, "Backup", backup.unwrap_or(false));
        let fields = vec![
            mixed_if(section_field, section.is_none()),
            mixed_if(fav_field, fav.is_none()),
            mixed_if(backup_field, backup.is_none()),
            kind_field(kind),
        ];
        RepoForm {
            state: FormState::new(format!("Edit {count} entries"), fields),
            bulk: true,
        }
    }

    /// A single-entry form titled `title`, seeded from `repo`.
    fn single(title: &str, repo: &Repo) -> Self {
        let fields = vec![
            FormField::text_picker(PATH, "Path", &repo.path.to_string_lossy()),
            FormField::text(NAME, "Name", repo.name.as_deref().unwrap_or("")),
            FormField::text(SLUG, "Slug", repo.slug.as_deref().unwrap_or("")),
            FormField::picker(
                SECTION,
                "Section",
                section_label(repo.section.as_deref()),
            ),
            FormField::toggle(FAV, "Fav", repo.fav),
            FormField::toggle(BACKUP, "Backup", repo.include_in_backup),
            kind_field(repo.kind),
        ];
        RepoForm {
            state: FormState::new(title, fields),
            bulk: false,
        }
    }

    /// Handles a key, returning a save draft, a picker request, a cancel, or
    /// pending.
    pub fn handle_key(&mut self, key: KeyEvent) -> FormResult {
        let was_on_name = self.state.focus_key() == NAME;
        let event = self.state.handle_key(key);
        if !was_on_name && self.state.focus_key() == NAME {
            self.autofill_name();
        }
        match event {
            FormEvent::Save => {
                if self.bulk {
                    FormResult::SaveBulk(self.bulk_draft())
                } else {
                    FormResult::Save(self.draft())
                }
            }
            FormEvent::Activate(key) if key == PATH => FormResult::PickPath,
            FormEvent::Activate(key) if key == SECTION => {
                FormResult::PickSection
            }
            FormEvent::Close => FormResult::Close,
            FormEvent::Discard => FormResult::Cancel,
            FormEvent::Pending
            | FormEvent::Changed(_)
            | FormEvent::Activate(_)
            | FormEvent::ExternalEditor(_) => FormResult::Pending,
        }
    }

    /// Inserts a bracketed paste into the focused text field, if any. The
    /// non-text fields (Section, Kind, Fav, Backup) take no paste.
    pub fn paste(&mut self, text: &str) {
        self.state.paste(text);
    }

    /// Draws the form as a modal centred in `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect, skin: &Skin) {
        self.state.render(frame, area, skin);
    }

    /// Whether any field differs from its baseline.
    pub fn is_dirty(&self) -> bool {
        self.state.is_dirty()
    }

    /// Records a successful save: the saved values become the baseline and
    /// the hint rows say so.
    pub fn mark_saved(&mut self) {
        self.state.mark_saved();
    }

    /// Whether the form closes now that its save succeeded (it was saved
    /// from the "Save changes?" question).
    pub fn closes_after_save(&self) -> bool {
        self.state.closes_after_save()
    }

    /// Reports a problem with `field`: it shows in red under the field,
    /// which takes the focus; the next key clears it.
    pub fn set_error(&mut self, field: Field, message: impl Into<String>) {
        self.state.set_error(field.key(), message);
    }

    /// The current kind (used to seed the section picker with the right list).
    pub fn kind(&self) -> RepoKind {
        kind_of(self.state.value(KIND))
    }

    /// The currently chosen section (`None` = Ungrouped), for the picker cursor.
    pub fn section(&self) -> Option<String> {
        section_of(self.state.value(SECTION))
    }

    /// Sets the section from the picker (a mixed bulk field is mixed no
    /// more, so the edit applies it) and returns the focus to the field.
    pub fn set_section(&mut self, section: Option<String>) {
        self.state
            .set_value(SECTION, section_label(section.as_deref()));
        self.state.focus_on(SECTION);
    }

    /// The path field's current value (used to seed the path picker).
    pub fn path_value(&self) -> String {
        self.state.live_value(PATH)
    }

    /// Replaces the path field (after picking a path) and auto-fills the name
    /// from its basename when the name is still blank.
    pub fn set_path(&mut self, path: &str) {
        self.state.set_value(PATH, path);
        self.autofill_name();
        self.state.focus_on(PATH);
    }

    /// Fills the name from the path's basename when the name is still blank.
    fn autofill_name(&mut self) {
        if self.bulk || !self.state.value(NAME).trim().is_empty() {
            return;
        }
        if let Some(base) = basename(self.state.value(PATH)) {
            self.state.set_value(NAME, &base);
        }
    }

    /// Builds the single-entry draft from the current field values.
    fn draft(&self) -> RepoDraft {
        let value = |key| self.state.value(key).to_string();
        RepoDraft {
            name: non_empty(value(NAME)),
            path: value(PATH),
            slug: non_empty(slugify(self.state.value(SLUG))),
            section: self.section(),
            kind: self.kind(),
            fav: is_yes(self.state.value(FAV)),
            include_in_backup: is_yes(self.state.value(BACKUP)),
        }
    }

    /// Builds the bulk draft: only the fields the user changed.
    fn bulk_draft(&self) -> BulkDraft {
        let changed = |key| {
            self.state
                .index_of(key)
                .is_some_and(|index| self.state.is_changed(index))
        };
        BulkDraft {
            section: changed(SECTION).then(|| self.section()),
            fav: changed(FAV).then(|| is_yes(self.state.value(FAV))),
            include_in_backup: changed(BACKUP)
                .then(|| is_yes(self.state.value(BACKUP))),
            kind: changed(KIND).then(|| self.kind()),
        }
    }
}

/// `field`, marked mixed when `is_mixed`.
fn mixed_if(field: FormField, is_mixed: bool) -> FormField {
    if is_mixed { field.mixed() } else { field }
}

/// The Kind stepper, on `kind`.
fn kind_field(kind: RepoKind) -> FormField {
    let options = [RepoKind::Git, RepoKind::Path]
        .into_iter()
        .map(|kind| kind_label(kind).to_string())
        .collect();
    FormField::stepper(KIND, "Kind", options, kind_label(kind))
}

/// The display label for a kind.
fn kind_label(kind: RepoKind) -> &'static str {
    match kind {
        RepoKind::Git => "git",
        RepoKind::Path => "file/folder",
    }
}

/// The kind a Kind label stands for.
fn kind_of(label: &str) -> RepoKind {
    if label == kind_label(RepoKind::Path) {
        RepoKind::Path
    } else {
        RepoKind::Git
    }
}

/// The Section field's text for `section` (`None` = Ungrouped).
fn section_label(section: Option<&str>) -> &str {
    section.unwrap_or(UNGROUPED)
}

/// The section a Section field's text stands for: "Ungrouped" is virtual,
/// so it reads as none.
fn section_of(label: &str) -> Option<String> {
    (label != UNGROUPED && !label.is_empty()).then(|| label.to_string())
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

#[cfg(test)]
mod tests;
