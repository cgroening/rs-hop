//! The help overlay listing all keyboard shortcuts, opened with `?`.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::tui::colors::{ACCENT, DIM};
use crate::tui::widgets::centered_rect;

/// The grouped shortcut list shown in the overlay.
const SHORTCUTS: &[(&str, &str)] = &[
    ("1 / 2 / 3", "switch tab (Git Repos / Files / Archive)"),
    ("Tab / Shift+Tab", "cycle to the next / previous tab"),
    ("\u{2191} \u{2193}", "move cursor (wraps)"),
    (
        "g / G · PgUp/PgDn · Ctrl+u/d",
        "top / bottom · page · half page",
    ),
    (
        "Space",
        "toggle selection   ·   Shift+\u{2191}\u{2193}: extend range",
    ),
    ("Esc", "clear the selection"),
    (
        "Enter",
        "open: git -> tool · folder -> cd · text file -> editor · else app",
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
        "force open with the default app (e.g. a text file in its GUI app)",
    ),
    (
        "f",
        "live fuzzy filter (Esc clears; matches are highlighted)",
    ),
    ("F", "toggle showing only git repos with a status change"),
    ("b", "open the selected repo on GitHub in the browser"),
    ("v", "cycle the detail panel (off / right / bottom)"),
    (
        "s",
        "git tabs: cycle sort (name/recent/frecency/custom) · Files: jump",
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
    ("z", "toggle favourite (selection or cursor)"),
    ("A", "archive / restore (selection or cursor)"),
    ("S", "set or change the slug"),
    ("i", "toggle showing slugs (dim, italic) after the name"),
    ("y", "copy the selected entry's path to the clipboard"),
    ("p", "repair a missing path"),
    ("!", "list entries with path errors and fix them"),
    (
        "r",
        "git tabs: reload status (R: + fetch) · Files: check paths exist",
    ),
    ("x", "refresh selection/cursor   ·   X: with git fetch"),
    ("?", "toggle this help"),
    ("q / Ctrl+Q", "quit"),
];

/// Renders the help overlay centred in `area`.
pub fn render(frame: &mut Frame, area: Rect) {
    let height = (SHORTCUTS.len() as u16 + 4).min(area.height);
    let rect = centered_rect(72, height, area);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .title(Span::styled(
            " Keyboard shortcuts ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT));

    let mut lines: Vec<Line> = SHORTCUTS
        .iter()
        .map(|(keys, description)| {
            Line::from(vec![
                Span::styled(
                    format!(" {keys:<12}"),
                    Style::default().fg(ACCENT),
                ),
                Span::styled(
                    (*description).to_string(),
                    Style::default().fg(DIM),
                ),
            ])
        })
        .collect();
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        " Esc / ? to close",
        Style::default().fg(DIM),
    )));
    frame.render_widget(Paragraph::new(lines).block(block), rect);
}
