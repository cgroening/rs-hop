//! The add/edit form for an entry: path, name, section, slug, kind, favourite.
//!
//! Which fields show depends on the current kind: a git entry shows the
//! Favourite toggle, a folder/file shows the Section dropdown; changing the
//! Kind field updates this live. `Tab`/`BackTab` (or `Up`/`Down`) step between
//! the visible fields, the text fields edit inline, `Left`/`Right` cycle the
//! section and the kind, `Space` toggles the favourite, `^O` opens the path
//! picker, `Enter`/`Ctrl+S` save and `Esc` cancels. When the name is still
//! blank, it is auto-filled from the path's basename (after picking a path, or
//! when the name field is focused).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::domain::repo::{Repo, RepoKind};
use crate::domain::sections::UNGROUPED;
use crate::domain::slug::slugify;
use crate::tui::colors::{ACCENT, DIM, SELECTION_BG};
use crate::tui::text_input::TextInput;
use crate::tui::widgets::centered_rect;

/// One editable field of the form.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Path,
    Name,
    Section,
    Slug,
    Kind,
    Fav,
    Backup,
}

/// The values captured by the form on save.
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

/// Outcome of feeding a key to the form.
pub enum FormResult {
    /// Still editing.
    Pending,
    /// The user saved these values.
    Save(RepoDraft),
    /// The user asked to pick the path with the filesystem picker.
    PickPath,
    /// The user cancelled.
    Cancel,
}

/// Add/edit form state.
pub struct RepoForm {
    title: String,
    name: TextInput,
    path: TextInput,
    slug: TextInput,
    kind: RepoKind,
    fav: bool,
    include_in_backup: bool,
    /// The section options offered by the dropdown: `None` (Ungrouped) first,
    /// then each known section.
    section_options: Vec<Option<String>>,
    /// The selected index into `section_options`.
    section_choice: usize,
    /// The original section, kept for entries whose Section field is hidden.
    seed_section: Option<String>,
    /// The focused field's position in the currently visible fields.
    focus: usize,
}

impl RepoForm {
    /// A blank add form, seeded with a `path` and guessed `kind`. The backup
    /// toggle defaults per kind: on for git repos, off for file/folder entries.
    pub fn for_add(path: &str, kind: RepoKind, sections: &[String]) -> Self {
        Self::build(
            "Add entry",
            "",
            path,
            "",
            kind,
            false,
            kind == RepoKind::Git,
            None,
            sections,
        )
    }

    /// An edit form seeded from an existing entry's fields.
    pub fn for_edit(repo: &Repo, sections: &[String]) -> Self {
        Self::build(
            "Edit entry",
            repo.name.as_deref().unwrap_or(""),
            &repo.path.to_string_lossy(),
            repo.slug.as_deref().unwrap_or(""),
            repo.kind,
            repo.fav,
            repo.include_in_backup,
            repo.section.clone(),
            sections,
        )
    }

    /// Builds a form from explicit seed values and the known section names.
    #[allow(clippy::too_many_arguments)]
    fn build(
        title: &str,
        name: &str,
        path: &str,
        slug: &str,
        kind: RepoKind,
        fav: bool,
        include_in_backup: bool,
        section: Option<String>,
        sections: &[String],
    ) -> Self {
        let section_options = section_options(sections);
        let section_choice =
            section_choice(&section_options, section.as_deref());
        RepoForm {
            title: title.to_string(),
            name: TextInput::new(name),
            path: TextInput::new(path),
            slug: TextInput::new(slug),
            kind,
            fav,
            include_in_backup,
            section_options,
            section_choice,
            seed_section: section,
            focus: 0,
        }
    }

    /// The fields visible for the current kind, in focus order: a git entry
    /// shows Fav, a folder/file shows Section.
    fn fields(&self) -> Vec<Field> {
        let mut fields = vec![Field::Path, Field::Name];
        match self.kind {
            RepoKind::Git => {
                fields.push(Field::Slug);
                fields.push(Field::Fav);
            }
            RepoKind::Path => {
                fields.push(Field::Section);
                fields.push(Field::Slug);
            }
        }
        // The backup toggle shows for both kinds; Kind sits at the very bottom.
        fields.push(Field::Backup);
        fields.push(Field::Kind);
        fields
    }

