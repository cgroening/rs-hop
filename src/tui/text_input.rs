//! A single-line text input with a block caret.
//!
//! It is the one editing primitive hop needs (the live filter, the slug prompt
//! and the form's text fields are all single-line), so all of them share this
//! logic rather than re-implementing key handling per field.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// Editable single-line text with a caret position (a char index, `0..=len`).
#[derive(Debug, Clone, Default)]
pub struct TextInput {
    chars: Vec<char>,
    cursor: usize,
}

impl TextInput {
    /// Creates an input seeded with `initial`, caret at the end.
    pub fn new(initial: &str) -> Self {
        let chars: Vec<char> = initial.chars().collect();
        let cursor = chars.len();
        TextInput { chars, cursor }
    }

    /// The current text.
    pub fn value(&self) -> String {
        self.chars.iter().collect()
    }

    /// Whether the input is empty.
    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// Handles an editing key, returning whether it was consumed. Steering keys
    /// (Enter, Esc, Tab) are left for the caller.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('u') if ctrl => {
                self.chars.clear();
                self.cursor = 0;
            }
            KeyCode::Char(c) if !ctrl => self.insert(c),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => {
                self.cursor = (self.cursor + 1).min(self.chars.len());
            }
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.chars.len(),
            _ => return false,
        }
        true
    }

    /// Inserts `c` at the caret.
    fn insert(&mut self, c: char) {
        self.chars.insert(self.cursor, c);
        self.cursor += 1;
    }

    /// Removes the character before the caret.
    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.chars.remove(self.cursor);
        }
    }

    /// Removes the character under the caret.
    fn delete(&mut self) {
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
        }
    }

    /// Renders the value as a line, styling the text with `base` and the block
    /// caret with `cursor`. The caret is shown only when `show_cursor` is set
    /// (i.e. the field is focused); otherwise the plain value is rendered.
    pub fn render_line(
        &self,
        base: Style,
        cursor: Color,
        show_cursor: bool,
    ) -> Line<'static> {
        if !show_cursor {
            return Line::from(Span::styled(self.value(), base));
        }
        let cursor_style = Style::default().fg(Color::Black).bg(cursor);
        let mut spans: Vec<Span> = Vec::new();
        for (index, ch) in self.chars.iter().enumerate() {
            let style = if index == self.cursor {
                cursor_style
            } else {
                base
            };
            spans.push(Span::styled(ch.to_string(), style));
        }
        if self.cursor >= self.chars.len() {
            spans.push(Span::styled("\u{2588}", Style::default().fg(cursor)));
        }
        Line::from(spans)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn inserts_and_reports_value() {
        let mut input = TextInput::new("");
        for c in "hop".chars() {
            input.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(input.value(), "hop");
    }

    #[test]
    fn backspace_removes_before_caret() {
        let mut input = TextInput::new("abc");
        input.handle_key(key(KeyCode::Backspace));
        assert_eq!(input.value(), "ab");
    }

    #[test]
    fn caret_moves_and_inserts_in_middle() {
        let mut input = TextInput::new("ac");
        input.handle_key(key(KeyCode::Left));
        input.handle_key(key(KeyCode::Char('b')));
        assert_eq!(input.value(), "abc");
    }

    #[test]
    fn ctrl_u_clears() {
        let mut input = TextInput::new("text");
        input.handle_key(KeyEvent::new(
            KeyCode::Char('u'),
            KeyModifiers::CONTROL,
        ));
        assert!(input.is_empty());
    }
}
