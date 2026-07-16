//! A filesystem picker for repairing or choosing entry paths.
//!
//! It opens at a starting directory (for path repair, the nearest existing
//! ancestor of the broken path), lists its children with a typed filter, lets
//! the user descend/ascend, and returns the chosen path. Folders are always
//! selectable; files only when `allow_files` is set. The box shows an `xx/yy`
//! position badge in its border and `Ctrl+h` toggles hidden (dot-prefixed)
//! entries, which are hidden by default. Styled after `ratada`'s picker while
//! staying hop's own non-blocking overlay.

use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratada::input::InputField;
use ratada::nav::cycle;
use ratada::text::truncate;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::theme::Skin;
use crate::tui::presentation::{FieldView, field_spans};
use crate::tui::skin::Colors;
use crate::tui::widgets::centered_rect;

/// The label in front of the picker's filter line.
const FILTER_LABEL: &str = "filter: ";

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
    /// When false (the default), dot-prefixed entries are hidden.
    show_hidden: bool,
    entries: Vec<Entry>,
    visible: Vec<usize>,
    filter: InputField,
    cursor: usize,
    /// The list scroll offset, carried across frames by `ratada::list`.
    offset: Cell<usize>,
}

impl PathPicker {
    /// Opens the picker at `start` (or the filesystem root when it has no
    /// usable directory). `allow_files` lets files be chosen, not just folders.
    pub fn new(start: &Path, allow_files: bool) -> Self {
        let current_dir = start_dir(start);
        let mut picker = PathPicker {
            current_dir,
            allow_files,
            show_hidden: false,
            entries: Vec::new(),
            visible: Vec::new(),
            filter: InputField::new(""),
            cursor: 0,
            offset: Cell::new(0),
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
            KeyCode::Backspace if self.filter.value().is_empty() => {
                self.ascend();
            }
            KeyCode::Char('h')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.toggle_hidden();
            }
            KeyCode::Enter => return self.choose(),
            _ => {
                if self.filter.handle_key(key) {
                    self.apply_filter();
                }
            }
        }
        PickerResult::Pending
    }

    /// Inserts a bracketed paste into the filter and re-runs the match.
    pub fn paste(&mut self, text: &str) {
        self.filter.paste(text);
        self.apply_filter();
    }

    /// Renders the picker centred in `area`, ratada-style: a rounded box titled
    /// `Pick path` with the current directory on a dim header row, the caret
    /// filter, the scrollable entry list, and a compact footer; the `xx/yy`
    /// position badge sits in the bottom-right border.
    pub fn render(&self, frame: &mut Frame, area: Rect, skin: &Skin) {
        let colors = Colors::from_palette(&skin.palette);
        let rect =
            centered_rect(70, area.height.saturating_sub(4).max(8), area);
        frame.render_widget(Clear, rect);
        let block = ratada::chrome::modal_block(skin, "Pick path");
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        // The position badge reads as part of the bottom-right border.
        let badge =
            ratada::chrome::position_badge(self.cursor, self.visible.len());
        ratada::chrome::render_badge(frame, rect, skin, &badge);

        let width = inner.width as usize;
        let footer = ratada::shortcut_hints::lines(
            &[
                ("\u{2190}\u{2192}", "browse"),
                ("enter", "pick"),
                ("^H", "hidden"),
            ],
            skin.palette.accent_dim,
            width,
        );
        let footer_h = (footer.len() as u16).max(1);
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(footer_h),
            ])
            .split(inner);

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate(&self.current_dir.to_string_lossy(), width),
                Style::default().fg(colors.dim),
            ))),
            rows[0],
        );
        self.render_filter(frame, rows[1], skin);
        self.render_list(frame, rows[2], skin);
        frame.render_widget(Paragraph::new(footer), rows[3]);
    }

    /// Renders the filter line (the toolkit's block caret, scrolling with `…`).
    fn render_filter(&self, frame: &mut Frame, area: Rect, skin: &Skin) {
        let colors = Colors::from_palette(&skin.palette);
        let mut spans =
            vec![Span::styled(FILTER_LABEL, Style::default().fg(colors.dim))];
        spans.extend(field_spans(FieldView {
            field: &self.filter,
            palette: &skin.palette,
            width: (area.width as usize)
                .saturating_sub(FILTER_LABEL.chars().count()),
            focused: true,
        }));
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// Renders the entry list via `ratada::list` (cursor highlight + scrollbar);
    /// directories keep the accent colour.
    fn render_list(&self, frame: &mut Frame, area: Rect, skin: &Skin) {
        let accent = Colors::from_palette(&skin.palette).accent;
        let width = area.width as usize;
        let rows: Vec<Line<'static>> = self
            .visible
            .iter()
            .map(|&index| {
                let entry = &self.entries[index];
                let marker = if entry.is_dir { "/" } else { " " };
                let line = Line::from(truncate(
                    &format!("{marker} {}", entry.name),
                    width,
                ));
                if entry.is_dir {
                    line.style(Style::default().fg(accent))
                } else {
                    line
                }
            })
            .collect();
        ratada::list::render(
            frame,
            area,
            skin,
            ratada::list::ListView {
                rows,
                selected: self.cursor,
                offset: &self.offset,
            },
        );
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
            self.filter = InputField::new("");
            self.reload();
        }
    }

    /// Moves to the parent directory.
    fn ascend(&mut self) {
        if let Some(parent) = self.current_dir.parent() {
            self.current_dir = parent.to_path_buf();
            self.filter = InputField::new("");
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

    /// Reloads the children of the current directory and resets the cursor.
    fn reload(&mut self) {
        self.entries = read_children(
            &self.current_dir,
            self.allow_files,
            self.show_hidden,
        );
        self.cursor = 0;
        self.offset.set(0);
        self.apply_filter();
    }

    /// Toggles hidden (dot-prefixed) entries, keeping the current directory and
    /// filter; re-reads the directory and re-applies the filter.
    fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.entries = read_children(
            &self.current_dir,
            self.allow_files,
            self.show_hidden,
        );
        self.offset.set(0);
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
/// when `allow_files`. Dot-prefixed entries are skipped unless `show_hidden`.
/// Unreadable directories yield an empty list.
fn read_children(
    dir: &Path,
    allow_files: bool,
    show_hidden: bool,
) -> Vec<Entry> {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<Entry> = Vec::new();
    let mut files: Vec<Entry> = Vec::new();
    for entry in read_dir.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !show_hidden && is_hidden(&name) {
            continue;
        }
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
    dirs.sort_by_key(|entry| entry.name.to_lowercase());
    files.sort_by_key(|entry| entry.name.to_lowercase());
    dirs.into_iter().chain(files).collect()
}

/// Whether an entry name is hidden (dot-prefixed, the Unix convention).
fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

#[cfg(test)]
mod tests {
    use super::is_hidden;

    #[test]
    fn dot_prefixed_names_are_hidden() {
        assert!(is_hidden(".git"));
        assert!(is_hidden(".config"));
        assert!(!is_hidden("src"));
        assert!(!is_hidden("Cargo.toml"));
    }
}
