//! The help overlay listing all keyboard shortcuts, opened with `?`.
//!
//! It looks like `ratada::help`'s overlay — uppercase section headers, an
//! aligned accent key column, dimmed descriptions and a hint footer — but stays
//! hop's own **non-blocking** overlay so a background refresh keeps running
//! behind it (see `docs/DEVELOPMENT.md`, "Architecture decisions"). The trailing
//! `Global` section is handed in by the caller, from the same tokens the footer
//! uses.

use std::cell::Cell;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::theme::Skin;
use crate::tui::widgets::centered_rect;

/// Rows the block border, the blank spacer and the footer take, on top of the
/// section rows.
const CHROME_ROWS: u16 = 5;
/// Spaces between the key column and the description.
const KEY_GAP: usize = 2;
/// Rows a page key moves.
const PAGE_ROWS: i32 = 10;

/// The shortcut list, grouped into the same sections as the footer hint bar.
/// The `Global` section is appended at render time so its keys stay in sync
/// with the keymap and the toolkit's own chords.
const SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Navigation",
        &[
            ("1 / 2 / 3", "switch tab (Git Repos / Files / Archive)"),
            ("Tab / Shift+Tab", "cycle to the next / previous tab"),
            ("\u{2191} \u{2193}", "move cursor (wraps)"),
            ("g / G", "top / bottom"),
            ("PgUp/PgDn \u{b7} Ctrl+u/d", "page \u{b7} half page"),
            ("Space", "toggle selection"),
            ("Shift+\u{2191}\u{2193}", "extend the selection range"),
            ("Esc", "clear the selection"),
        ],
    ),
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
            ("e", "edit the selected entry"),
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
            ("v", "cycle the detail panel (off / right / bottom)"),
            ("i", "toggle showing slugs (dim, italic) after the name"),
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
                "s",
                "git tabs: cycle sort (name/recent/frecency/custom) \u{b7} \
                 Files: jump",
            ),
            ("M", "Files: manage sections (add / rename / delete / move)"),
            (
                "Alt+\u{2191}\u{2193}",
                "reorder (custom sort, or within a Files section)",
            ),
            (
                "Ctrl+\u{2191}\u{2193}",
                "Files: jump to the previous / next section",
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

/// An owned `(label, hints)` section, as the app builds the `Global` group.
pub type Section = (String, Vec<(String, String)>);

/// The overlay's scroll position, plus the largest offset the last render
/// allowed — so the key handler can clamp without knowing the viewport.
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

    /// Moves the view for `key`, clamped to the scrollable range. Keys that do
    /// not scroll leave it alone.
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
        let max = i32::from(self.max.get());
        let moved = i32::from(self.offset.get()).saturating_add(delta);
        self.offset.set(moved.clamp(0, max) as u16);
    }
}

/// Renders the help overlay centred in `area`. `global` is the trailing section
/// with the app-wide chords, built from the same tokens as the footer; `scroll`
/// carries the view offset across frames.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    skin: &Skin,
    global: &Section,
    scroll: &Scroll,
) {
    let palette = &skin.palette;
    // Wide enough for the long descriptions, tall enough to fill the screen.
    let width = (area.width * 4 / 5).clamp(60, area.width);
    let height = (section_rows(global) + CHROME_ROWS).min(area.height);
    let rect = centered_rect(width, height, area);
    frame.render_widget(Clear, rect);
    let block = ratada::chrome::modal_block(skin, "Keyboard shortcuts");
    let inner = block.inner(rect);

    let key_width = key_column_width(global);
    let header_style =
        ratada::style::fg(palette.accent_dim).add_modifier(Modifier::BOLD);
    let key_style =
        ratada::style::fg(palette.accent).add_modifier(Modifier::BOLD);

    let mut lines: Vec<Line> = Vec::new();
    for (title, hints) in sections(global) {
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
    scroll.max.set(total.saturating_sub(viewport));
    let offset = scroll.offset.get().min(scroll.max.get());
    scroll.offset.set(offset);

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

/// The static sections followed by the caller's `Global` one.
fn sections(
    global: &Section,
) -> impl Iterator<Item = (&str, Vec<(&str, &str)>)> {
    let listed = SECTIONS.iter().map(|(title, hints)| {
        (*title, hints.iter().map(|(k, d)| (*k, *d)).collect())
    });
    let trailing = std::iter::once((
        global.0.as_str(),
        global
            .1
            .iter()
            .map(|(k, d)| (k.as_str(), d.as_str()))
            .collect(),
    ));
    listed.chain(trailing)
}

/// The rows the sections occupy: one header plus one row per hint.
fn section_rows(global: &Section) -> u16 {
    let listed: usize = SECTIONS.iter().map(|(_, hints)| hints.len() + 1).sum();
    u16::try_from(listed + global.1.len() + 1).unwrap_or(u16::MAX)
}

/// The width of the aligned key column, sized to the widest key.
fn key_column_width(global: &Section) -> usize {
    let listed = SECTIONS
        .iter()
        .flat_map(|(_, hints)| hints.iter().map(|(key, _)| key.width()));
    let owned = global.1.iter().map(|(key, _)| key.width());
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
