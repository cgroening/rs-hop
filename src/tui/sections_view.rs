//! Renders the Files tab as a `List` of section-header bars and entry rows.
//!
//! Unlike the git table, sections interleave non-selectable header rows with
//! entry rows, so a `Table` cannot carry the cursor cleanly. Following mdtask's
//! finder, the scroll offset is kept across frames (`offset`) so the cursor
//! pages within the viewport and the list only scrolls at the edges; the
//! cursor itself is an entry display position (headers are purely visual).

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use chrono::{DateTime, Local};
use ratada::text::truncate;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::domain::repo::{Repo, is_dir_target};
use crate::domain::sections::SectionGroup;
use crate::domain::stats::{CodeEntry, GitStats};
use crate::theme::Skin;
use crate::tui::columns::{
    CellSource, ColumnSet, StatColumn, cell_text, stat_columns,
};
use crate::tui::presentation::{IconSet, name_plain, slug_style};
use crate::tui::skin::Colors;

/// The lower and upper bound for the auto-sized name column.
const NAME_MIN: usize = 4;
const NAME_MAX: usize = 30;
/// Fixed width of the type column (git / folder / file).
const TYPE_WIDTH: usize = 6;
/// Fixed width of the ZIP Backup column (fits a `YYYY-MM-DD` date).
const ZIP_WIDTH: usize = 10;

/// The styling context for a sectioned render, bundled to keep the parameter
/// count low.
pub struct SectionedView<'a> {
    /// The display-ordered sections with their entry service indices.
    pub groups: &'a [SectionGroup],
    /// All service entries (indexed by the groups' items).
    pub repos: &'a [Repo],
    /// The glyph set.
    pub icons: &'a IconSet,
    /// The active theme, for the scrollbar.
    pub skin: &'a Skin,
    /// The colour roles resolved from the active theme.
    pub colors: &'a Colors,
    /// Service indices that are part of the multi-selection.
    pub selected: &'a HashSet<usize>,
    /// Whether a multi-selection is active (shows the leading marker column).
    pub has_selection: bool,
    /// Paths flagged missing by the on-demand existence check.
    pub missing: &'a HashSet<PathBuf>,
    /// Whether to show each entry's slug (dim, italic) after its name.
    pub show_slugs: bool,
    /// Last ZIP-backup time per entry path, for the "ZIP Backup" column.
    pub zip_backups: &'a HashMap<PathBuf, DateTime<Local>>,
    /// The scroll offset carried across frames.
    pub offset: &'a Cell<usize>,
    /// Which columns to show.
    pub columns: ColumnSet,
    /// Cached code and size statistics, keyed by entry path.
    pub code: &'a HashMap<PathBuf, CodeEntry>,
    /// Cached history statistics, keyed by entry path.
    pub git: &'a HashMap<PathBuf, GitStats>,
    /// Paths a statistics worker has not reported yet.
    pub computing: &'a HashSet<PathBuf>,
    /// The current spinner glyph while a worker runs.
    pub spinner: Option<&'a str>,
    /// The reference time for ages, in unix seconds.
    pub now: i64,
}

impl SectionedView<'_> {
    /// The rendered text of one statistics cell.
    fn stat_text(&self, repo: &Repo, column: StatColumn) -> String {
        let source = CellSource {
            code: self.code.get(&repo.path),
            git: self.git.get(&repo.path),
            open_count: repo.open_count,
            last_used: repo.last_used,
            pending: self.computing.contains(&repo.path),
            now: self.now,
        };
        cell_text(column, source)
            .unwrap_or_else(|| self.spinner.unwrap_or("-").to_string())
    }
}

