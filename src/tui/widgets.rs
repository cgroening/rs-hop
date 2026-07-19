//! Reusable modal widgets: confirm, text prompt, message and single-select.
//!
//! Destructive actions always confirm; a text prompt drives the slug input; the
//! select list backs the form's kind picker. Each is a small state struct with
//! `handle_key` and `render`, so call sites never rebuild the dialog.

use crossterm::event::{KeyCode, KeyEvent};
use ratada::input::InputField;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph};

use crate::theme::Skin;
use crate::tui::list_layout::moved_cursor;
use crate::tui::presentation::{FieldView, field_spans};
use crate::tui::skin::Colors;

/// The width of hop's small confirm/prompt dialogs, in percent of the screen.
const DIALOG_WIDTH_PCT: u16 = 60;
/// The height of hop's small confirm/prompt dialogs, in rows.
const DIALOG_HEIGHT: u16 = 7;

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
    pub fn render(&self, frame: &mut Frame, area: Rect, skin: &Skin) {
        let colors = Colors::from_palette(&skin.palette);
        let body = vec![
            Line::from(Span::styled(
                self.message.clone(),
                Style::default().fg(colors.danger),
            )),
            Line::raw(""),
            Line::from(vec![
                Span::styled("y", Style::default().fg(colors.accent)),
                Span::styled(" confirm   ", Style::default().fg(colors.dim)),
                Span::styled("n", Style::default().fg(colors.accent)),
                Span::styled(" cancel", Style::default().fg(colors.dim)),
            ]),
        ];
        render_modal(frame, area, dialog_box(skin, &self.title), |_| body);
    }
}

/// The shared geometry of hop's small confirm/prompt dialogs.
fn dialog_box<'a>(skin: &'a Skin, title: &'a str) -> ModalBox<'a> {
    ModalBox {
        skin,
        title,
        width_percent: DIALOG_WIDTH_PCT,
        height: DIALOG_HEIGHT,
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

    /// Renders the prompt centred in `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect, skin: &Skin) {
        let colors = Colors::from_palette(&skin.palette);
        let label = format!("{}: ", self.label);
        let body = |inner_width: usize| {
            let mut field = vec![Span::styled(
                label.clone(),
                Style::default().fg(colors.dim),
            )];
            field.extend(field_spans(FieldView {
                field: &self.input,
                palette: &skin.palette,
                width: inner_width.saturating_sub(label.chars().count()),
                focused: true,
            }));
            vec![
                Line::from(field),
                Line::raw(""),
                Line::from(Span::styled(
                    "Enter save · Esc cancel",
                    Style::default().fg(colors.dim),
                )),
            ]
        };
        render_modal(frame, area, dialog_box(skin, &self.title), body);
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
            KeyCode::Enter => SelectResult::Selected(self.cursor),
            KeyCode::Esc => SelectResult::Cancel,
            _ if self.navigate(key) => SelectResult::Pending,
            _ => SelectResult::Pending,
        }
    }

    /// Applies a navigation key through the shared helper, reporting whether it
    /// was one.
    fn navigate(&mut self, key: KeyEvent) -> bool {
        match moved_cursor(key, self.cursor, self.items.len()) {
            Some(cursor) => {
                self.cursor = cursor;
                true
            }
            None => false,
        }
    }

    /// Renders the list centred in `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect, skin: &Skin) {
        let colors = Colors::from_palette(&skin.palette);
        let height = (self.items.len() as u16 + 2).clamp(3, 16);
        let rect = centered_rect(50, height, area);
        frame.render_widget(Clear, rect);
        let block = ratada::chrome::modal_block(skin, &self.title);
        let items: Vec<ListItem> = self
            .items
            .iter()
            .map(|item| ListItem::new(item.clone()))
            .collect();
        let list = List::new(items)
            .block(block)
            .highlight_style(colors.selection_style());
        let mut state = ListState::default();
        state.select(Some(self.cursor));
        frame.render_stateful_widget(list, rect, &mut state);
        // The area inside the border, where the rows are drawn.
        let inner = Rect {
            x: rect.x + 1,
            y: rect.y + 1,
            width: rect.width.saturating_sub(2),
            height: rect.height.saturating_sub(2),
        };
        // Rendering settled the scroll offset; show a scrollbar if it overflows.
        ratada::scroll::render_scrollbar(
            frame,
            inner,
            skin,
            ratada::nav::ScrollView {
                total: self.items.len(),
                offset: state.offset(),
                viewport: inner.height as usize,
            },
        );
    }
}

/// Centres a `width` by `height` box, in cells, within `area`. Both are capped
/// at what `area` can give, so an oversized box shrinks instead of overflowing.
pub fn centered_box(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// Computes a centred rect of `width_percent` percent and `height` rows within
/// `area`. Percentages above 100 are treated as the full width.
pub fn centered_rect(width_percent: u16, height: u16, area: Rect) -> Rect {
    // u32 so a wide terminal times a large percentage cannot overflow u16.
    let width = u32::from(area.width) * u32::from(width_percent.min(100)) / 100;
    centered_box(width as u16, height, area)
}

/// The chrome of a centred modal: what it is titled and how big it is.
struct ModalBox<'a> {
    /// The active theme.
    skin: &'a Skin,
    /// The title drawn into the top border.
    title: &'a str,
    /// The box width, as a percentage of the area.
    width_percent: u16,
    /// The box height, in rows.
    height: u16,
}

/// Renders a rounded, accent-titled modal (clibase `modal_block`) over a lifted
/// fill, centred in `area`. `body` is built from the box's inner width, so a
/// text field inside can scroll its value to fit.
fn render_modal(
    frame: &mut Frame,
    area: Rect,
    modal: ModalBox<'_>,
    body: impl FnOnce(usize) -> Vec<Line<'static>>,
) {
    let rect = centered_rect(modal.width_percent, modal.height, area);
    frame.render_widget(Clear, rect);
    let block = ratada::chrome::modal_block(modal.skin, modal.title);
    let inner_width = block.inner(rect).width as usize;
    frame.render_widget(Paragraph::new(body(inner_width)).block(block), rect);
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    #[test]
    fn centered_box_caps_an_oversized_box_at_the_area() {
        let area = Rect::new(0, 0, 40, 10);
        let rect = centered_box(200, 99, area);
        assert_eq!(rect, area, "an oversized box must shrink, not overflow");
    }

    #[test]
    fn centered_box_centres_a_fitting_box() {
        let rect = centered_box(20, 4, Rect::new(0, 0, 40, 10));
        assert_eq!(rect, Rect::new(10, 3, 20, 4));
    }

    #[test]
    fn centered_rect_survives_a_very_wide_area() {
        // `area.width * width_percent` must not overflow u16.
        let rect = centered_rect(100, 4, Rect::new(0, 0, 2000, 10));
        assert_eq!(rect.width, 2000);
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
