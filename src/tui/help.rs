//! The help overlay listing all keyboard shortcuts, opened with `?`.
//!
//! It looks like `ratada::help`'s overlay — uppercase section headers, an
//! aligned accent key column, dimmed descriptions and a hint footer — but stays
//! hop's own **non-blocking** overlay so a background refresh keeps running
//! behind it (see `docs/DEVELOPMENT.md`, "Architecture decisions") and adds the
//! fuzzy filter §1.7 asks for.
//!
//! No key is spelled out here. Every section arrives already resolved against
//! the keymap (see [`bindings::HELP_SECTIONS`](crate::tui::bindings)), so a
//! `[keys]` override moves the key in the overlay as well.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::theme::Skin;
use ratada::fuzzy;

use crate::tui::scroll::Scroll;
use crate::tui::widgets::centered_box;

/// Rows the block border, the blank spacer and the footer take, on top of the
/// section rows.
const CHROME_ROWS: u16 = 5;
/// The width the overlay wants for its longest description, before it has to
/// give in and take whatever the terminal offers.
const PREFERRED_WIDTH: u16 = 60;
/// Spaces between the key column and the description.
const KEY_GAP: usize = 2;
/// An owned `(label, hints)` section, resolved against the keymap by the app.
pub type Section = (String, Vec<(String, String)>);

/// Renders the help overlay centred in `area`.
///
/// `query` filters the rows fuzzily (an empty query shows everything) and
/// `scroll` carries the view offset across frames.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    skin: &Skin,
    view: &HelpView,
    scroll: &Scroll,
) {
    let palette = &skin.palette;
    let matching = filtered(view.sections, view.query);
    // Four fifths of the screen, but never below the width the long
    // descriptions need. `centered_box` caps both against `area`.
    let width = (area.width * 4 / 5).max(PREFERRED_WIDTH);
    let height = section_rows(&matching) + CHROME_ROWS;
    let rect = centered_box(width, height, area);
    frame.render_widget(Clear, rect);
    let block = ratada::chrome::modal_block(skin, "Keyboard shortcuts");
    let inner = block.inner(rect);

    let key_width = key_column_width(&matching);
    let header_style =
        ratada::style::fg(palette.accent_dim).add_modifier(Modifier::BOLD);
    let key_style =
        ratada::style::fg(palette.accent).add_modifier(Modifier::BOLD);

    let mut lines: Vec<Line> = Vec::new();
    for (title, hints) in &matching {
        lines
            .push(Line::from(Span::styled(title.to_uppercase(), header_style)));
        for (key, description) in hints {
            lines.push(Line::from(vec![
                Span::styled(format!("  {key:key_width$}"), key_style),
                Span::styled(
                    description.to_string(),
                    ratada::style::secondary(palette),
                ),
            ]));
        }
    }
    if matching.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  no shortcut matches \"{}\"", view.query),
            ratada::style::secondary(palette),
        )));
    }

    // The footer stays pinned; only the sections scroll.
    let viewport = inner.height.saturating_sub(2);
    let total = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let offset = scroll.fit(total, viewport);

    frame.render_widget(block, rect);
    let body = Rect {
        height: viewport,
        ..inner
    };
    frame.render_widget(Paragraph::new(lines).scroll((offset, 0)), body);
    ratada::scroll::render_scrollbar(
        frame,
        body,
        skin,
        ratada::nav::ScrollView {
            total: total as usize,
            offset: offset as usize,
            viewport: viewport as usize,
        },
    );

    let footer = Rect {
        y: inner.y + inner.height - 1,
        height: 1,
        ..inner
    };
    frame.render_widget(
        Paragraph::new(footer_hint(skin, view.query, footer.width as usize)),
        footer,
    );
}

/// What the overlay draws: the resolved sections and the active filter.
pub struct HelpView<'a> {
    /// Every section, in display order, keys already resolved.
    pub sections: &'a [Section],
    /// The typed filter; empty shows everything.
    pub query: &'a str,
}