    /// Handles a key, returning a save draft, a cancel, or pending.
    pub fn handle_key(&mut self, key: KeyEvent) -> FormResult {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return FormResult::Cancel,
            KeyCode::Enter => return FormResult::Save(self.draft()),
            KeyCode::Char('s') if ctrl => {
                return FormResult::Save(self.draft());
            }
            KeyCode::Char('o') if ctrl => return FormResult::PickPath,
            KeyCode::Tab | KeyCode::Down => self.move_focus(1),
            KeyCode::BackTab | KeyCode::Up => {
                self.move_focus(self.fields().len() - 1)
            }
            _ => self.edit_focused(key),
        }
        FormResult::Pending
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
            Field::Section => match key.code {
                KeyCode::Left => self.cycle_section(-1),
                KeyCode::Right => self.cycle_section(1),
                _ => {}
            },
            Field::Kind => self.edit_kind(key),
            Field::Fav => {
                if key.code == KeyCode::Char(' ') {
                    self.fav = !self.fav;
                }
            }
            Field::Backup => {
                if key.code == KeyCode::Char(' ') {
                    self.include_in_backup = !self.include_in_backup;
                }
            }
        }
    }

    /// Cycles the kind on `Left`/`Right`, keeping focus on the Kind field even
    /// though the visible field set changes with the kind.
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

    /// Cycles the section choice by `delta` (wrapping).
    fn cycle_section(&mut self, delta: isize) {
        let len = self.section_options.len();
        if len == 0 {
            return;
        }
        let next =
            (self.section_choice as isize + delta).rem_euclid(len as isize);
        self.section_choice = next as usize;
    }

    /// The path field's current value (used to seed the path picker).
    pub fn path_value(&self) -> String {
        self.path.value()
    }

    /// Replaces the path field (after picking a path) and auto-fills the name
    /// from its basename when the name is still blank.
    pub fn set_path(&mut self, path: &str) {
        self.path = TextInput::new(path);
        self.autofill_name();
        self.focus = 0;
    }

    /// Fills the name from the path's basename when the name is still blank.
    fn autofill_name(&mut self) {
        if !self.name.value().trim().is_empty() {
            return;
        }
        if let Some(base) = basename(&self.path.value()) {
            self.name = TextInput::new(&base);
        }
    }

    /// Builds the draft from the current field values.
    fn draft(&self) -> RepoDraft {
        let name = non_empty(self.name.value());
        let slug = non_empty(slugify(&self.slug.value()));
        let section = if self.fields().contains(&Field::Section) {
            self.section_options
                .get(self.section_choice)
                .cloned()
                .flatten()
        } else {
            self.seed_section.clone()
        };
        RepoDraft {
            name,
            path: self.path.value(),
            slug,
            section,
            kind: self.kind,
            fav: self.fav,
            include_in_backup: self.include_in_backup,
        }
    }

    /// Renders the form centred in `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let fields = self.fields();
        let height = fields.len() as u16 + 4;
        let rect = centered_rect(70, height, area);
        frame.render_widget(Clear, rect);
        let block = Block::default()
            .title(Span::styled(
                format!(" {} ", self.title),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT));
        let mut lines: Vec<Line> = fields
            .iter()
            .enumerate()
            .map(|(index, field)| self.field_line(*field, index))
            .collect();
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "Tab field · \u{2190}\u{2192} change · Space fav · ^O pick path · \
             Enter save · Esc cancel",
            Style::default().fg(DIM),
        )));
        frame.render_widget(Paragraph::new(lines).block(block), rect);
    }

    /// Builds the rendered line for `field` at focus position `index`.
    fn field_line(&self, field: Field, index: usize) -> Line<'static> {
        match field {
            Field::Path => self.text_line("Path", &self.path, index),
            Field::Name => self.text_line("Name", &self.name, index),
            Field::Slug => self.text_line("Slug", &self.slug, index),
            Field::Section => {
                self.choice_line("Section", self.section_label(), index)
            }
            Field::Kind => {
                self.choice_line("Kind", kind_label(self.kind), index)
            }
            Field::Fav => self.fav_line(index),
            Field::Backup => self.backup_line(index),
        }
    }

    /// A labelled text field line, highlighting it when focused.
    fn text_line(
        &self,
        label: &str,
        input: &TextInput,
        index: usize,
    ) -> Line<'static> {
        let mut spans = vec![self.label_span(label, index)];
        let focused = index == self.focus;
        spans.extend(input.render_line(Style::default(), focused).spans);
        self.styled(spans, index)
    }

    /// A `< value >` selector line (section, kind).
    fn choice_line(
        &self,
        label: &str,
        value: &str,
        index: usize,
    ) -> Line<'static> {
        let spans = vec![
            self.label_span(label, index),
            Span::styled(format!("< {value} >"), Style::default().fg(ACCENT)),
        ];
        self.styled(spans, index)
    }

    /// The favourite toggle line.
    fn fav_line(&self, index: usize) -> Line<'static> {
        let mark = if self.fav { "[x]" } else { "[ ]" };
        let spans =
            vec![self.label_span("Fav", index), Span::raw(mark.to_string())];
        self.styled(spans, index)
    }

    /// The "include in backup all (`Z`)" toggle line.
    fn backup_line(&self, index: usize) -> Line<'static> {
        let mark = if self.include_in_backup { "[x]" } else { "[ ]" };
        let spans = vec![
            self.label_span("Backup", index),
            Span::raw(mark.to_string()),
        ];
        self.styled(spans, index)
    }

    /// The label of the currently selected section option.
    fn section_label(&self) -> &str {
        match self.section_options.get(self.section_choice) {
            Some(Some(name)) => name,
            _ => UNGROUPED,
        }
    }

    /// The fixed-width field label, accented when focused.
    fn label_span(&self, label: &str, index: usize) -> Span<'static> {
        let style = if index == self.focus {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(DIM)
        };
        Span::styled(format!("{label:<8}"), style)
    }

    /// Applies the focus background tint to a field line.
    fn styled(&self, spans: Vec<Span<'static>>, index: usize) -> Line<'static> {
        let line = Line::from(spans);
        if index == self.focus {
            line.style(Style::default().bg(SELECTION_BG))
        } else {
            line
        }
    }
}

