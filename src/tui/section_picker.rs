//! A fuzzy-searchable picker for choosing (or creating) an entry's section.
//!
//! It lists `Ungrouped` plus the current kind's sections, filters them as the
//! user types (fuzzy, via `ratada::finder::filter`), and — when the typed name
//! matches no existing section — offers a "create" row so a new section can be
//! made inline (registered on save via `RepoService::ensure_section`). Styled
//! after hop's own non-blocking overlays like [`crate::tui::path_picker`].

use std::cell::Cell;

use crossterm::event::{KeyCode, KeyEvent};
use ratada::input::InputField;
use ratada::nav::cycle;
use ratada::text::truncate;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::domain::sections::UNGROUPED;
use crate::theme::Skin;
use crate::tui::presentation::{FieldView, field_spans};
use crate::tui::skin::Colors;
use crate::tui::widgets::centered_rect;

/// The label in front of the picker's filter line.
const FILTER_LABEL: &str = "filter: ";

/// The picker's footer hints.
const FOOTER_HINTS: &[(&str, &str)] = &[
    ("\u{2191}\u{2193}", "move"),
    ("type", "filter / new"),
    ("enter", "choose"),
];

/// The box width as a percentage of the area.
const WIDTH_PCT: u16 = 60;

/// Outcome of feeding a key to the section picker.
pub enum SectionPickResult {
    /// Still choosing.
    Pending,
    /// The user picked a section (`None` = Ungrouped, `Some(name)` = a section
    /// name, which may be newly typed).
    Picked(Option<String>),
    /// The user cancelled.
    Cancel,
}

/// Fuzzy section-picker state. `items[0]` is always the Ungrouped label; the
/// rest are the section names in order.
pub struct SectionPicker {
    items: Vec<String>,
    filter: InputField,
    /// Indices into `items` matching the filter, best match first.
    visible: Vec<usize>,
    /// Cursor over the combined list (the visible items, then the create row
    /// when one is offered).
    cursor: usize,
    /// The list scroll offset, carried across frames by `ratada::list`.
    offset: Cell<usize>,
}

impl SectionPicker {
    /// Builds a picker over `sections` (plus Ungrouped), with the cursor on the
    /// currently chosen section (`None` = Ungrouped).
    pub fn new(sections: &[String], current: Option<&str>) -> Self {
        let mut items = vec![UNGROUPED.to_string()];
        items.extend(sections.iter().cloned());
        let cursor = current
            .and_then(|name| {
                items.iter().position(|i| i.eq_ignore_ascii_case(name))
            })
            .unwrap_or(0);
        let mut picker = SectionPicker {
            items,
            filter: InputField::new(""),
            visible: Vec::new(),
            cursor,
            offset: Cell::new(0),
        };
        picker.apply_filter();
        picker.cursor = cursor.min(picker.row_count().saturating_sub(1));
        picker
    }

    /// Handles a key: navigate, choose, cancel, or edit the filter.
    pub fn handle_key(&mut self, key: KeyEvent) -> SectionPickResult {
        match key.code {
            KeyCode::Esc => return SectionPickResult::Cancel,
            KeyCode::Up => self.move_cursor(-1),
            KeyCode::Down => self.move_cursor(1),
            KeyCode::Enter => return self.choose(),
            _ => {
                if self.filter.handle_key(key) {
                    self.apply_filter();
                }
            }
        }
        SectionPickResult::Pending
    }

    /// Inserts a bracketed paste into the filter and re-runs the match.
    pub fn paste(&mut self, text: &str) {
        self.filter.paste(text);
        self.apply_filter();
    }

    /// Chooses the highlighted row: an existing item, or the create row when the
    /// cursor sits past the visible items.
    fn choose(&self) -> SectionPickResult {
        if self.cursor < self.visible.len() {
            let index = self.visible[self.cursor];
            let value = (index != 0).then(|| self.items[index].clone());
            return SectionPickResult::Picked(value);
        }
        match self.create_label() {
            Some(name) => SectionPickResult::Picked(Some(name)),
            None => SectionPickResult::Pending,
        }
    }

    /// The name the create row would add, or `None` when the query is blank or
    /// already names an existing item (case-insensitive).
    fn create_label(&self) -> Option<String> {
        let query = self.filter.value().trim();
        if query.is_empty() {
            return None;
        }
        let exists = self
            .items
            .iter()
            .any(|item| item.eq_ignore_ascii_case(query));
        (!exists).then(|| query.to_string())
    }

    /// The number of selectable rows: the visible items plus the create row.
    fn row_count(&self) -> usize {
        self.visible.len() + usize::from(self.create_label().is_some())
    }

    /// Moves the cursor cyclically within the selectable rows.
    fn move_cursor(&mut self, delta: isize) {
        self.cursor = cycle(self.cursor, self.row_count(), delta);
    }

    /// Recomputes the visible indices from the filter text and clamps the cursor.
    fn apply_filter(&mut self) {
        self.visible = ratada::finder::filter(&self.items, self.filter.value());
        let rows = self.row_count();
        if self.cursor >= rows {
            self.cursor = rows.saturating_sub(1);
        }
        self.offset.set(0);
    }