/// The sections whose rows match `query`, with empty sections dropped.
fn filtered(sections: &[Section], query: &str) -> Vec<Section> {
    if query.trim().is_empty() {
        return sections.to_vec();
    }
    sections
        .iter()
        .filter_map(|(title, hints)| {
            let kept: Vec<(String, String)> = hints
                .iter()
                .filter(|(key, description)| is_match(key, description, query))
                .cloned()
                .collect();
            (!kept.is_empty()).then(|| (title.clone(), kept))
        })
        .collect()
}

/// Whether one row matches `query`, over its key and its description together.
fn is_match(key: &str, description: &str, query: &str) -> bool {
    fuzzy::score(&format!("{key} {description}"), query).is_some()
}

/// The rows the sections occupy: one header plus one row per hint.
fn section_rows(sections: &[Section]) -> u16 {
    let rows: usize = sections.iter().map(|(_, hints)| hints.len() + 1).sum();
    u16::try_from(rows.max(1)).unwrap_or(u16::MAX)
}

/// The width of the aligned key column, sized to the widest key.
fn key_column_width(sections: &[Section]) -> usize {
    sections
        .iter()
        .flat_map(|(_, hints)| hints.iter().map(|(key, _)| key.width()))
        .max()
        .unwrap_or(0)
        + KEY_GAP
}

/// The overlay's footer, in the shared hint style. It shows the typed filter
/// once there is one, so the narrowed list never looks like the whole list.
fn footer_hint(skin: &Skin, query: &str, width: usize) -> Line<'static> {
    let typed = format!("filter: {query}");
    let hints: Vec<(&str, &str)> = if query.is_empty() {
        vec![
            ("\u{2191}\u{2193}", "scroll"),
            ("type", "filter"),
            ("esc/?", "close"),
        ]
    } else {
        vec![
            ("\u{2191}\u{2193}", "scroll"),
            (typed.as_str(), "esc clears"),
        ]
    };
    ratada::shortcut_hints::lines(&hints, skin.palette.accent_dim, width)
        .into_iter()
        .next()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::config::Config;

    fn sample_sections() -> Vec<Section> {
        vec![
            (
                "Navigation".to_string(),
                vec![("space".to_string(), "toggle selection".to_string())],
            ),
            (
                "Open".to_string(),
                vec![("L".to_string(), "open the git tool".to_string())],
            ),
            (
                "Global".to_string(),
                vec![("q".to_string(), "quit".to_string())],
            ),
        ]
    }

    /// The overlay used to render only between 60 and 125 columns: below that
    /// its `clamp` had `min > max`, above it the width overflowed a percentage.
    #[test]
    fn renders_at_any_terminal_size() {
        let skin = Config::default().skin();
        let sections = sample_sections();
        let scroll = Scroll::default();
        for query in ["", "open"] {
            for width in [1, 20, 40, 59, 60, 100, 126, 200, 400] {
                for height in [1, 2, 3, 5, 40] {
                    let mut terminal =
                        Terminal::new(TestBackend::new(width, height))
                            .expect("the test backend never fails");
                    let view = HelpView {
                        sections: &sections,
                        query,
                    };
                    terminal
                        .draw(|frame| {
                            render(frame, frame.area(), &skin, &view, &scroll);
                        })
                        .unwrap_or_else(|error| {
                            panic!("{width}x{height} failed: {error}")
                        });
                }
            }
        }
    }

    #[test]
    fn an_empty_query_keeps_every_section() {
        let sections = sample_sections();
        assert_eq!(filtered(&sections, "").len(), 3);
        assert_eq!(filtered(&sections, "   ").len(), 3);
    }

    #[test]
    fn a_query_keeps_only_matching_rows_and_drops_empty_sections() {
        let sections = sample_sections();
        let matching = filtered(&sections, "quit");
        assert_eq!(matching.len(), 1, "only Global survives");
        assert_eq!(matching[0].0, "Global");
        assert_eq!(matching[0].1.len(), 1);
    }

    #[test]
    fn a_query_matches_the_key_as_well_as_the_description() {
        let sections = sample_sections();
        // "L" is a key, not a word in any description.
        let by_key = filtered(&sections, "L");
        assert!(by_key.iter().any(|(title, _)| title == "Open"));
    }

    #[test]
    fn a_query_matching_nothing_yields_no_sections() {
        let sections = sample_sections();
        assert!(filtered(&sections, "zzzzqqqq").is_empty());
    }
}
