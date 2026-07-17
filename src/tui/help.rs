//! The help overlay listing all keyboard shortcuts, opened with `?`.
//!
//! It looks like `ratada::help`'s overlay — uppercase section headers, an
//! aligned accent key column, dimmed descriptions and a hint footer — but stays
//! hop's own **non-blocking** overlay so a background refresh keeps running
//! behind it (see `docs/DEVELOPMENT.md`, "Architecture decisions"). The leading
//! `Navigation` and the trailing `Global` section are handed in by the caller
//! ([`HelpSections`]), from the same tokens the footer uses: both carry keys a
//! `[keys]` override can move, so they must not be spelled out here.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::theme::Skin;
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
/// The shortcut list, grouped into the same sections as the footer hint bar.
/// The `Navigation` and `Global` sections are handed in at render time so their
/// keys stay in sync with the keymap and the toolkit's own chords.
const SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Open",
        &[
            (
                "Enter",
                "jump only: write path and exit (folder cd, file -> parent)",
            ),
            (
                "L",
                "open: git -> tool \u{b7} folder -> cd \u{b7} text file -> \
                 editor \u{b7} else app",
            ),
            (
                "l",
                "git: open the tool (lazygit) as an overlay, then return here",
            ),
            (
                "o",
                "jump only: write path and exit (folder cd, file -> parent)",
            ),
            (
                "O",
                "force open with the default app (e.g. a text file in its GUI \
                 app)",
            ),
            (
                "b",
                "open on GitHub in the browser (a tab per selected git repo, \
                 else the cursor)",
            ),
        ],
    ),
    (
        "Manage",
        &[
            ("n", "add an entry (fill the form; ^O picks the path)"),
            (
                "e",
                "edit the entry (or bulk-edit when several are selected)",
            ),
            (
                "d / Del / Backspace",
                "delete (acts on the selection, else the cursor)",
            ),
            (
                "u",
                "undo the last change (delete / archive / favourite / edit)",
            ),
            ("*", "toggle favourite (selection or cursor)"),
            ("A", "archive / restore (selection or cursor)"),
            ("S", "set or change the slug"),
            (
                "y",
                "copy path to the clipboard (all selected, one per line, else \
                 the cursor)",
            ),
        ],
    ),
    (
        "Backup",
        &[
            (
                "z",
                "zip the selected/cursor repo or folder to the backup folder",
            ),
            (
                "Z",
                "zip every entry opted into backup (form 'Backup' toggle)",
            ),
        ],
    ),
    (
        "View",
        &[
            (
                "f",
                "live fuzzy filter (Esc clears; matches are highlighted)",
            ),
            ("F", "toggle showing only git repos with a status change"),
            ("c", "cycle the columns (Standard / Code / Activity)"),
            ("t", "pick the column to sort by (again: flip direction)"),
            (",", "toggle floating favourites to the top"),
            ("i", "toggle the slug column (dim, italic) after the name"),
        ],
    ),
    (
        "Panel",
        &[
            ("v", "show or hide the detail panel"),
            ("V", "move the panel: right / bottom"),
            ("Ctrl+\u{2191}\u{2193}", "scroll the panel"),
            ("Ctrl+\u{2190}\u{2192}", "make the panel smaller / bigger"),
        ],
    ),
    (
        "Git",
        &[
            (
                "r",
                "git tabs: reload status (R: + fetch) \u{b7} Files: check \
                 paths exist",
            ),
            ("x", "refresh selection/cursor   \u{b7}   X: with git fetch"),
        ],
    ),
    (
        "Sections",
        &[
            (
                ".",
                "toggle grouping into sections (off: flat, global sort)",
            ),
            ("s", "jump to a section (in the grouped view)"),
            ("M", "manage sections (add / rename / delete / move)"),
            (
                "Alt+\u{2191}\u{2193}",
                "reorder within a group (custom sort only)",
            ),
        ],
    ),
    (
        "Paths",
        &[
            ("p", "repair a missing path"),
            ("!", "list entries with path errors and fix them"),
        ],
    ),
];

/// An owned `(label, hints)` section, as the app builds the `Navigation` and
/// `Global` groups.
pub type Section = (String, Vec<(String, String)>);

/// The sections the app builds from the keymap, wrapping the static ones.
///
/// Both carry keys that a `[keys]` override can move, so they are resolved by
/// the caller rather than spelled out in [`SECTIONS`].
pub struct HelpSections<'a> {
    /// The leading section: cursor movement and the selection keys.
    pub navigation: &'a Section,
    /// The trailing section: the app-wide chords.
    pub global: &'a Section,
}

