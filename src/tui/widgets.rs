//! Reusable modal widgets: confirm, text prompt and single-select.
//!
//! hop keeps them as small non-blocking state structs (`handle_key` and
//! `render`), because its event loop drains background workers between
//! frames. The look is the toolkit's: the modal frame, fill and padding
//! (`overlay::framed`), a box as big as its content plus
//! `MODAL_CHROME_WIDTH`/`MODAL_CHROME_HEIGHT`, the modal hint block at the
//! bottom and, for a list, the pointer cursor and the `n/m` badge.

use std::cell::Cell;

use crossterm::event::{KeyCode, KeyEvent};
use ratada::chrome::{self, MODAL_CHROME_HEIGHT, MODAL_CHROME_WIDTH};
use ratada::input::InputField;
use ratada::layout::centered_rect;
use ratada::list::{self, CursorMark, ListView};
use ratada::modal::{hint_block, hint_block_height, hints_width};
use ratada::overlay;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::theme::Skin;
use crate::tui::list_layout::moved_cursor;
use crate::tui::presentation::{FieldView, field_spans};
use crate::tui::skin::Colors;

/// The keys of a confirmation.
const CONFIRM_HINTS: [(&str, &str); 2] = [("y", "confirm"), ("n", "cancel")];
/// The keys of a text prompt.
const PROMPT_HINTS: [(&str, &str); 2] = [("enter", "save"), ("esc", "cancel")];
/// The keys of a select list.
const SELECT_HINTS: [(&str, &str); 2] = [("enter", "pick"), ("esc", "cancel")];
/// The fewest columns a prompt's value gets, however short it is.
const PROMPT_VALUE_WIDTH: usize = 32;
/// The most rows a select list shows before it scrolls.
const SELECT_MAX_ROWS: u16 = 14;
/// The pointer column and the scrollbar beside a list row.
const LIST_ROW_CHROME: usize = 3;
/// The check after the entry that applies (gap and glyph).
const CURRENT_MARK_WIDTH: usize = 2;

/// Outcome of feeding a key to a confirm dialog.
pub enum ConfirmResult {
    /// Still open.
    Pending,
    /// The user confirmed (`y`/Enter).
    Yes,
    /// The user declined (`n`/Esc).
    No,
}

