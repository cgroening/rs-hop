//! The manage-sections overlay: a list of the user sections with keys to add,
//! rename, delete and reorder them. The implicit "Ungrouped" section is shown
//! as a locked trailing row that cannot be edited or moved.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState,
};

use crate::domain::sections::UNGROUPED;
use crate::tui::colors::{ACCENT, DIM, selection_style};
use crate::tui::navigation::cycle;
use crate::tui::widgets::centered_rect;

/// What a key press on the sections manager asks the app to do.
pub enum SectionsAction {
    /// Still open.
    Pending,
    /// Close the manager.
    Close,
    /// Prompt for a new section name.
    New,
    /// Prompt to rename the named section.
    Rename(String),
    /// Confirm deleting the named section.
    Delete(String),
    /// Move the section from one position to another in the order.
    Move {
        /// Current index.
        from: usize,
        /// Target index.
        to: usize,
    },
}

/// The manage-sections list state.
pub struct SectionsModal {
    names: Vec<String>,
    cursor: usize,
}

impl SectionsModal {
    /// Builds the manager over `names`, with the cursor clamped into range.
    pub fn new(names: Vec<String>, cursor: usize) -> Self {
        let cursor = cursor.min(names.len().saturating_sub(1));
        SectionsModal { names, cursor }
    }

    /// The cursor position (used to re-open the manager after a change).
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Handles a key, returning the requested action.
    pub fn handle_key(&mut self, key: KeyEvent) -> SectionsAction {
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => SectionsAction::Close,
            KeyCode::Up if alt => self.move_by(-1),
            KeyCode::Down if alt => self.move_by(1),
            KeyCode::Up => self.step(-1),
            KeyCode::Down => self.step(1),
            KeyCode::Char('n') => SectionsAction::New,
            KeyCode::Char('r') => self.rename_action(),
            KeyCode::Char('d') => self.delete_action(),
            _ => SectionsAction::Pending,
        }
    }

    /// Moves the cursor by `delta` (cyclic) over the real sections.
    fn step(&mut self, delta: isize) -> SectionsAction {
        if !self.names.is_empty() {
            self.cursor = cycle(self.cursor, self.names.len(), delta);
        }
        SectionsAction::Pending
    }

    /// Requests a reorder of the cursor section by `delta`, staying in range.
    fn move_by(&mut self, delta: isize) -> SectionsAction {
        let target = self.cursor as isize + delta;
        if target < 0 || target as usize >= self.names.len() {
            return SectionsAction::Pending;
        }
        SectionsAction::Move {
            from: self.cursor,
            to: target as usize,
        }
    }

    /// A rename action for the cursor section, if any.
    fn rename_action(&self) -> SectionsAction {
        match self.names.get(self.cursor) {
            Some(name) => SectionsAction::Rename(name.clone()),
            None => SectionsAction::Pending,
        }
    }

    /// A delete action for the cursor section, if any.
    fn delete_action(&self) -> SectionsAction {
        match self.names.get(self.cursor) {
            Some(name) => SectionsAction::Delete(name.clone()),
            None => SectionsAction::Pending,
        }
    }

    /// Renders the manager centred in `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let height = (self.names.len() as u16 + 5).clamp(6, 18);
        let rect = centered_rect(50, height, area);
        frame.render_widget(Clear, rect);
        let block = Block::default()
            .title(Span::styled(
                " Manage sections ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);

        let mut items: Vec<ListItem> = self
            .names
            .iter()
            .map(|name| ListItem::new(Line::raw(name.clone())))
            .collect();
        items.push(ListItem::new(Line::from(Span::styled(
            format!("{UNGROUPED} (auto)"),
            Style::default().fg(DIM),
        ))));
        items.push(ListItem::new(Line::raw("")));
        items.push(ListItem::new(Line::from(Span::styled(
            "n new · r rename · d delete · Alt+\u{2191}\u{2193} move · Esc close",
            Style::default().fg(DIM),
        ))));

        let list = List::new(items).highlight_style(selection_style());
        let mut state = ListState::default();
        if !self.names.is_empty() {
            state.select(Some(self.cursor));
        }
        frame.render_stateful_widget(list, inner, &mut state);
    }
}
