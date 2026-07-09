//! The typography the detail panel and the statistics columns share.
//!
//! A section header, a label/value field line, a log line and a share bar. They
//! live here rather than in `preview.rs` so the panel and the table agree on
//! how a number, a label and a heading look.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::tui::skin::Colors;

/// The bar glyph. Single-cell, like every other glyph hop draws.
const BAR_FULL: &str = "\u{2588}";
/// The unfilled remainder of a bar.
const BAR_EMPTY: &str = "\u{2591}";

/// A section heading: the uppercase title in the dimmed accent, then a rule to
/// the panel's right edge. Matches the help overlay's headings.
pub fn section_header(
    title: &str,
    width: usize,
    colors: &Colors,
) -> Line<'static> {
    let label = format!("{} ", title.to_uppercase());
    let used = UnicodeWidthStr::width(label.as_str());
    let rule = "\u{2500}".repeat(width.saturating_sub(used));
    Line::from(vec![
        Span::styled(
            label,
            Style::default()
                .fg(colors.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(rule, Style::default().fg(colors.dim)),
    ])
}

/// A field line: a dim label padded to `label_width`, then the value in the
/// normal foreground. Two colours, so the eye finds the values.
pub fn field(
    label: &str,
    value: &str,
    label_width: usize,
    colors: &Colors,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{label:<label_width$}"),
            Style::default().fg(colors.dim),
        ),
        Span::styled(value.to_string(), Style::default().fg(colors.foreground)),
    ])
}

/// A `git log --oneline` entry: the hash dimmed, the subject in normal text.
pub fn log_line(entry: &str, colors: &Colors) -> Line<'static> {
    match entry.split_once(' ') {
        Some((hash, subject)) => Line::from(vec![
            Span::styled(format!("{hash} "), Style::default().fg(colors.dim)),
            Span::styled(
                subject.to_string(),
                Style::default().fg(colors.foreground),
            ),
        ]),
        None => Line::from(Span::styled(
            entry.to_string(),
            Style::default().fg(colors.foreground),
        )),
    }
}

/// A horizontal bar `width` cells wide, filled to `share` of its length.
/// `share` is clamped to `0.0..=1.0`; a non-zero share always shows at least
/// one filled cell, so a small language does not vanish.
pub fn bar(share: f32, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let share = share.clamp(0.0, 1.0);
    let mut filled = (share * width as f32).round() as usize;
    if share > 0.0 {
        filled = filled.max(1);
    }
    let filled = filled.min(width);
    BAR_FULL.repeat(filled) + &BAR_EMPTY.repeat(width - filled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn colors() -> Colors {
        Colors::from_palette(&Config::default().palette())
    }

    #[test]
    fn bar_fills_proportionally_and_never_overflows() {
        assert_eq!(bar(0.0, 4), "░░░░");
        assert_eq!(bar(0.5, 4), "██░░");
        assert_eq!(bar(1.0, 4), "████");
        // Out-of-range shares are clamped, not wrapped.
        assert_eq!(bar(1.5, 4), "████");
        assert_eq!(bar(-1.0, 4), "░░░░");
        assert_eq!(bar(0.5, 0), "");
    }

    #[test]
    fn bar_shows_a_tiny_share_as_one_cell() {
        // 1 % of 20 rounds to zero cells; the language would disappear.
        assert_eq!(bar(0.01, 20).chars().filter(|c| *c == '█').count(), 1);
    }

    #[test]
    fn field_pads_the_label_column() {
        let line = field("Kind", "git", 10, &colors());
        assert_eq!(line.spans[0].content.as_ref(), "Kind      ");
        assert_eq!(line.spans[1].content.as_ref(), "git");
    }

    #[test]
    fn section_header_uppercases_and_rules_to_the_edge() {
        let line = section_header("git", 20, &colors());
        assert_eq!(line.spans[0].content.as_ref(), "GIT ");
        assert_eq!(line.width(), 20);
    }

    #[test]
    fn log_line_dims_the_hash_only() {
        let line = log_line("a1b2c3d feat: add stats", &colors());
        assert_eq!(line.spans.len(), 2);
        assert_eq!(line.spans[0].content.as_ref(), "a1b2c3d ");
        // A line without a space is left whole rather than mangled.
        assert_eq!(log_line("nospace", &colors()).spans.len(), 1);
    }
}
