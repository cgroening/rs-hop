//! The add/edit form for an entry: path, name, slug, section, favourite,
//! backup and kind.
//!
//! `Tab`/`BackTab` (or `Up`/`Down`) step between the visible fields; the text
//! fields edit inline; `Left`/`Right` cycle the kind; `Space` toggles the
//! favourite and backup checkboxes; `Enter` on the Section field opens the
//! fuzzy section picker; `^O` opens the path picker; `Enter` elsewhere (or
//! `Ctrl+S`) saves and `Esc` cancels. When the name is still blank it is
//! auto-filled from the path's basename.
//!
//! A **bulk** form (editing several entries at once) hides Path/Name/Slug and
//! shows only Section/Favourite/Backup/Kind. A field whose value differs across
//! the selection renders as *mixed* and is only written when the user touches
//! it, so an untouched field leaves every selected entry as it was.

use crossterm::event::{KeyCode, KeyEvent};
use ratada::input::InputField;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::domain::repo::{Repo, RepoKind};
use crate::domain::sections::UNGROUPED;
use crate::domain::slug::slugify;
use crate::theme::Skin;
use crate::tui::presentation::{FieldView, field_spans};
use crate::tui::skin::Colors;
use crate::tui::widgets::centered_rect;

/// Display columns reserved for a field's label, before its value.
const LABEL_WIDTH: usize = 8;
/// The placeholder shown for a bulk field whose value differs across entries.
const MIXED: &str = "(mixed)";

/// What a field line needs to draw itself, bundled to keep the parameter count
/// low.
struct LineCtx<'a> {
    /// The colour roles resolved from the active theme.
    colors: &'a Colors,
    /// The active theme, whose palette colours the caret of a text field.
    skin: &'a Skin,
    /// The display columns a field's value may occupy, after its label.
    value_width: usize,
}

