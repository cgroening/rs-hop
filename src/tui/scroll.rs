//! The scroll position shared by the scrollable overlays and panels.
//!
//! A [`Scroll`] remembers where a taller-than-the-screen body is scrolled to,
//! plus the largest offset the last render allowed. Key handlers can therefore
//! clamp without knowing the viewport height: rendering calls [`Scroll::fit`],
//! which records the range and returns the offset to draw at.
//!
//! Interior mutability ([`Cell`]) keeps `render` taking `&self`, as the widgets
//! do.

use std::cell::Cell;

use crossterm::event::{KeyCode, KeyEvent};

/// Rows a page key moves.
const PAGE_ROWS: i32 = 10;

/// A scroll offset, clamped to the range the last render reported.
#[derive(Debug, Default)]
pub struct Scroll {
    offset: Cell<u16>,
    max: Cell<u16>,
}

impl Scroll {
    /// Scrolls back to the top, when the overlay opens.
    pub fn reset(&self) {
        self.offset.set(0);
    }

    /// The offset the body should be drawn at.
    pub fn offset(&self) -> u16 {
        self.offset.get()
    }

    /// Moves the view by `delta` rows, clamped to the scrollable range.
    /// `i32::MIN` and `i32::MAX` jump to the top and the bottom.
    pub fn scroll_by(&self, delta: i32) {
        let max = i32::from(self.max.get());
        let moved = i32::from(self.offset.get()).saturating_add(delta);
        self.offset.set(moved.clamp(0, max) as u16);
    }

    /// Moves the view for `key`. Keys that do not scroll leave it alone.
    pub fn handle_key(&self, key: KeyEvent) {
        let delta = match key.code {
            KeyCode::Up | KeyCode::Char('k') => -1,
            KeyCode::Down | KeyCode::Char('j') => 1,
            KeyCode::PageUp | KeyCode::Char('u') => -PAGE_ROWS,
            KeyCode::PageDown | KeyCode::Char('d') => PAGE_ROWS,
            KeyCode::Home | KeyCode::Char('g') => i32::MIN,
            KeyCode::End | KeyCode::Char('G') => i32::MAX,
            _ => return,
        };
        self.scroll_by(delta);
    }

    /// Records the scrollable range for a `total`-row body seen through a
    /// `viewport`-row window, and returns the clamped offset to render at.
    /// Called from `render`, so the key handler can clamp on the next press.
    pub fn fit(&self, total: u16, viewport: u16) -> u16 {
        self.max.set(total.saturating_sub(viewport));
        let offset = self.offset.get().min(self.max.get());
        self.offset.set(offset);
        offset
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyModifiers;

    use super::*;

    #[test]
    fn fit_clamps_the_offset_to_the_new_range() {
        let scroll = Scroll::default();
        scroll.scroll_by(i32::MAX);
        // Nothing rendered yet, so nothing scrolls.
        assert_eq!(scroll.offset(), 0);
        // 30 rows through a 10-row window scroll by at most 20.
        assert_eq!(scroll.fit(30, 10), 0);
        scroll.scroll_by(i32::MAX);
        assert_eq!(scroll.offset(), 20);
        // A taller window shrinks the range and pulls the offset back.
        assert_eq!(scroll.fit(30, 25), 5);
    }

    #[test]
    fn a_body_that_fits_never_scrolls() {
        let scroll = Scroll::default();
        scroll.fit(5, 20);
        scroll.scroll_by(3);
        assert_eq!(scroll.offset(), 0);
    }

    #[test]
    fn handle_key_maps_the_movement_keys() {
        let scroll = Scroll::default();
        scroll.fit(100, 10);
        let press = |code| KeyEvent::new(code, KeyModifiers::NONE);
        scroll.handle_key(press(KeyCode::Down));
        assert_eq!(scroll.offset(), 1);
        scroll.handle_key(press(KeyCode::Char('j')));
        assert_eq!(scroll.offset(), 2);
        scroll.handle_key(press(KeyCode::End));
        assert_eq!(scroll.offset(), 90);
        scroll.handle_key(press(KeyCode::Home));
        assert_eq!(scroll.offset(), 0);
        scroll.handle_key(press(KeyCode::Char('x')));
        assert_eq!(scroll.offset(), 0);
    }
}
