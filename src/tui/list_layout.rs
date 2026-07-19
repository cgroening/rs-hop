//! Layout maths shared by the two list renderers: the scroll offset and the
//! width of the optional Slug column.
//!
//! Both [`table`](crate::tui::table) and
//! [`sections_view`](crate::tui::sections_view) scroll the same way and size the
//! Slug column the same way. Keeping the maths here means the two cannot drift
//! apart - which they already had, the sectioned view having grown an extra
//! snap-to-top rule the flat table never got.

use crossterm::event::{KeyCode, KeyEvent};
use ratada::nav::{cycle, step_clamped};
use unicode_width::UnicodeWidthStr;

/// The header label of the Slug column, which is also its floor width.
pub const SLUG_HEADER: &str = "Slug";
/// The upper bound of the Slug column, so a long slug cannot dominate the row.
pub const SLUG_MAX: usize = 20;

/// Where the cursor sits in a list and how much of it is on screen.
///
/// Grouping these five values is what makes [`settled_offset`] readable: a call
/// like `settled_offset(0, 1, 1, 10, 5)` says nothing at the call site.
pub struct ListScroll {
    /// The offset carried over from the previous frame.
    pub saved: usize,
    /// The cursor's row index (in rows, so headers count in a grouped list).
    pub cursor: usize,
    /// How many rows the list has in total.
    pub row_count: usize,
    /// How many rows are visible at once.
    pub viewport: usize,
    /// Row of the first entry in a grouped list, whose leading section header
    /// should never be the only thing scrolled away. `None` for a flat list.
    pub first_entry_row: Option<usize>,
}

/// Predicts the scroll offset: keeps the saved one unless the cursor fell off an
/// edge of the viewport, then clamps it back just far enough to reveal it.
///
/// In a grouped list an offset that would hide nothing but the leading section
/// header snaps to the very top instead.
pub fn settled_offset(scroll: &ListScroll) -> usize {
    let mut offset = scroll.saved.min(scroll.row_count.saturating_sub(1));
    if scroll.cursor < offset {
        offset = scroll.cursor;
    } else if scroll.viewport > 0 && scroll.cursor >= offset + scroll.viewport {
        offset = scroll.cursor + 1 - scroll.viewport;
    }
    if let Some(first_entry_row) = scroll.first_entry_row
        && offset <= first_entry_row
    {
        offset = 0;
    }
    offset
}

/// The Slug column width for a widest slug of `widest` cells: floored at the
/// header label and capped at [`SLUG_MAX`].
pub fn slug_column_width(widest: usize) -> usize {
    widest
        .max(UnicodeWidthStr::width(SLUG_HEADER))
        .min(SLUG_MAX)
}

/// How many rows an overlay list moves by default when paging, where the real
/// viewport height is not tracked.
const OVERLAY_PAGE: isize = 10;

/// Where a navigation key moves the cursor of a list of `len` rows, or `None`
/// when the key is not a navigation key.
///
/// Arrows wrap, page jumps and `Home`/`End` clamp - the split §1.7 prescribes.
/// Every overlay list routes through this, so none of them can end up
/// supporting a different subset of the movement keys.
pub fn moved_cursor(key: KeyEvent, cursor: usize, len: usize) -> Option<usize> {
    match key.code {
        KeyCode::Up => Some(cycle(cursor, len, -1)),
        KeyCode::Down => Some(cycle(cursor, len, 1)),
        KeyCode::PageUp => Some(step_clamped(cursor, len, -OVERLAY_PAGE)),
        KeyCode::PageDown => Some(step_clamped(cursor, len, OVERLAY_PAGE)),
        KeyCode::Home => Some(0),
        KeyCode::End => Some(len.saturating_sub(1)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat-list scroll state, for the cases where grouping plays no role.
    fn flat(saved: usize, cursor: usize, viewport: usize) -> ListScroll {
        ListScroll {
            saved,
            cursor,
            row_count: 20,
            viewport,
            first_entry_row: None,
        }
    }

    #[test]
    fn the_offset_stays_put_while_the_cursor_is_still_visible() {
        // Viewport of 5 showing rows 4..9; the cursor moves up from 8 to 5. It
        // stays visible, so the offset must not move (the eager-scroll bug).
        assert_eq!(settled_offset(&flat(4, 5, 5)), 4);
        assert_eq!(settled_offset(&flat(4, 4, 5)), 4);
    }

    #[test]
    fn the_list_scrolls_up_only_once_the_cursor_crosses_the_top_edge() {
        assert_eq!(settled_offset(&flat(4, 3, 5)), 3);
    }

    #[test]
    fn the_list_pages_down_at_the_bottom_edge() {
        // Cursor past the bottom row scrolls down just enough to reveal it.
        assert_eq!(settled_offset(&flat(4, 9, 5)), 5);
    }

    #[test]
    fn a_grouped_list_snaps_to_the_top_rather_than_hiding_only_a_header() {
        let scroll = ListScroll {
            saved: 1,
            cursor: 1,
            row_count: 10,
            viewport: 5,
            first_entry_row: Some(2),
        };
        assert_eq!(settled_offset(&scroll), 0);
    }

    #[test]
    fn a_flat_list_never_snaps_to_the_top() {
        // The same numbers without a first entry row keep the offset - this is
        // the one behaviour that separates the two renderers.
        let scroll = ListScroll {
            saved: 1,
            cursor: 1,
            row_count: 10,
            viewport: 5,
            first_entry_row: None,
        };
        assert_eq!(settled_offset(&scroll), 1);
    }

    /// A bare key press, as the overlays receive it.
    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    #[test]
    fn the_arrows_wrap_at_both_ends() {
        assert_eq!(moved_cursor(press(KeyCode::Up), 0, 5), Some(4));
        assert_eq!(moved_cursor(press(KeyCode::Down), 4, 5), Some(0));
    }

    #[test]
    fn page_jumps_and_home_end_clamp_rather_than_wrapping() {
        // §1.7: arrows are cyclic, page jumps and Home/End are not.
        assert_eq!(moved_cursor(press(KeyCode::PageUp), 2, 100), Some(0));
        assert_eq!(moved_cursor(press(KeyCode::PageDown), 98, 100), Some(99));
        assert_eq!(moved_cursor(press(KeyCode::Home), 40, 100), Some(0));
        assert_eq!(moved_cursor(press(KeyCode::End), 40, 100), Some(99));
    }

    #[test]
    fn an_empty_list_stays_at_index_zero() {
        for code in [
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Home,
            KeyCode::End,
        ] {
            assert_eq!(moved_cursor(press(code), 0, 0), Some(0));
        }
    }

    #[test]
    fn a_non_navigation_key_is_left_to_the_caller() {
        assert_eq!(moved_cursor(press(KeyCode::Char('x')), 1, 5), None);
        assert_eq!(moved_cursor(press(KeyCode::Enter), 1, 5), None);
        assert_eq!(moved_cursor(press(KeyCode::Esc), 1, 5), None);
    }

    #[test]
    fn the_slug_column_is_floored_at_the_header_and_capped() {
        assert_eq!(slug_column_width(0), SLUG_HEADER.len());
        assert_eq!(slug_column_width(2), SLUG_HEADER.len());
        assert_eq!(slug_column_width(12), 12);
        assert_eq!(slug_column_width(99), SLUG_MAX);
    }
}