    /// Renders the picker centred in `area`, sized to its content (never taller
    /// than needed): a titled box with the caret filter, the list (create row
    /// last) and a compact footer.
    pub fn render(&self, frame: &mut Frame, area: Rect, skin: &Skin) {
        // Footer height depends on the box width, which the box height does not,
        // so it can be computed up front from the intended inner width.
        let inner_width = ((u32::from(area.width) * u32::from(WIDTH_PCT) / 100)
            as usize)
            .saturating_sub(2);
        let footer = ratada::shortcut_hints::lines(
            FOOTER_HINTS,
            skin.palette.accent_dim,
            inner_width,
        );
        let footer_h = (footer.len() as u16).max(1);
        // At most one create row can join the full item list.
        let list_h = (self.items.len() as u16 + 1).max(1);
        // Borders (2) + filter (1) + list + footer; capped to the screen.
        let height = 2 + 1 + list_h + footer_h;
        let rect = centered_rect(WIDTH_PCT, height, area);

        frame.render_widget(Clear, rect);
        let block = ratada::chrome::modal_block(skin, "Choose section");
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        let badge =
            ratada::chrome::position_badge(self.cursor, self.row_count());
        ratada::chrome::render_badge(frame, rect, skin, &badge);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(footer_h),
            ])
            .split(inner);

        self.render_filter(frame, rows[0], skin);
        self.render_list(frame, rows[1], skin);
        frame.render_widget(Paragraph::new(footer), rows[2]);
    }

    /// Renders the filter line (block caret, scrolling with `…`).
    fn render_filter(&self, frame: &mut Frame, area: Rect, skin: &Skin) {
        let colors = Colors::from_palette(&skin.palette);
        let mut spans =
            vec![Span::styled(FILTER_LABEL, Style::default().fg(colors.dim))];
        spans.extend(field_spans(FieldView {
            field: &self.filter,
            palette: &skin.palette,
            width: (area.width as usize)
                .saturating_sub(FILTER_LABEL.chars().count()),
            focused: true,
        }));
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// Renders the list via `ratada::list`: the visible items, then the create
    /// row (accented) when a new name would be added.
    fn render_list(&self, frame: &mut Frame, area: Rect, skin: &Skin) {
        let accent = Colors::from_palette(&skin.palette).accent;
        let width = area.width as usize;
        let mut lines: Vec<Line<'static>> = self
            .visible
            .iter()
            .map(|&index| Line::from(truncate(&self.items[index], width)))
            .collect();
        if let Some(name) = self.create_label() {
            lines.push(
                Line::from(truncate(&format!("+ Create \"{name}\""), width))
                    .style(Style::default().fg(accent)),
            );
        }
        ratada::list::render(
            frame,
            area,
            skin,
            ratada::list::ListView {
                rows: lines,
                selected: self.cursor,
                offset: &self.offset,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    fn key(ch: char) -> KeyEvent {
        KeyEvent::from(KeyCode::Char(ch))
    }

    fn sections() -> Vec<String> {
        vec!["Backend".to_string(), "Frontend".to_string()]
    }

    #[test]
    fn lists_ungrouped_plus_sections_when_empty() {
        let picker = SectionPicker::new(&sections(), None);
        // Ungrouped + two sections, no create row for an empty query.
        assert_eq!(picker.visible.len(), 3);
        assert_eq!(picker.row_count(), 3);
        assert!(picker.create_label().is_none());
    }

    #[test]
    fn picks_ungrouped_at_the_top() {
        let picker = SectionPicker::new(&sections(), None);
        assert!(matches!(picker.choose(), SectionPickResult::Picked(None)));
    }

    #[test]
    fn filters_and_picks_an_existing_section() {
        let mut picker = SectionPicker::new(&sections(), None);
        picker.handle_key(key('f'));
        // "f" narrows to Frontend, on which the cursor rests; Enter picks it
        // (a create row for the partial "f" sits below, unselected).
        assert_eq!(picker.visible.len(), 1);
        assert!(matches!(
            picker.choose(),
            SectionPickResult::Picked(Some(name)) if name == "Frontend"
        ));
    }

    #[test]
    fn a_pasted_query_narrows_the_list_like_typing() {
        let mut picker = SectionPicker::new(&sections(), None);
        picker.paste("Frontend");
        assert_eq!(picker.visible.len(), 1);
        assert!(matches!(
            picker.choose(),
            SectionPickResult::Picked(Some(name)) if name == "Frontend"
        ));
    }

    #[test]
    fn an_exact_existing_name_offers_no_create_row() {
        let mut picker = SectionPicker::new(&sections(), None);
        for ch in "Backend".chars() {
            picker.handle_key(key(ch));
        }
        assert!(picker.create_label().is_none());
    }

    #[test]
    fn creates_a_new_section_when_nothing_matches() {
        let mut picker = SectionPicker::new(&sections(), None);
        for ch in "Ops".chars() {
            picker.handle_key(key(ch));
        }
        assert_eq!(picker.create_label().as_deref(), Some("Ops"));
        // The create row is the only selectable row; Enter creates it.
        assert!(matches!(
            picker.choose(),
            SectionPickResult::Picked(Some(name)) if name == "Ops"
        ));
    }
}
