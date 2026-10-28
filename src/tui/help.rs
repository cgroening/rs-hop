//! The help overlay listing all keyboard shortcuts, opened with `?`.
//!
//! The overlay itself is `ratada::help`, the same one mdtask shows: a search
//! line with a caret at the top, fuzzy-filtered sections with highlighted
//! matches, a cursor row, `Tab`/`BackTab` section jumps and a position badge.
//! hop drives it through the toolkit's non-blocking entry points
//! ([`ratada::help::render`] and [`ratada::help::handle_key`]) rather than the
//! blocking `show`, so a background refresh keeps running behind it (see
//! `docs/DEVELOPMENT.md`, "Architecture decisions").
//!
//! No key is spelled out here. Every section arrives already resolved against
//! the keymap (see [`bindings::HELP_SECTIONS`](crate::tui::bindings)), so a
//! `[keys]` override moves the key in the overlay as well.

use ratada::help::{HelpSection, HelpState};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::theme::Skin;

/// An owned `(label, hints)` section, resolved against the keymap by the app.
pub type Section = (String, Vec<(String, String)>);

/// Renders the help overlay for `sections` centred in `area`.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    skin: &Skin,
    sections: &[Section],
    state: &HelpState,
) {
    ratada::help::render(frame, area, skin, &as_help_sections(sections), state);
}

/// Borrows hop's owned sections in the shape `ratada::help` takes.
pub fn as_help_sections(sections: &[Section]) -> Vec<HelpSection<'_, String>> {
    sections
        .iter()
        .map(|(title, bindings)| HelpSection { title, bindings })
        .collect()
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
                "Global".to_string(),
                vec![("q".to_string(), "quit".to_string())],
            ),
        ]
    }

    /// The typed query is drawn on the search line, so the filter is visible
    /// while it narrows the list - the bug this module was rewritten for.
    #[test]
    fn the_typed_query_is_visible_in_the_overlay() {
        let skin = Config::default().skin();
        let sections = sample_sections();
        let mut state = HelpState::new();
        state.paste("quit");
        let mut terminal = Terminal::new(TestBackend::new(80, 24))
            .expect("the test backend never fails");
        terminal
            .draw(|frame| {
                render(frame, frame.area(), &skin, &sections, &state);
            })
            .expect("the test backend never fails");
        let buffer = terminal.backend().buffer();
        let screen: String = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            screen.lines().any(|line| line.contains("search quit")),
            "no search line with the query in:\n{screen}"
        );
    }

    #[test]
    fn the_conversion_keeps_titles_and_bindings_in_order() {
        let sections = sample_sections();
        let converted = as_help_sections(&sections);
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0].title, "Navigation");
        assert_eq!(converted[1].bindings[0].0, "q");
    }
}