/// A yes/no confirmation.
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

    /// Renders the dialog centred in `area`, as big as its message and keys.
    pub fn render(&self, frame: &mut Frame, area: Rect, skin: &Skin) {
        let colors = Colors::from_palette(&skin.palette);
        let message: Vec<&str> = self.message.lines().collect();
        let width = message
            .iter()
            .map(|line| line.width())
            .max()
            .unwrap_or(0)
            .max(hints_width(&CONFIRM_HINTS));
        let rows = message.len() as u16
            + fitted_hint_rows(&CONFIRM_HINTS, width, area);
        let inner = open_box(frame, area, skin, &self.title, (width, rows));
        let mut lines: Vec<Line> = message
            .iter()
            .map(|line| {
                Line::from(Span::styled(
                    (*line).to_string(),
                    Style::default().fg(colors.danger),
                ))
            })
            .collect();
        let inner_width = usize::from(inner.width);
        lines.extend(hint_block(&CONFIRM_HINTS, &skin.palette, inner_width));
        frame.render_widget(Paragraph::new(lines), inner);
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
    input: InputField,
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
            input: InputField::new(initial),
        }
    }

    /// Handles a key: Enter submits, Esc cancels, the rest edits the value.
    pub fn handle_key(&mut self, key: KeyEvent) -> PromptResult {
        match key.code {
            KeyCode::Enter => PromptResult::Submit(self.input.value().into()),
            KeyCode::Esc => PromptResult::Cancel,
            _ => {
                self.input.handle_key(key);
                PromptResult::Pending
            }
        }
    }

    /// Inserts a bracketed paste into the value at the caret.
    pub fn paste(&mut self, text: &str) {
        self.input.paste(text);
    }

    /// Renders the prompt centred in `area`: the labelled field over the keys.
    pub fn render(&self, frame: &mut Frame, area: Rect, skin: &Skin) {
        let colors = Colors::from_palette(&skin.palette);
        let label = format!("{}  ", self.label);
        let value = self.input.value().width().max(PROMPT_VALUE_WIDTH);
        let width = (label.width() + value + 1).max(hints_width(&PROMPT_HINTS));
        let rows = 1 + fitted_hint_rows(&PROMPT_HINTS, width, area);
        let inner = open_box(frame, area, skin, &self.title, (width, rows));
        let inner_width = usize::from(inner.width);
        let mut field =
            vec![Span::styled(label.clone(), Style::default().fg(colors.dim))];
        field.extend(field_spans(FieldView {
            field: &self.input,
            palette: &skin.palette,
            width: inner_width.saturating_sub(label.width()),
            focused: true,
        }));
        let mut lines = vec![Line::from(field)];
        lines.extend(hint_block(&PROMPT_HINTS, &skin.palette, inner_width));
        frame.render_widget(Paragraph::new(lines), inner);
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
    /// The entry that applies right now; it keeps the check glyph.
    current: Option<usize>,
    offset: Cell<usize>,
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
            current: None,
            offset: Cell::new(0),
        }
    }

    /// Marks `index` as the entry that applies: the list opens on it and it
    /// keeps the check glyph while the cursor moves.
    #[must_use]
    pub fn with_current(mut self, index: usize) -> Self {
        if index < self.items.len() {
            self.cursor = index;
            self.current = Some(index);
        }
        self
    }

    /// Handles navigation and selection keys.
    pub fn handle_key(&mut self, key: KeyEvent) -> SelectResult {
        match key.code {
            KeyCode::Enter => SelectResult::Selected(self.cursor),
            KeyCode::Esc => SelectResult::Cancel,
            _ => {
                if let Some(cursor) =
                    moved_cursor(key, self.cursor, self.items.len())
                {
                    self.cursor = cursor;
                }
                SelectResult::Pending
            }
        }
    }

    /// Renders the list centred in `area`: as wide as its widest entry, as
    /// tall as its entries (scrolling beyond `SELECT_MAX_ROWS`), the pointer
    /// on the cursor row, the badge in the bottom border and the keys below.
    pub fn render(&self, frame: &mut Frame, area: Rect, skin: &Skin) {
        let mark = if self.current.is_some() {
            CURRENT_MARK_WIDTH
        } else {
            0
        };
        let labels = self.items.iter().map(|item| item.width()).max();
        let width = (labels.unwrap_or(0) + mark + LIST_ROW_CHROME)
            .max(hints_width(&SELECT_HINTS));
        let list_rows = (self.items.len() as u16).clamp(1, SELECT_MAX_ROWS);
        let rows = list_rows + fitted_hint_rows(&SELECT_HINTS, width, area);
        let rect = box_rect(area, (width, rows));
        frame.render_widget(Clear, rect);
        let inner = overlay::framed(frame, rect, skin, &self.title);
        let hint_rows =
            hint_block_height(&SELECT_HINTS, usize::from(inner.width));
        let list_area = Rect {
            height: inner.height.saturating_sub(hint_rows),
            ..inner
        };
        let check = skin.glyphs.check;
        let rows: Vec<Line<'static>> = self
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                if self.current == Some(index) {
                    Line::from(vec![
                        Span::raw(item.clone()),
                        Span::styled(
                            format!(" {check}"),
                            ratada::style::accent(&skin.palette),
                        ),
                    ])
                } else {
                    Line::from(item.clone())
                }
            })
            .collect();
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
            height: hint_rows,
            ..inner
        };
        let hints =
            hint_block(&SELECT_HINTS, &skin.palette, usize::from(inner.width));
        frame.render_widget(Paragraph::new(hints), hint_area);
        let badge = chrome::position_badge(self.cursor, self.items.len());
        chrome::render_badge(frame, rect, skin, &badge);
    }
}

