//! Renders the Files tab as a `List` of section-header bars and entry rows.
//!
//! Unlike the git table, sections interleave non-selectable header rows with
//! entry rows, so a `Table` cannot carry the cursor cleanly. Following mdtask's
//! finder, the scroll offset is kept across frames (`offset`) so the cursor
//! pages within the viewport and the list only scrolls at the edges; the
//! cursor itself is an entry display position (headers are purely visual).

use std::cell::Cell;
use std::collections::HashSet;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState};
use unicode_width::UnicodeWidthStr;

use crate::domain::repo::{Repo, RepoKind};
use crate::domain::sections::SectionGroup;
use crate::tui::colors::{
    ACCENT, DANGER, DIM, FAVOURITE, MULTI_SELECT_BG, selection_style,
};
use crate::tui::presentation::{IconSet, render_scrollbar, truncate};

/// The lower and upper bound for the auto-sized name column.
const NAME_MIN: usize = 4;
const NAME_MAX: usize = 30;
/// Fixed width of the type column (git / folder / file).
const TYPE_WIDTH: usize = 6;

/// The styling context for a sectioned render, bundled to keep the parameter
/// count low.
pub struct SectionedView<'a> {
    /// The display-ordered sections with their entry service indices.
    pub groups: &'a [SectionGroup],
    /// All service entries (indexed by the groups' items).
    pub repos: &'a [Repo],
    /// The glyph set.
    pub icons: &'a IconSet,
    /// Service indices that are part of the multi-selection.
    pub selected: &'a HashSet<usize>,
    /// Whether a multi-selection is active (shows the leading marker column).
    pub has_selection: bool,
    /// The scroll offset carried across frames.
    pub offset: &'a Cell<usize>,
}

/// Renders the sectioned list into `area`, highlighting the entry at display
/// position `cursor`.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    cursor: usize,
    view: &SectionedView,
) {
    let row_count = view.groups.len() + entry_count(view.groups);
    let viewport = area.height as usize;
    let overflow = viewport > 0 && row_count > viewport;
    // Reserve the rightmost column for the scrollbar when the list overflows.
    let list_area = if overflow {
        Rect {
            width: area.width.saturating_sub(1),
            ..area
        }
    } else {
        area
    };
    let content_width = list_area.width as usize;

    let (items, cursor_row, first_entry_row) =
        build_items(view, cursor, content_width);

    let offset = settled_offset(
        view.offset.get(),
        cursor_row,
        first_entry_row,
        row_count,
        viewport,
    );
    view.offset.set(offset);

    let list = List::new(items).highlight_style(selection_style());
    let mut state = ListState::default();
    *state.offset_mut() = offset;
    state.select(Some(cursor_row));
    frame.render_stateful_widget(list, list_area, &mut state);

    if overflow {
        render_scrollbar(frame, area, row_count, offset, viewport);
    }
}

/// The total number of entry rows across `groups`.
fn entry_count(groups: &[SectionGroup]) -> usize {
    groups.iter().map(|group| group.items.len()).sum()
}

/// Builds the list items and returns them with the cursor's row index and the
/// first entry row index (used to snap the offset to the very top).
fn build_items<'a>(
    view: &SectionedView,
    cursor: usize,
    width: usize,
) -> (Vec<ListItem<'a>>, usize, usize) {
    let name_width = name_width(view);
    let mut items: Vec<ListItem> = Vec::new();
    let mut cursor_row = 0;
    let mut first_entry_row = 0;
    let mut entry_pos = 0;
    let mut seen_entry = false;
    for group in view.groups {
        items.push(header_item(&group.label, width));
        for &index in &group.items {
            let row = items.len();
            if !seen_entry {
                first_entry_row = row;
                seen_entry = true;
            }
            if entry_pos == cursor {
                cursor_row = row;
            }
            items.push(entry_item(view, index, name_width, width));
            entry_pos += 1;
        }
    }
    (items, cursor_row, first_entry_row)
}

