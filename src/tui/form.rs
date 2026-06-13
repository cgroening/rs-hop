//! The add/edit form for an entry: path, name, slug, kind and favourite.
//!
//! All fields are visible at once; `Tab`/`BackTab` (or `Up`/`Down`) step
//! between them, the text fields edit inline, `Left`/`Right` cycle the kind,
//! `Space` toggles the favourite, `^O` opens the path picker, `Enter`/`Ctrl+S`
//! save and `Esc` cancels. When the name is still blank, it is auto-filled from
//! the path's basename (after picking a path, or when the name field is
//! focused).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::domain::repo::{Repo, RepoKind};
use crate::domain::slug::slugify;
use crate::tui::colors::{ACCENT, DIM, SELECTION_BG};
use crate::tui::text_input::TextInput;
use crate::tui::widgets::centered_rect;

/// Field positions in display (focus) order.
const PATH_FIELD: usize = 0;
const NAME_FIELD: usize = 1;
const SECTION_FIELD: usize = 2;
const SLUG_FIELD: usize = 3;
const KIND_FIELD: usize = 4;
const FAV_FIELD: usize = 5;
/// The number of fields in the form.
const FIELD_COUNT: usize = 6;

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
    section: TextInput,
    kind: RepoKind,
    fav: bool,
    focus: usize,
    known_sections: Vec<String>,
}

impl RepoForm {
    /// A blank add form, optionally seeded with a `path` and guessed `kind`.
    pub fn for_add(path: &str, kind: RepoKind) -> Self {
        RepoForm {
            title: "Add entry".to_string(),
            name: TextInput::new(""),
            path: TextInput::new(path),
            slug: TextInput::new(""),
            section: TextInput::new(""),
            kind,
            fav: false,
            focus: PATH_FIELD,
            known_sections: Vec::new(),
        }
    }

    /// An edit form seeded from an existing entry's fields.
    pub fn for_edit(repo: &Repo) -> Self {
        RepoForm {
            title: "Edit entry".to_string(),
            name: TextInput::new(repo.name.as_deref().unwrap_or("")),
            path: TextInput::new(&repo.path.to_string_lossy()),
            slug: TextInput::new(repo.slug.as_deref().unwrap_or("")),
            section: TextInput::new(repo.section.as_deref().unwrap_or("")),
            kind: repo.kind,
            fav: repo.fav,
            focus: PATH_FIELD,
            known_sections: Vec::new(),
        }
    }

    /// Records the existing section names shown as a hint under the form.
    pub fn with_known_sections(mut self, sections: &[String]) -> Self {
        self.known_sections = sections.to_vec();
        self
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
            KeyCode::BackTab | KeyCode::Up => self.move_focus(FIELD_COUNT - 1),
            _ => self.edit_focused(key),
        }
        FormResult::Pending
    }

    /// Moves focus by `delta` (wrapping), auto-filling the name on arrival.
    fn move_focus(&mut self, delta: usize) {
        self.focus = (self.focus + delta) % FIELD_COUNT;
        if self.focus == NAME_FIELD {
            self.autofill_name();
        }
    }

    /// Routes an editing key to the focused field.
    fn edit_focused(&mut self, key: KeyEvent) {
        match self.focus {
            PATH_FIELD => {
                self.path.handle_key(key);
            }
            NAME_FIELD => {
                self.name.handle_key(key);
            }
            SECTION_FIELD => {
                self.section.handle_key(key);
            }
            SLUG_FIELD => {
                self.slug.handle_key(key);
            }
            KIND_FIELD => match key.code {
                KeyCode::Left => self.kind = prev_kind(self.kind),
                KeyCode::Right => self.kind = next_kind(self.kind),
                _ => {}
            },
            _ => {
                if key.code == KeyCode::Char(' ') {
                    self.fav = !self.fav;
                }
            }
        }
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
        self.focus = PATH_FIELD;
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
        let section = non_empty(self.section.value().trim().to_string());
        RepoDraft {
            name,
            path: self.path.value(),
            slug,
            section,
            kind: self.kind,
            fav: self.fav,
        }
    }

    /// Renders the form centred in `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let rect = centered_rect(70, 13, area);
        frame.render_widget(Clear, rect);
        let block = Block::default()
            .title(Span::styled(
                format!(" {} ", self.title),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT));
        let lines = vec![
            self.text_line("Path", &self.path, PATH_FIELD),
            self.text_line("Name", &self.name, NAME_FIELD),
            self.text_line("Section", &self.section, SECTION_FIELD),
            self.text_line("Slug", &self.slug, SLUG_FIELD),
            self.kind_line(KIND_FIELD),
            self.fav_line(FAV_FIELD),
            self.known_sections_line(),
            Line::from(Span::styled(
                "Tab field · \u{2190}\u{2192} kind · Space fav · ^O pick path · \
                 Enter save · Esc cancel",
                Style::default().fg(DIM),
            )),
        ];
        frame.render_widget(Paragraph::new(lines).block(block), rect);
    }

    /// A dim line listing the existing section names, or blank when there are
    /// none (a new name typed in the Section field creates a section on save).
    fn known_sections_line(&self) -> Line<'static> {
        if self.known_sections.is_empty() {
            return Line::raw("");
        }
        Line::from(Span::styled(
            format!("existing: {}", self.known_sections.join(", ")),
            Style::default().fg(DIM),
        ))
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

    /// The kind selector line.
    fn kind_line(&self, index: usize) -> Line<'static> {
        let spans = vec![
            self.label_span("Kind", index),
            Span::styled(
                format!("< {} >", kind_label(self.kind)),
                Style::default().fg(ACCENT),
            ),
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

/// The next kind in the cycle.
fn next_kind(kind: RepoKind) -> RepoKind {
    match kind {
        RepoKind::Git => RepoKind::Folder,
        RepoKind::Folder => RepoKind::File,
        RepoKind::File => RepoKind::Git,
    }
}

/// The previous kind in the cycle.
fn prev_kind(kind: RepoKind) -> RepoKind {
    match kind {
        RepoKind::Git => RepoKind::File,
        RepoKind::Folder => RepoKind::Git,
        RepoKind::File => RepoKind::Folder,
    }
}

/// The display label for a kind.
fn kind_label(kind: RepoKind) -> &'static str {
    match kind {
        RepoKind::Git => "git",
        RepoKind::Folder => "folder",
        RepoKind::File => "file",
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
}
