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
    ("1 / 2 / 3", "switch tab (Git Repos / Files / Archiv)"),
    ("\u{2191} \u{2193}", "move cursor (wraps)"),
    (
        "Space",
        "toggle selection   ·   Shift+\u{2191}\u{2193}: extend range",
    ),
    ("Esc", "clear the selection"),
    ("Enter", "open: write path + launch git tool, then exit"),
    ("o", "jump only: write path and exit (cd, no tool)"),
    ("f", "live fuzzy filter (Esc clears)"),
    ("s", "cycle sort (name / recent / custom)"),
    ("Alt+\u{2191}\u{2193}", "reorder entry (custom sort only)"),
    ("n", "add an entry (pick a path, then fill the form)"),
    ("e", "edit the selected entry"),
    ("d", "delete (acts on the selection, else the cursor)"),
    ("z", "toggle favourite (selection or cursor)"),
    ("A", "archive / restore (selection or cursor)"),
    ("S", "set or change the slug"),
    ("y", "copy the selected entry's path to the clipboard"),
    ("p", "repair a missing path"),
    ("!", "list entries with path errors and fix them"),
    ("r", "reload all git status   ·   R: git fetch + reload"),
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