/// Predicts the scroll offset: keep the saved one unless the cursor fell off an
/// edge, then clamp it back; snap to the top when only header rows would hide.
fn settled_offset(
    saved: usize,
    cursor_row: usize,
    first_entry_row: usize,
    row_count: usize,
    viewport: usize,
) -> usize {
    let mut offset = saved.min(row_count.saturating_sub(1));
    if cursor_row < offset {
        offset = cursor_row;
    } else if viewport > 0 && cursor_row >= offset + viewport {
        offset = cursor_row + 1 - viewport;
    }
    if offset <= first_entry_row {
        offset = 0;
    }
    offset
}

/// A full-width section-header bar: the bold accent label then a dim rule.
fn header_item<'a>(label: &str, width: usize) -> ListItem<'a> {
    let title = format!(" {label} ");
    let used = UnicodeWidthStr::width(title.as_str());
    let rule = "\u{2500}".repeat(width.saturating_sub(used));
    ListItem::new(Line::from(vec![
        Span::styled(
            title,
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(rule, Style::default().fg(DIM)),
    ]))
}

/// One entry row: marker/fav prefix, the auto-sized name, the type and the dim
/// path, tinted when part of the multi-selection.
fn entry_item<'a>(
    view: &SectionedView,
    index: usize,
    name_width: usize,
    width: usize,
) -> ListItem<'a> {
    let repo = &view.repos[index];
    let selected = view.selected.contains(&index);
    // A 2-cell lead: the selection marker when a selection is active.
    let lead = if view.has_selection && selected {
        Span::styled(
            "\u{25b8} ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("  ")
    };
    let name = pad(&repo.display_name(), name_width);
    let kind = pad(kind_label(repo.kind), TYPE_WIDTH);
    // Cells before the path: lead(2) + marker(1) + fav(1) + space(1) + name
    //  + gap(2) + type + gap(2).
    let used = 2 + 1 + 1 + 1 + name_width + 2 + TYPE_WIDTH + 2;
    let path =
        truncate(&repo.path.to_string_lossy(), width.saturating_sub(used));

    let spans = vec![
        lead,
        marker_span(repo, view.icons),
        fav_span(repo, view.icons),
        Span::raw(" "),
        Span::raw(name),
        Span::raw("  "),
        Span::raw(kind),
        Span::raw("  "),
        Span::styled(path, Style::default().fg(DIM)),
    ];
    let item = ListItem::new(Line::from(spans));
    if selected {
        item.style(Style::default().bg(MULTI_SELECT_BG))
    } else {
        item
    }
}

/// The error marker glyph (red) when the entry's path is missing, else blank.
fn marker_span(repo: &Repo, icons: &IconSet) -> Span<'static> {
    if repo.entry_error().is_some() {
        Span::styled(
            icons.missing.to_string(),
            Style::default().fg(DANGER).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw(" ")
    }
}

/// The favourite star glyph when favourited, else blank.
fn fav_span(repo: &Repo, icons: &IconSet) -> Span<'static> {
    if repo.fav {
        Span::styled(
            icons.favourite.to_string(),
            Style::default().fg(FAVOURITE),
        )
    } else {
        Span::raw(" ")
    }
}

/// The auto-sized name column width: the widest name, bounded.
fn name_width(view: &SectionedView) -> usize {
    view.groups
        .iter()
        .flat_map(|group| group.items.iter())
        .map(|&index| {
            UnicodeWidthStr::width(view.repos[index].display_name().as_str())
        })
        .max()
        .unwrap_or(NAME_MIN)
        .clamp(NAME_MIN, NAME_MAX)
}

/// Pads `text` with trailing spaces to `width`, truncating when it is longer.
fn pad(text: &str, width: usize) -> String {
    let len = UnicodeWidthStr::width(text);
    if len >= width {
        truncate(text, width)
    } else {
        format!("{text}{}", " ".repeat(width - len))
    }
}

/// The type label for the Files tab.
fn kind_label(kind: RepoKind) -> &'static str {
    match kind {
        RepoKind::Git => "git",
        RepoKind::Folder => "folder",
        RepoKind::File => "file",
    }
}
