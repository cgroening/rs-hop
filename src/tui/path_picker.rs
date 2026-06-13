//! A filesystem picker for repairing or choosing entry paths.
//!
//! It opens at a starting directory (for path repair, the nearest existing
//! ancestor of the broken path), lists its children with a typed filter, lets
//! the user descend/ascend, and returns the chosen path. Folders are always
//! selectable; files only when `allow_files` is set.

use std::fs;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph,
};

use crate::tui::colors::{ACCENT, DIM, selection_style};
use crate::tui::navigation::cycle;
use crate::tui::presentation::truncate;
use crate::tui::text_input::TextInput;
use crate::tui::widgets::centered_rect;

/// Outcome of feeding a key to the picker.
pub enum PickerResult {
    /// Still browsing.
    Pending,
    /// The user chose this path.
    Selected(PathBuf),
    /// The user cancelled.
    Cancel,
}

/// One listed child entry.
struct Entry {
    path: PathBuf,
    name: String,
    is_dir: bool,
}

/// Browsable filesystem picker state.
pub struct PathPicker {
    current_dir: PathBuf,
    allow_files: bool,
    entries: Vec<Entry>,
    visible: Vec<usize>,
    filter: TextInput,
    cursor: usize,
}

impl PathPicker {
    /// Opens the picker at `start` (or the filesystem root when it has no
    /// usable directory). `allow_files` lets files be chosen, not just folders.
    pub fn new(start: &Path, allow_files: bool) -> Self {
        let current_dir = start_dir(start);
        let mut picker = PathPicker {
            current_dir,
            allow_files,
            entries: Vec::new(),
            visible: Vec::new(),
            filter: TextInput::new(""),
            cursor: 0,
        };
        picker.reload();
        picker
    }

    /// Handles a key, navigating the tree or choosing/cancelling.
    pub fn handle_key(&mut self, key: KeyEvent) -> PickerResult {
        match key.code {
            KeyCode::Esc => return PickerResult::Cancel,
            KeyCode::Up => self.move_cursor(-1),
            KeyCode::Down => self.move_cursor(1),
            KeyCode::Right => self.descend(),
            KeyCode::Left => self.ascend(),
            KeyCode::Backspace if self.filter.is_empty() => self.ascend(),
            KeyCode::Enter => return self.choose(),
            _ => {
                if self.filter.handle_key(key) {
                    self.apply_filter();
                }
            }
        }
        PickerResult::Pending
    }

    /// Renders the picker centred in `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let rect =
            centered_rect(70, area.height.saturating_sub(4).max(8), area);
        frame.render_widget(Clear, rect);
        let title = format!(
            " Pick path - {} ",
            truncate(
                &self.current_dir.to_string_lossy(),
                rect.width.saturating_sub(16) as usize,
            )
        );
        let block = Block::default()
            .title(Span::styled(
                title,
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(inner);
        self.render_filter(frame, rows[0]);
        self.render_list(frame, rows[1]);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " \u{2191}\u{2193} move · \u{2192} open · \u{2190} up · \
                 Enter choose · Esc cancel",
                Style::default().fg(DIM),
            ))),
            rows[2],
        );
    }

    /// Renders the filter line.
    fn render_filter(&self, frame: &mut Frame, area: Rect) {
        let mut spans =
            vec![Span::styled("filter: ", Style::default().fg(DIM))];
        spans.extend(self.filter.render_line(Style::default()).spans);
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// Renders the entry list.
    fn render_list(&self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .visible
            .iter()
            .map(|&index| {
                let entry = &self.entries[index];
                let suffix = if entry.is_dir { "/" } else { "" };
                let style = if entry.is_dir {
                    Style::default().fg(ACCENT)
                } else {
                    Style::default()
                };
                ListItem::new(Span::styled(
                    format!("{}{suffix}", entry.name),
                    style,
                ))
            })
            .collect();
        let list = List::new(items).highlight_style(selection_style());
        let mut state = ListState::default();
        if !self.visible.is_empty() {
            state.select(Some(self.cursor));
        }
        frame.render_stateful_widget(list, area, &mut state);
    }

    /// Moves the cursor cyclically within the visible entries.
    fn move_cursor(&mut self, delta: isize) {
        self.cursor = cycle(self.cursor, self.visible.len(), delta);
    }

    /// Descends into the highlighted directory.
    fn descend(&mut self) {
        if let Some(entry) = self.selected_entry()
            && entry.is_dir
        {
            self.current_dir = entry.path.clone();
            self.filter = TextInput::new("");
            self.reload();
        }
    }

    /// Moves to the parent directory.
    fn ascend(&mut self) {
        if let Some(parent) = self.current_dir.parent() {
            self.current_dir = parent.to_path_buf();
            self.filter = TextInput::new("");
            self.reload();
        }
    }

    /// Chooses the highlighted entry when it is selectable.
    fn choose(&mut self) -> PickerResult {
        match self.selected_entry() {
            Some(entry) if entry.is_dir || self.allow_files => {
                PickerResult::Selected(entry.path.clone())
            }
            _ => PickerResult::Pending,
        }
    }

    /// The highlighted entry, if any.
    fn selected_entry(&self) -> Option<&Entry> {
        self.visible
            .get(self.cursor)
            .map(|&index| &self.entries[index])
    }

    /// Reloads the children of the current directory and resets the filter.
    fn reload(&mut self) {
        self.entries = read_children(&self.current_dir, self.allow_files);
        self.cursor = 0;
        self.apply_filter();
    }

    /// Recomputes the visible indices from the filter text.
    fn apply_filter(&mut self) {
        let needle = self.filter.value().to_lowercase();
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                needle.is_empty() || entry.name.to_lowercase().contains(&needle)
            })
            .map(|(index, _)| index)
            .collect();
        if self.cursor >= self.visible.len() {
            self.cursor = self.visible.len().saturating_sub(1);
        }
    }
}

/// The directory to open at: `start` if it is a directory, else its parent,
/// else the filesystem root.
fn start_dir(start: &Path) -> PathBuf {
    if start.is_dir() {
        return start.to_path_buf();
    }
    start
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Reads the children of `dir`: directories first (sorted), then files (sorted)
/// when `allow_files`. Unreadable directories yield an empty list.
fn read_children(dir: &Path, allow_files: bool) -> Vec<Entry> {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<Entry> = Vec::new();
    let mut files: Vec<Entry> = Vec::new();
    for entry in read_dir.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            dirs.push(Entry {
                path,
                name,
                is_dir: true,
            });
        } else if allow_files {
            files.push(Entry {
                path,
                name,
                is_dir: false,
            });
        }
    }
    dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    dirs.into_iter().chain(files).collect()
}