/// The Section dropdown options: Ungrouped (`None`) first, then each section.
fn section_options(sections: &[String]) -> Vec<Option<String>> {
    let mut options = vec![None];
    options.extend(sections.iter().cloned().map(Some));
    options
}

/// The option index matching `section` (case-insensitive), or 0 (Ungrouped).
fn section_choice(options: &[Option<String>], section: Option<&str>) -> usize {
    let Some(name) = section else {
        return 0;
    };
    options
        .iter()
        .position(|option| {
            option
                .as_deref()
                .is_some_and(|o| o.eq_ignore_ascii_case(name))
        })
        .unwrap_or(0)
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
    fn section_choice_matches_case_insensitively() {
        let options = section_options(&["Work".to_string()]);
        assert_eq!(section_choice(&options, Some("work")), 1);
        assert_eq!(section_choice(&options, Some("Misc")), 0);
        assert_eq!(section_choice(&options, None), 0);
    }

    #[test]
    fn visible_fields_follow_the_kind() {
        let git = RepoForm::for_add("/p", RepoKind::Git, &[]);
        assert!(git.fields().contains(&Field::Fav));
        assert!(!git.fields().contains(&Field::Section));

        let folder = RepoForm::for_add("/p", RepoKind::Path, &[]);
        assert!(folder.fields().contains(&Field::Section));
        assert!(!folder.fields().contains(&Field::Fav));

        // The backup toggle shows for both kinds.
        assert!(git.fields().contains(&Field::Backup));
        assert!(folder.fields().contains(&Field::Backup));
    }

    #[test]
    fn for_add_seeds_backup_toggle_per_kind() {
        // Git repos default to included, file/folder entries to excluded.
        let git = RepoForm::for_add("/p", RepoKind::Git, &[]);
        assert!(git.draft().include_in_backup);
        let folder = RepoForm::for_add("/p", RepoKind::Path, &[]);
        assert!(!folder.draft().include_in_backup);
    }
}
