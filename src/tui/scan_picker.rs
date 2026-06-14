//! A standalone multi-select picker for `hop scan`: choose which discovered
//! git repositories to import. Owns its own [`Tui`] guard and render loop;
//! returns the chosen paths, or `None` when cancelled.

use std::io;
use std::path::{Path, PathBuf};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, List, ListItem, ListState, Paragraph,
};

use crate::tui::colors::{ACCENT, DIM, selection_style};
use crate::tui::navigation::cycle;
use crate::tui::presentation::truncate;
use crate::tui::terminal::Tui;

/// Runs the picker over the discovered-but-new `found` repos, showing the
/// already-`known` ones for context. Returns the chosen paths, or `None` when
/// the user cancels.
///
/// # Errors
/// Returns an I/O error if the terminal cannot be set up or read.
pub fn run(
    found: &[PathBuf],
    known: &[PathBuf],
) -> io::Result<Option<Vec<PathBuf>>> {
    let mut tui = Tui::new()?;
    let mut state = PickerState::new(found.len());
    loop {
        tui.terminal
            .draw(|frame| render(frame, found, known, &state))?;
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Ok(None),
            KeyCode::Enter => return Ok(Some(state.chosen(found))),
            KeyCode::Up => state.move_cursor(-1),
            KeyCode::Down => state.move_cursor(1),
            KeyCode::Char(' ') => state.toggle(),
            KeyCode::Char('a') => state.toggle_all(),
            _ => {}
        }
    }
}

/// The picker's mutable state: per-row selection (all on by default) and cursor.
struct PickerState {
    selected: Vec<bool>,
    cursor: usize,
}

impl PickerState {
    fn new(len: usize) -> Self {
        PickerState {
            selected: vec![true; len],
            cursor: 0,
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        self.cursor = cycle(self.cursor, self.selected.len(), delta);
    }

    fn toggle(&mut self) {
        if let Some(flag) = self.selected.get_mut(self.cursor) {
            *flag = !*flag;
        }
    }

    /// Selects all when any is unselected, otherwise clears all.
    fn toggle_all(&mut self) {
        let select = self.selected.iter().any(|s| !s);
        self.selected.iter_mut().for_each(|s| *s = select);
    }

    /// The chosen paths (the selected rows of `found`).
    fn chosen(&self, found: &[PathBuf]) -> Vec<PathBuf> {
        found
            .iter()
            .zip(&self.selected)
            .filter(|&(_, &on)| on)
            .map(|(path, _)| path.clone())
            .collect()
    }
}

/// Renders the picker: a title, the selectable new repos, the dim known list
/// and a hint line.
fn render(
    frame: &mut Frame,
    found: &[PathBuf],
    known: &[PathBuf],
    state: &PickerState,
) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    let count = state.selected.iter().filter(|s| **s).count();
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(
                " Scan: {} new git repo(s) - {count} selected",
                found.len()
            ),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))),
        chunks[0],
    );

    render_body(frame, chunks[1], found, known, state);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " Space toggle · a all · Enter import · Esc cancel",
            Style::default().fg(DIM),
        ))),
        chunks[2],
    );
}

/// Renders the selectable list and, when present, the dim known-repos panel.
fn render_body(
    frame: &mut Frame,
    area: Rect,
    found: &[PathBuf],
    known: &[PathBuf],
    state: &PickerState,
) {
    let (list_area, known_area) = if known.is_empty() {
        (area, None)
    } else {
        let height = (known.len() as u16 + 2).min(area.height / 2).max(3);
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(height)])
            .split(area);
        (parts[0], Some(parts[1]))
    };

    let width = list_area.width as usize;
    let items: Vec<ListItem> = found
        .iter()
        .enumerate()
        .map(|(row, path)| {
            let mark = if state.selected[row] { "[x] " } else { "[ ] " };
            let name = basename(path);
            let rest = width.saturating_sub(mark.len() + name.len() + 1);
            ListItem::new(Line::from(vec![
                Span::styled(mark, Style::default().fg(ACCENT)),
                Span::raw(name),
                Span::raw(" "),
                Span::styled(
                    truncate(&path.to_string_lossy(), rest),
                    Style::default().fg(DIM),
                ),
            ]))
        })
        .collect();
    let list = List::new(items).highlight_style(selection_style());
    let mut list_state = ListState::default().with_selected(Some(state.cursor));
    frame.render_stateful_widget(list, list_area, &mut list_state);

    if let Some(known_area) = known_area {
        render_known(frame, known_area, known);
    }
}

/// Renders the dim, non-selectable "already in hop" panel.
fn render_known(frame: &mut Frame, area: Rect, known: &[PathBuf]) {
    let block = Block::default()
        .title(format!(" already in hop ({}) ", known.len()))
        .borders(Borders::TOP)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(DIM));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let width = inner.width as usize;
    let lines: Vec<Line> = known
        .iter()
        .map(|path| {
            Line::from(Span::styled(
                truncate(&path.to_string_lossy(), width),
                Style::default().fg(DIM),
            ))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The final path component, or the whole path when it has none.
fn basename(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}
