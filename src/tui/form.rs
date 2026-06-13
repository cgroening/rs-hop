//! The add/edit form for an entry: name, path, slug, kind and favourite.
//!
//! All fields are visible at once; `Tab`/`BackTab` step between them, the text
//! fields edit inline, `Left`/`Right` cycle the kind, `Space` toggles the
//! favourite, `Enter`/`Ctrl+S` save and `Esc` cancels.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::domain::repo::RepoKind;
use crate::domain::slug::slugify;
use crate::tui::colors::{ACCENT, DIM, SELECTION_BG};
use crate::tui::text_input::TextInput;
use crate::tui::widgets::centered_rect;

/// The number of fields in the form.
const FIELD_COUNT: usize = 5;

/// The values captured by the form on save.
pub struct RepoDraft {
    /// Explicit name, or `None` to fall back to the path's basename.
    pub name: Option<String>,
    /// The entry path as typed.
    pub path: String,
    /// Slug, or `None` when blank.
    pub slug: Option<String>,
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
    focus: usize,
}

impl RepoForm {
    /// A blank add form, optionally seeded with a `path` and guessed `kind`.
    pub fn for_add(path: &str, kind: RepoKind) -> Self {
        RepoForm {
            title: "Add entry".to_string(),
            name: TextInput::new(""),
            path: TextInput::new(path),
            slug: TextInput::new(""),
            kind,
            fav: false,
            focus: 0,
        }
    }

    /// An edit form seeded from an existing entry's fields.
    pub fn for_edit(
        name: &str,
        path: &str,
        slug: &str,
        kind: RepoKind,
        fav: bool,
    ) -> Self {
        RepoForm {
            title: "Edit entry".to_string(),
            name: TextInput::new(name),
            path: TextInput::new(path),
            slug: TextInput::new(slug),
            kind,
            fav,
            focus: 0,
        }
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
            KeyCode::Tab => self.focus = (self.focus + 1) % FIELD_COUNT,
            KeyCode::BackTab => {
                self.focus = (self.focus + FIELD_COUNT - 1) % FIELD_COUNT;
            }
            _ => self.edit_focused(key),
        }
        FormResult::Pending
    }

    /// Routes an editing key to the focused field.
    fn edit_focused(&mut self, key: KeyEvent) {
        match self.focus {
            0 => {
                self.name.handle_key(key);
            }
            1 => {
                self.path.handle_key(key);
            }
            2 => {
                self.slug.handle_key(key);
            }
            3 => match key.code {
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

    /// Builds the draft from the current field values.
    fn draft(&self) -> RepoDraft {
        let name = non_empty(self.name.value());
        let slug = non_empty(slugify(&self.slug.value()));
        RepoDraft {
            name,
            path: self.path.value(),
            slug,
            kind: self.kind,
            fav: self.fav,
        }
    }

    /// Renders the form centred in `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let rect = centered_rect(70, 11, area);
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
            self.text_line("Name", &self.name, 0),
            self.text_line("Path", &self.path, 1),
            self.text_line("Slug", &self.slug, 2),
            self.kind_line(3),
            self.fav_line(4),
            Line::raw(""),
            Line::from(Span::styled(
                "Tab field · \u{2190}\u{2192} kind · Space fav · Enter save · \
                 Esc cancel",
                Style::default().fg(DIM),
            )),
        ];
        frame.render_widget(Paragraph::new(lines).block(block), rect);
    }

    /// A labelled text field line, highlighting it when focused.
    fn text_line(
        &self,
        label: &str,
        input: &TextInput,
        index: usize,
    ) -> Line<'static> {
        let mut spans = vec![self.label_span(label, index)];
        spans.extend(input.render_line(Style::default()).spans);
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
        Span::styled(format!("{label:<6}"), style)
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
