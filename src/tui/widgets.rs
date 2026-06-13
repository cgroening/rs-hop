//! Reusable modal widgets: confirm, text prompt, message and single-select.
//!
//! Destructive actions always confirm; a text prompt drives the slug input; the
//! select list backs the form's kind picker. Each is a small state struct with
//! `handle_key` and `render`, so call sites never rebuild the dialog.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph,
};

use crate::tui::colors::{ACCENT, DANGER, DIM, selection_style};
use crate::tui::navigation::cycle;
use crate::tui::text_input::TextInput;

/// Outcome of feeding a key to a confirm dialog.
pub enum ConfirmResult {
    /// Still open.
    Pending,
    /// The user confirmed (`y`/Enter).
    Yes,
    /// The user declined (`n`/Esc).
    No,
}

/// A yes/no confirmation, defaulting to no.
pub struct ConfirmModal {
    title: String,
    message: String,
}

impl ConfirmModal {
    /// Builds a confirmation with `title` and `message`.
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        ConfirmModal {
            title: title.into(),
            message: message.into(),
        }
    }

    /// Handles a key, mapping `y`/Enter to yes and `n`/Esc to no.
    pub fn handle_key(&self, key: KeyEvent) -> ConfirmResult {
        match key.code {
            KeyCode::Char('y' | 'Y') | KeyCode::Enter => ConfirmResult::Yes,
            KeyCode::Char('n' | 'N') | KeyCode::Esc => ConfirmResult::No,
            _ => ConfirmResult::Pending,
        }
    }

    /// Renders the dialog centred in `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let body = vec![
            Line::from(Span::raw(self.message.clone())),
            Line::raw(""),
            Line::from(vec![
                Span::styled("y", Style::default().fg(ACCENT)),
                Span::styled(" confirm   ", Style::default().fg(DIM)),
                Span::styled("n", Style::default().fg(ACCENT)),
                Span::styled(" cancel", Style::default().fg(DIM)),
            ]),
        ];
        render_modal(frame, area, &self.title, DANGER, body, 60, 7);
    }
}

/// Outcome of feeding a key to a text prompt.
pub enum PromptResult {
    /// Still editing.
    Pending,
    /// The user submitted this value.
    Submit(String),
    /// The user cancelled.
    Cancel,
}

/// A single-line text prompt (used for the slug).
pub struct TextPrompt {
    title: String,
    label: String,
    input: TextInput,
}

impl TextPrompt {
    /// Builds a prompt with `title`, a field `label` and an initial value.
    pub fn new(
        title: impl Into<String>,
        label: impl Into<String>,
        initial: &str,
    ) -> Self {
        TextPrompt {
            title: title.into(),
            label: label.into(),
            input: TextInput::new(initial),
        }
    }

    /// Handles a key: Enter submits, Esc cancels, the rest edits the value.
    pub fn handle_key(&mut self, key: KeyEvent) -> PromptResult {
        match key.code {
            KeyCode::Enter => PromptResult::Submit(self.input.value()),
            KeyCode::Esc => PromptResult::Cancel,
            _ => {
                self.input.handle_key(key);
                PromptResult::Pending
            }
        }
    }

    /// Renders the prompt centred in `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let mut field = vec![Span::styled(
            format!("{}: ", self.label),
            Style::default().fg(DIM),
        )];
        field.extend(self.input.render_line(Style::default(), true).spans);
        let body = vec![
            Line::from(field),
            Line::raw(""),
            Line::from(Span::styled(
                "Enter save · Esc cancel",
                Style::default().fg(DIM),
            )),
        ];
        render_modal(frame, area, &self.title, ACCENT, body, 60, 7);
    }
}

/// Outcome of feeding a key to a select list.
pub enum SelectResult {
    /// Still open.
    Pending,
    /// The user chose this index.
    Selected(usize),
    /// The user cancelled.
    Cancel,
}

/// A single-select list with cyclic navigation.
pub struct SelectModal {
    title: String,
    items: Vec<String>,
    cursor: usize,
}

impl SelectModal {
    /// Builds a select list over `items` with an initial cursor (clamped).
    pub fn new(
        title: impl Into<String>,
        items: Vec<String>,
        cursor: usize,
    ) -> Self {
        let len = items.len();
        let cursor = if len == 0 { 0 } else { cursor.min(len - 1) };
        SelectModal {
            title: title.into(),
            items,
            cursor,
        }
    }

    /// Handles navigation and selection keys.
    pub fn handle_key(&mut self, key: KeyEvent) -> SelectResult {
        match key.code {
            KeyCode::Up => {
                self.cursor = cycle(self.cursor, self.items.len(), -1);
                SelectResult::Pending
            }
            KeyCode::Down => {
                self.cursor = cycle(self.cursor, self.items.len(), 1);
                SelectResult::Pending
            }
            KeyCode::Enter => SelectResult::Selected(self.cursor),
            KeyCode::Esc => SelectResult::Cancel,
            _ => SelectResult::Pending,
        }
    }

    /// Renders the list centred in `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let height = (self.items.len() as u16 + 2).clamp(3, 16);
        let rect = centered_rect(50, height, area);
        frame.render_widget(Clear, rect);
        let block = Block::default()
            .title(Span::styled(
                format!(" {} ", self.title),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT));
        let items: Vec<ListItem> = self
            .items
            .iter()
            .map(|item| ListItem::new(item.clone()))
            .collect();
        let list = List::new(items)
            .block(block)
            .highlight_style(selection_style());
        let mut state = ListState::default();
        state.select(Some(self.cursor));
        frame.render_stateful_widget(list, rect, &mut state);
    }
}

/// Computes a centred rect of `width` percent and `height` rows within `area`.
pub fn centered_rect(width_percent: u16, height: u16, area: Rect) -> Rect {
    let height = height.min(area.height);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((area.height.saturating_sub(height)) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vertical[1])[1]
}

/// Renders a bordered modal with `title`, `border` colour and `body` lines.
fn render_modal(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    border: ratatui::style::Color,
    body: Vec<Line<'static>>,
    width_percent: u16,
    height: u16,
) {
    let rect = centered_rect(width_percent, height, area);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(border).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border));
    let paragraph = Paragraph::new(body).block(block);
    frame.render_widget(paragraph, rect);
}