/// Renders the sectioned list into `area`, highlighting the entry at display
/// position `cursor`.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    cursor: usize,
    view: &SectionedView,
) {
    // The statistics sets need a column-title row; the sectioned list has none
    // of its own, so it is drawn above the list and taken off its viewport.
    let header_rows: u16 = u16::from(view.columns.is_statistics());
    let area = if header_rows > 0 && area.height > header_rows {
        render_column_header(frame, area, view);
        Rect {
            y: area.y + header_rows,
            height: area.height - header_rows,
            ..area
        }
    } else {
        area
    };

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

    let list = List::new(items).highlight_style(view.colors.selection_style());
    let mut state = ListState::default();
    *state.offset_mut() = offset;
    state.select(Some(cursor_row));
    frame.render_stateful_widget(list, list_area, &mut state);

    if overflow {
        ratada::scroll::render_scrollbar(
            frame,
            area,
            view.skin,
            ratada::nav::ScrollView {
                total: row_count,
                offset,
                viewport,
            },
        );
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
        items.push(header_item(&group.label, width, view.colors));
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

/// The column-title row for a statistics set, drawn above the sectioned list.
fn render_column_header(frame: &mut Frame, area: Rect, view: &SectionedView) {
    let row = Rect { height: 1, ..area };
    let name_width = name_width(view);
    let mut spans = vec![Span::raw("     ")];
    spans.push(Span::raw(pad("Name", name_width)));
    for column in stat_columns(view.columns) {
        spans.push(Span::raw("  "));
        spans.push(Span::raw(format!(
            "{:>width$}",
            column.title(),
            width = column.width() as usize
        )));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(view.colors.header_style()),
        row,
    );
}

/// The statistics cells of one entry, right-aligned under their headers.
fn stat_spans(view: &SectionedView, repo: &Repo) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for column in stat_columns(view.columns) {
        let text = view.stat_text(repo, *column);
        let width = column.width() as usize;
        let cell = if column.is_numeric() {
            format!("{text:>width$}")
        } else {
            pad(&text, width)
        };
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            cell,
            Style::default().fg(view.colors.foreground),
        ));
    }
    spans
}

