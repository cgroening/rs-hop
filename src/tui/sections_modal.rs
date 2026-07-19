//! The manage-sections overlay: a list of the user sections with keys to add,
//! rename, delete and reorder them. The implicit "Ungrouped" section is shown
//! as a locked trailing row that cannot be edited or moved.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState};

use crate::domain::sections::UNGROUPED;
use crate::theme::Skin;
use crate::tui::list_layout::moved_cursor;
use crate::tui::skin::Colors;
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
            _ if !alt && self.navigate(key) => SectionsAction::Pending,
            KeyCode::Char('n') => SectionsAction::New,
            KeyCode::Char('r') => self.rename_action(),
            KeyCode::Char('d') => self.delete_action(),
            _ => SectionsAction::Pending,
        }
    }

    /// Applies a navigation key through the shared helper, reporting whether it
    /// was one.
    fn navigate(&mut self, key: KeyEvent) -> bool {
        match moved_cursor(key, self.cursor, self.names.len()) {
            Some(cursor) => {
                self.cursor = cursor;
                true
            }
            None => false,
        }
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
    pub fn render(&self, frame: &mut Frame, area: Rect, skin: &Skin) {
        let colors = Colors::from_palette(&skin.palette);
        let height = (self.names.len() as u16 + 5).clamp(6, 18);
        let rect = centered_rect(50, height, area);
        frame.render_widget(Clear, rect);
        let block = ratada::chrome::modal_block(skin, "Manage sections");
        let inner = block.inner(rect);
        frame.render_widget(block, rect);

        let mut items: Vec<ListItem> = self
            .names
            .iter()
            .map(|name| ListItem::new(Line::raw(name.clone())))
            .collect();
        items.push(ListItem::new(Line::from(Span::styled(
            format!("{UNGROUPED} (auto)"),
            Style::default().fg(colors.dim),
        ))));
        items.push(ListItem::new(Line::raw("")));
        items.push(ListItem::new(Line::from(Span::styled(
            "n new · r rename · d delete · Alt+\u{2191}\u{2193} move · Esc close",
            Style::default().fg(colors.dim),
        ))));

        // The box is capped, so a long section list overflows. Keep the cursor
        // inside the visible window and show the shared scrollbar, rather than
        // silently cutting the rest off.
        let total = items.len();
        let viewport = inner.height as usize;
        let offset = ratada::nav::keep_visible(
            ratada::nav::ScrollView {
                total,
                offset: 0,
                viewport,
            },
            self.cursor,
        );
        let list = List::new(items).highlight_style(colors.selection_style());
        let mut state = ListState::default().with_offset(offset);
        if !self.names.is_empty() {
            state.select(Some(self.cursor));
        }
        frame.render_stateful_widget(list, inner, &mut state);
        ratada::scroll::render_scrollbar(
            frame,
            inner,
            skin,
            ratada::nav::ScrollView {
                total,
                offset,
                viewport,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modal() -> SectionsModal {
        SectionsModal::new(
            ["Work", "Personal", "Archive"]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            0,
        )
    }

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn press_alt(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::ALT)
    }

    #[test]
    fn the_cursor_is_clamped_into_range_on_construction() {
        let modal = SectionsModal::new(vec!["One".to_string()], 99);
        assert_eq!(modal.cursor(), 0);
        // An empty list has nowhere to point.
        assert_eq!(SectionsModal::new(Vec::new(), 3).cursor(), 0);
    }

    #[test]
    fn the_arrows_wrap_and_the_page_keys_clamp() {
        let mut modal = modal();
        modal.handle_key(press(KeyCode::Up));
        assert_eq!(
            modal.cursor(),
            2,
            "up from the first row wraps to the last"
        );
        modal.handle_key(press(KeyCode::Down));
        assert_eq!(modal.cursor(), 0);

        modal.handle_key(press(KeyCode::End));
        assert_eq!(modal.cursor(), 2);
        modal.handle_key(press(KeyCode::PageDown));
        assert_eq!(modal.cursor(), 2, "a page jump clamps at the end");
        modal.handle_key(press(KeyCode::PageUp));
        assert_eq!(modal.cursor(), 0, "and at the start");
    }

    #[test]
    fn alt_arrows_reorder_instead_of_moving_the_cursor() {
        let mut modal = modal();
        modal.handle_key(press(KeyCode::Down));
        assert_eq!(modal.cursor(), 1);

        let action = modal.handle_key(press_alt(KeyCode::Up));
        assert!(matches!(action, SectionsAction::Move { from: 1, to: 0 }));
        assert_eq!(modal.cursor(), 1, "the cursor follows the applied move");
    }

    #[test]
    fn a_reorder_past_either_end_is_refused() {
        let mut modal = modal();
        assert!(matches!(
            modal.handle_key(press_alt(KeyCode::Up)),
            SectionsAction::Pending
        ));
        modal.handle_key(press(KeyCode::End));
        assert!(matches!(
            modal.handle_key(press_alt(KeyCode::Down)),
            SectionsAction::Pending
        ));
    }

    #[test]
    fn the_letter_keys_request_their_actions_for_the_cursor_section() {
        let mut modal = modal();
        assert!(matches!(
            modal.handle_key(press(KeyCode::Char('n'))),
            SectionsAction::New
        ));
        assert!(matches!(
            modal.handle_key(press(KeyCode::Char('r'))),
            SectionsAction::Rename(name) if name == "Work"
        ));
        assert!(matches!(
            modal.handle_key(press(KeyCode::Char('d'))),
            SectionsAction::Delete(name) if name == "Work"
        ));
        assert!(matches!(
            modal.handle_key(press(KeyCode::Esc)),
            SectionsAction::Close
        ));
    }

    #[test]
    fn an_empty_list_has_nothing_to_rename_or_delete() {
        let mut modal = SectionsModal::new(Vec::new(), 0);
        assert!(matches!(
            modal.handle_key(press(KeyCode::Char('r'))),
            SectionsAction::Pending
        ));
        assert!(matches!(
            modal.handle_key(press(KeyCode::Char('d'))),
            SectionsAction::Pending
        ));
    }
}