/// One editable field of the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Path,
    Name,
    Section,
    Slug,
    Kind,
    Fav,
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
    /// The user cancelled.
    Cancel,
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
        }
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
        // `is_command`, not a bare CONTROL check: AltGr is Control+Alt and must
        // keep typing into the form's fields.
        let ctrl = ratada::input::is_command(key);
        match key.code {
            KeyCode::Esc => return FormResult::Cancel,
            KeyCode::Enter => {
                if self.focused_field() == Field::Section {
                    return FormResult::PickSection;
                }
                return self.save();
            }
            KeyCode::Char('s') if ctrl => return self.save(),
            KeyCode::Char('o') if ctrl && !self.bulk => {
                return FormResult::PickPath;
            }
            KeyCode::Tab | KeyCode::Down => self.move_focus(1),
            KeyCode::BackTab | KeyCode::Up => {
                self.move_focus(self.fields().len() - 1)
            }
            _ => self.edit_focused(key),
        }
        FormResult::Pending
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

    /// Moves focus by `delta` (wrapping), auto-filling the name on arrival.
    fn move_focus(&mut self, delta: usize) {
        let fields = self.fields();
        self.focus = (self.focus + delta) % fields.len();
        if fields[self.focus] == Field::Name {
            self.autofill_name();
        }
    }

    /// Routes an editing key to the focused field.
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
            // The Section field opens the fuzzy picker on Enter (handled above).
            Field::Section => {}
            Field::Kind => self.edit_kind(key),
            Field::Fav => {
                if key.code == KeyCode::Char(' ') {
                    self.toggle_fav();
                }
            }
            Field::Backup => {
                if key.code == KeyCode::Char(' ') {
                    self.toggle_backup();
                }
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

    /// Renders the form centred in `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect, skin: &Skin) {
        let colors = Colors::from_palette(&skin.palette);
        let fields = self.fields();
        let height = fields.len() as u16 + 4;
        let rect = centered_rect(70, height, area);
        frame.render_widget(Clear, rect);
        let block = ratada::chrome::modal_block(skin, &self.title);
        let inner_width = block.inner(rect).width as usize;
        let ctx = LineCtx {
            colors: &colors,
            skin,
            value_width: inner_width.saturating_sub(LABEL_WIDTH),
        };
        let mut lines: Vec<Line> = fields
            .iter()
            .enumerate()
            .map(|(index, field)| self.field_line(*field, index, &ctx))
            .collect();
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            self.hint(),
            Style::default().fg(colors.dim),
        )));
        frame.render_widget(Paragraph::new(lines).block(block), rect);
    }

    /// The footer hint line for this form's mode.
    fn hint(&self) -> &'static str {
        if self.bulk {
            "Tab field · \u{2190}\u{2192} kind · Space toggle · Enter on \
             Section: pick · ^S save · Esc cancel"
        } else {
            "Tab field · \u{2190}\u{2192} kind · Space toggle · Enter on \
             Section: pick · ^O path · ^S save · Esc cancel"
        }
    }

    /// Builds the rendered line for `field` at focus position `index`.
    fn field_line(
        &self,
        field: Field,
        index: usize,
        ctx: &LineCtx,
    ) -> Line<'static> {
        let colors = ctx.colors;
        match field {
            Field::Path => self.text_line("Path", &self.path, index, ctx),
            Field::Name => self.text_line("Name", &self.name, index, ctx),
            Field::Slug => self.text_line("Slug", &self.slug, index, ctx),
            Field::Section => {
                // No `< >` chrome: the section is chosen via the picker
                // (Enter), not cycled like the kind.
                let label = self.section_label();
                self.value_line("Section", &label, index, colors)
            }
            Field::Kind => {
                self.choice_line("Kind", kind_label(self.kind), index, colors)
            }
            Field::Fav => self.fav_line(index, colors),
            Field::Backup => self.backup_line(index, colors),
        }
    }

    /// A labelled text field line, highlighting it when focused. The value is
    /// drawn by the toolkit, so a long path scrolls under the caret instead of
    /// spilling out of the box.
    fn text_line(
        &self,
        label: &str,
        input: &InputField,
        index: usize,
        ctx: &LineCtx,
    ) -> Line<'static> {
        let mut spans = vec![self.label_span(label, index, ctx.colors)];
        spans.extend(field_spans(FieldView {
            field: input,
            palette: &ctx.skin.palette,
            width: ctx.value_width,
            focused: index == self.focus,
        }));
        self.styled(spans, index, ctx.colors)
    }

    /// A `< value >` selector line (the kind, cycled with Left/Right).
    fn choice_line(
        &self,
        label: &str,
        value: &str,
        index: usize,
        colors: &Colors,
    ) -> Line<'static> {
        let spans = vec![
            self.label_span(label, index, colors),
            Span::styled(
                format!("< {value} >"),
                Style::default().fg(colors.accent),
            ),
        ];
        self.styled(spans, index, colors)
    }

    /// A labelled value line without the `< >` selector chrome (the section,
    /// which is chosen via the fuzzy picker on Enter, not cycled).
    fn value_line(
        &self,
        label: &str,
        value: &str,
        index: usize,
        colors: &Colors,
    ) -> Line<'static> {
        let spans = vec![
            self.label_span(label, index, colors),
            Span::styled(value.to_string(), Style::default().fg(colors.accent)),
        ];
        self.styled(spans, index, colors)
    }

    /// The favourite toggle line.
    fn fav_line(&self, index: usize, colors: &Colors) -> Line<'static> {
        let mark = checkbox(self.fav, self.fav_mixed, self.fav_touched);
        let spans = vec![
            self.label_span("Fav", index, colors),
            Span::raw(mark.to_string()),
        ];
        self.styled(spans, index, colors)
    }

    /// The "include in backup all (`Z`)" toggle line.
    fn backup_line(&self, index: usize, colors: &Colors) -> Line<'static> {
        let mark = checkbox(
            self.include_in_backup,
            self.backup_mixed,
            self.backup_touched,
        );
        let spans = vec![
            self.label_span("Backup", index, colors),
            Span::raw(mark.to_string()),
        ];
        self.styled(spans, index, colors)
    }

    /// The label shown for the Section field: the chosen section, Ungrouped, or
    /// the mixed placeholder in an untouched bulk form.
    fn section_label(&self) -> String {
        if self.bulk && self.section_mixed && !self.section_touched {
            return MIXED.to_string();
        }
        self.section
            .clone()
            .unwrap_or_else(|| UNGROUPED.to_string())
    }

    /// The fixed-width field label, accented when focused.
    fn label_span(
        &self,
        label: &str,
        index: usize,
        colors: &Colors,
    ) -> Span<'static> {
        let style = if index == self.focus {
            Style::default()
                .fg(colors.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(colors.dim)
        };
        Span::styled(format!("{label:<LABEL_WIDTH$}"), style)
    }

    /// Applies the focus background tint to a field line.
    fn styled(
        &self,
        spans: Vec<Span<'static>>,
        index: usize,
        colors: &Colors,
    ) -> Line<'static> {
        let line = Line::from(spans);
        if index == self.focus {
            line.style(Style::default().bg(colors.selection_bg))
        } else {
            line
        }
    }
}

/// The checkbox glyph: mixed (`[-]`) while an untouched bulk field differs, else
/// on (`[x]`) or off (`[ ]`).
fn checkbox(on: bool, mixed: bool, touched: bool) -> &'static str {
    if mixed && !touched {
        "[-]"
    } else if on {
        "[x]"
    } else {
        "[ ]"
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
    use super::*;

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