/// A full-width section-header bar: the bold accent label then a dim rule.
fn header_item<'a>(label: &str, width: usize, colors: &Colors) -> ListItem<'a> {
    let title = format!(" {label} ");
    let used = UnicodeWidthStr::width(title.as_str());
    let rule = "\u{2500}".repeat(width.saturating_sub(used));
    ListItem::new(Line::from(vec![
        Span::styled(
            title,
            Style::default()
                .fg(colors.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(rule, Style::default().fg(colors.dim)),
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
            Style::default()
                .fg(view.colors.accent)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("  ")
    };
    let name_field = name_field_spans(
        &repo.display_name(),
        shown_slug(view, repo),
        name_width,
        view.colors,
    );
    if view.columns.is_statistics() {
        let mut spans = vec![
            lead,
            marker_span(repo, view),
            fav_span(repo, view),
            Span::raw(" "),
        ];
        spans.extend(name_field);
        spans.extend(stat_spans(view, repo));
        let item = ListItem::new(Line::from(spans));
        return if selected {
            item.style(Style::default().bg(view.colors.multi_select_bg))
        } else {
            item
        };
    }
    let kind = pad(type_label(repo), TYPE_WIDTH);
    // Cells before the path: lead(2) + marker(1) + fav(1) + space(1) + name
    //  + gap(2) + type + gap(2); the path then fills up to the trailing
    // gap(2) + ZIP-backup column at the right edge.
    let used = 2 + 1 + 1 + 1 + name_width + 2 + TYPE_WIDTH + 2 + 2 + ZIP_WIDTH;
    let path = pad(&repo.path.to_string_lossy(), width.saturating_sub(used));
    let zip = format!("{:>ZIP_WIDTH$}", zip_cell_text(repo, view));

    let mut spans = vec![
        lead,
        marker_span(repo, view),
        fav_span(repo, view),
        Span::raw(" "),
    ];
    spans.extend(name_field);
    spans.extend([
        Span::raw("  "),
        Span::raw(kind),
        Span::raw("  "),
        Span::styled(path, Style::default().fg(view.colors.dim)),
        Span::raw("  "),
        Span::styled(zip, Style::default().fg(view.colors.dim)),
    ]);
    let item = ListItem::new(Line::from(spans));
    if selected {
        item.style(Style::default().bg(view.colors.multi_select_bg))
    } else {
        item
    }
}

/// The error marker glyph (red) when the entry's path was flagged missing by
/// the existence check, else blank.
fn marker_span(repo: &Repo, view: &SectionedView) -> Span<'static> {
    if view.missing.contains(&repo.path) {
        Span::styled(
            view.icons.missing.to_string(),
            Style::default()
                .fg(view.colors.danger)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw(" ")
    }
}

/// The favourite star glyph when favourited, else blank.
fn fav_span(repo: &Repo, view: &SectionedView) -> Span<'static> {
    if repo.fav {
        Span::styled(
            view.icons.favourite.to_string(),
            Style::default().fg(view.colors.favourite),
        )
    } else {
        Span::raw(" ")
    }
}

/// The auto-sized name column width: the widest name (plus its slug when
/// shown), bounded.
fn name_width(view: &SectionedView) -> usize {
    view.groups
        .iter()
        .flat_map(|group| group.items.iter())
        .map(|&index| {
            let repo = &view.repos[index];
            UnicodeWidthStr::width(
                name_plain(&repo.display_name(), shown_slug(view, repo))
                    .as_str(),
            )
        })
        .max()
        .unwrap_or(NAME_MIN)
        .clamp(NAME_MIN, NAME_MAX)
}

/// The slug to display after `repo`'s name, or `None` when slugs are hidden.
fn shown_slug<'a>(view: &SectionedView, repo: &'a Repo) -> Option<&'a str> {
    repo.slug.as_deref().filter(|_| view.show_slugs)
}

/// The name-column spans: the name, plus a dim-italic ` slug` when shown, fit
/// and padded to `width` (the slug is shortened first when space is tight).
fn name_field_spans(
    name: &str,
    slug: Option<&str>,
    width: usize,
    colors: &Colors,
) -> Vec<Span<'static>> {
    let Some(slug) = slug else {
        return vec![Span::raw(pad(name, width))];
    };
    let name_w = UnicodeWidthStr::width(name);
    if width <= name_w + 1 {
        // No room for a space and at least one slug character.
        return vec![Span::raw(pad(name, width))];
    }
    let slug_text = truncate(slug, width - name_w - 1);
    let used = name_w + 1 + UnicodeWidthStr::width(slug_text.as_str());
    vec![
        Span::raw(name.to_string()),
        Span::styled(format!(" {slug_text}"), slug_style(colors)),
        Span::raw(" ".repeat(width.saturating_sub(used))),
    ]
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

/// The ZIP Backup cell text for `repo`: the excluded marker when the entry opts
/// out of the "backup all" run, else the last-backup date (`YYYY-MM-DD`) or a
/// dash when never backed up.
fn zip_cell_text(repo: &Repo, view: &SectionedView) -> String {
    if !repo.include_in_backup {
        return view.icons.excluded.to_string();
    }
    view.zip_backups
        .get(&repo.path)
        .map_or_else(|| "-".to_string(), |dt| dt.format("%Y-%m-%d").to_string())
}

/// The detected type label for an entry on the Files tab.
fn type_label(repo: &Repo) -> &'static str {
    if is_dir_target(&repo.path) {
        "folder"
    } else {
        "file"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settled_offset_pages_and_snaps() {
        // Cursor visible near the top: snap to 0 (only headers would hide).
        assert_eq!(settled_offset(0, 1, 1, 10, 5), 0);
        // Cursor below the viewport: scroll so it is the last visible row.
        assert_eq!(settled_offset(0, 8, 1, 20, 5), 4);
        // Cursor above the saved offset: scroll back up to it.
        assert_eq!(settled_offset(6, 2, 1, 20, 5), 2);
        // An offset at/under the first entry row snaps to the very top.
        assert_eq!(settled_offset(1, 1, 2, 10, 5), 0);
    }

    #[test]
    fn pad_fills_or_truncates() {
        assert_eq!(pad("ab", 5), "ab   ");
        assert_eq!(pad("abcdef", 4), "abc…");
    }

    #[test]
    fn name_field_spans_adds_slug_only_with_room() {
        let colors =
            Colors::from_palette(&crate::config::Config::default().palette());
        // No slug -> one padded span.
        assert_eq!(name_field_spans("hop", None, 10, &colors).len(), 1);
        // Slug with room -> name + slug + padding.
        assert_eq!(name_field_spans("hop", Some("hp"), 12, &colors).len(), 3);
        // Too narrow for a slug -> just the padded name.
        assert_eq!(name_field_spans("hop", Some("hp"), 4, &colors).len(), 1);
    }
}