/// The rows the hint block of `hints` takes in a box of `width` content
/// columns centred in `area`: more than one while a narrow screen narrows
/// the box and the hints wrap.
pub fn fitted_hint_rows(
    hints: &[(&str, &str)],
    width: usize,
    area: Rect,
) -> u16 {
    let inner = width.min(
        usize::from(area.width).saturating_sub(usize::from(MODAL_CHROME_WIDTH)),
    );
    hint_block_height(hints, inner)
}

/// The rect of a modal box whose content is `width` columns by `rows` rows,
/// centred in `area`: the content plus the frame and the padding, never
/// larger than `area`.
pub fn box_rect(area: Rect, (width, rows): (usize, u16)) -> Rect {
    let width = u16::try_from(width)
        .unwrap_or(u16::MAX)
        .saturating_add(MODAL_CHROME_WIDTH);
    centered_rect(width, rows.saturating_add(MODAL_CHROME_HEIGHT), area)
}

/// Clears and frames a box for `content` (columns, rows) centred in `area`,
/// titled `title`, and returns the padded content rect.
fn open_box(
    frame: &mut Frame,
    area: Rect,
    skin: &Skin,
    title: &str,
    content: (usize, u16),
) -> Rect {
    let rect = box_rect(area, content);
    frame.render_widget(Clear, rect);
    overlay::framed(frame, rect, skin, title)
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    #[test]
    fn a_box_is_its_content_plus_the_modal_chrome() {
        let rect = box_rect(Rect::new(0, 0, 80, 24), (20, 4));
        assert_eq!(rect.width, 20 + MODAL_CHROME_WIDTH);
        assert_eq!(rect.height, 4 + MODAL_CHROME_HEIGHT);
    }

    #[test]
    fn an_oversized_box_shrinks_to_the_area() {
        let area = Rect::new(0, 0, 40, 10);
        assert_eq!(box_rect(area, (200, 99)), area);
    }

    #[test]
    fn the_current_entry_opens_under_the_cursor_with_its_check() {
        let modal = SelectModal::new("Sort", vec!["a".into(), "b".into()], 0)
            .with_current(1);
        let text = rendered(&modal, 60, 20);
        assert!(text.contains("\u{203a} b \u{2713}"), "{text}");
        assert!(text.contains("enter pick"), "{text}");
        assert!(text.contains("2/2"), "{text}");
    }

    #[test]
    fn a_confirm_keeps_its_keys_in_the_hint_block() {
        let skin = crate::config::Config::default().skin();
        let modal = ConfirmModal::new("Delete", "Delete 2 entries?");
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        terminal
            .draw(|frame| modal.render(frame, frame.area(), &skin))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("y confirm \u{b7} n cancel"), "{text}");
    }

    /// Collects the whole rendered buffer into one string.
    fn rendered(modal: &SelectModal, width: u16, height: u16) -> String {
        let skin = crate::config::Config::default().skin();
        let mut terminal =
            Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| modal.render(frame, frame.area(), &skin))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    fn many(count: usize) -> SelectModal {
        let items = (0..count).map(|i| format!("item {i}")).collect();
        SelectModal::new("Errors", items, count - 1)
    }

    #[test]
    fn shows_scrollbar_when_items_overflow() {
        let modal = many(40);
        assert!(
            rendered(&modal, 60, 20).contains('█'),
            "an overflowing list should show a scrollbar thumb"
        );
    }

    #[test]
    fn hides_scrollbar_when_items_fit() {
        let modal = SelectModal::new("Errors", vec!["only".to_string()], 0);
        assert!(
            !rendered(&modal, 60, 20).contains('█'),
            "a list that fits should not show a scrollbar"
        );
    }
}