/// Renders the help overlay centred in `area`. `scroll` carries the view offset
/// across frames.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    skin: &Skin,
    groups: &HelpSections,
    scroll: &Scroll,
) {
    let palette = &skin.palette;
    // Four fifths of the screen, but never below the width the long
    // descriptions need. `centered_box` caps both against `area`.
    let width = (area.width * 4 / 5).max(PREFERRED_WIDTH);
    let height = section_rows(groups) + CHROME_ROWS;
    let rect = centered_box(width, height, area);
    frame.render_widget(Clear, rect);
    let block = ratada::chrome::modal_block(skin, "Keyboard shortcuts");
    let inner = block.inner(rect);

    let key_width = key_column_width(groups);
    let header_style =
        ratada::style::fg(palette.accent_dim).add_modifier(Modifier::BOLD);
    let key_style =
        ratada::style::fg(palette.accent).add_modifier(Modifier::BOLD);

    let mut lines: Vec<Line> = Vec::new();
    for (title, hints) in sections(groups) {
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
        Paragraph::new(footer_hint(skin, footer.width as usize)),
        footer,
    );
}

/// The caller's `Navigation` section, the static ones, then its `Global` one.
fn sections<'a>(
    groups: &'a HelpSections,
) -> impl Iterator<Item = (&'a str, Vec<(&'a str, &'a str)>)> {
    let listed = SECTIONS.iter().map(|(title, hints)| {
        (*title, hints.iter().map(|(k, d)| (*k, *d)).collect())
    });
    std::iter::once(borrow(groups.navigation))
        .chain(listed)
        .chain(std::iter::once(borrow(groups.global)))
}

/// An owned section as the borrowed pairs [`sections`] yields.
fn borrow(section: &Section) -> (&str, Vec<(&str, &str)>) {
    let hints = section.1.iter().map(|(k, d)| (k.as_str(), d.as_str()));
    (section.0.as_str(), hints.collect())
}

/// The rows the sections occupy: one header plus one row per hint.
fn section_rows(groups: &HelpSections) -> u16 {
    let listed: usize = SECTIONS.iter().map(|(_, hints)| hints.len() + 1).sum();
    let owned = groups.navigation.1.len() + groups.global.1.len() + 2;
    u16::try_from(listed + owned).unwrap_or(u16::MAX)
}

/// The width of the aligned key column, sized to the widest key.
fn key_column_width(groups: &HelpSections) -> usize {
    let listed = SECTIONS
        .iter()
        .flat_map(|(_, hints)| hints.iter().map(|(key, _)| key.width()));
    let owned = [groups.navigation, groups.global]
        .into_iter()
        .flat_map(|section| section.1.iter().map(|(key, _)| key.width()));
    listed.chain(owned).max().unwrap_or(0) + KEY_GAP
}

/// The overlay's footer, in the shared hint style.
fn footer_hint(skin: &Skin, width: usize) -> Line<'static> {
    ratada::shortcut_hints::lines(
        &[("\u{2191}\u{2193}", "scroll"), ("esc/?", "close")],
        skin.palette.accent_dim,
        width,
    )
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

    fn global_section() -> Section {
        (
            "Global".to_string(),
            vec![("q".to_string(), "quit".to_string())],
        )
    }

    fn navigation_section() -> Section {
        (
            "Navigation".to_string(),
            vec![("space".to_string(), "toggle selection".to_string())],
        )
    }

    /// The overlay used to render only between 60 and 125 columns: below that
    /// its `clamp` had `min > max`, above it the width overflowed a percentage.
    #[test]
    fn renders_at_any_terminal_size() {
        let skin = Config::default().skin();
        let navigation = navigation_section();
        let global = global_section();
        let groups = HelpSections {
            navigation: &navigation,
            global: &global,
        };
        let scroll = Scroll::default();
        for width in [1, 20, 40, 59, 60, 100, 126, 200, 400] {
            for height in [1, 2, 3, 5, 40] {
                let mut terminal =
                    Terminal::new(TestBackend::new(width, height))
                        .expect("the test backend never fails");
                terminal
                    .draw(|frame| {
                        render(frame, frame.area(), &skin, &groups, &scroll);
                    })
                    .unwrap_or_else(|error| {
                        panic!("{width}x{height} failed to render: {error}")
                    });
            }
        }
    }
}
