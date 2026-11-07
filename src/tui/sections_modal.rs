//! The manage-sections overlay: a list of the user sections with keys to add,
//! rename, delete and reorder them. The implicit "Ungrouped" section is shown
//! as a locked trailing row that cannot be edited or moved.

use std::cell::Cell;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratada::chrome;
use ratada::list::{self, CursorMark, ListView};
use ratada::modal::{HINT_BLOCK_ROWS, hint_block, hints_width};
use ratada::overlay;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::domain::sections::UNGROUPED;
use crate::theme::Skin;
use crate::tui::list_layout::moved_cursor;
use crate::tui::skin::Colors;
use crate::tui::widgets::box_rect;

/// The manager's keys, in the modal hint block.
const HINTS: [(&str, &str); 5] = [
    ("n", "new"),
    ("r", "rename"),
    ("d", "delete"),
    ("alt+\u{2191}\u{2193}", "move"),
    ("esc", "close"),
];
/// The most section rows shown before the list scrolls.
const MAX_ROWS: u16 = 14;
/// The pointer column and the scrollbar beside a row.
const ROW_CHROME: usize = 3;

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
    offset: Cell<usize>,
}

impl SectionsModal {
    /// Builds the manager over `names`, with the cursor clamped into range.
    pub fn new(names: Vec<String>, cursor: usize) -> Self {
        let cursor = cursor.min(names.len().saturating_sub(1));
        SectionsModal {
            names,
            cursor,
            offset: Cell::new(0),
        }
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

    /// Renders the manager centred in `area`: the sections with the pointer
    /// cursor (the locked "Ungrouped" row dimmed below them), the badge in
    /// the bottom border and the keys in the hint block.
    pub fn render(&self, frame: &mut Frame, area: Rect, skin: &Skin) {
        let colors = Colors::from_palette(&skin.palette);
        let ungrouped = format!("{UNGROUPED} (auto)");
        let widest = self
            .names
            .iter()
            .map(|name| name.width())
            .chain(std::iter::once(ungrouped.width()))
            .max()
            .unwrap_or(0);
        let width = (widest + ROW_CHROME).max(hints_width(&HINTS));
        let list_rows = (self.names.len() as u16 + 1).min(MAX_ROWS);
        let rect = box_rect(area, (width, list_rows + HINT_BLOCK_ROWS));
        frame.render_widget(Clear, rect);
        let inner = overlay::framed(frame, rect, skin, "Manage sections");
        let list_area = Rect {
            height: inner.height.saturating_sub(HINT_BLOCK_ROWS),
            ..inner
        };

        let mut rows: Vec<Line> = self
            .names
            .iter()
            .map(|name| Line::raw(name.clone()))
            .collect();
        rows.push(Line::from(Span::styled(
            ungrouped,
            Style::default().fg(colors.dim),
        )));
        list::render_marked(
            frame,
            list_area,
            skin,
            ListView {
                rows,
                selected: self.cursor,
                offset: &self.offset,
            },
            CursorMark::Pointer,
        );
        let hint_area = Rect {
            y: list_area.bottom(),
            height: HINT_BLOCK_ROWS,
            ..inner
        };
        let hints = hint_block(&HINTS, &skin.palette, usize::from(inner.width));
        frame.render_widget(Paragraph::new(hints), hint_area);
        let badge = chrome::position_badge(self.cursor, self.names.len());
        chrome::render_badge(frame, rect, skin, &badge);
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
