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

use crate::config::Config;
use crate::domain::filter::{Tab, TabKind};
use crate::domain::repo::{Repo, is_dir_target};
use crate::domain::sections::SectionGroup;
use crate::domain::stats::{CodeEntry, GitStats};
use crate::theme::Skin;
use crate::tui::columns::{
    CellSource, ColumnSet, StatColumn, cell_text, stat_columns,
};
use crate::tui::git_columns::{
    branch_text, effective_info, git_marker_errored, github_text,
    status_display, zip_date_text,
};
use crate::tui::presentation::{IconSet, slug_style, status_span};
use crate::tui::skin::Colors;

/// The lower and upper bound for the auto-sized name column.
const NAME_MIN: usize = 4;
const NAME_MAX: usize = 30;
/// Fixed width of the type column (git / folder / file).
const TYPE_WIDTH: usize = 6;
/// Fixed width of the ZIP Backup column (fits a `YYYY-MM-DD` date).
const ZIP_WIDTH: usize = 10;
/// Lower and upper bounds for the git branch column.
const BRANCH_MIN: usize = 6;
const BRANCH_MAX: usize = 20;
/// Lower and upper bounds for the git status column.
const STATUS_MIN: usize = 4;
const STATUS_MAX: usize = 12;
/// Upper bound of the Slug column, so a long slug cannot dominate.
const SLUG_MAX: usize = 20;

/// The styling context for a sectioned render, bundled to keep the parameter
/// count low.
pub struct SectionedView<'a> {
    /// The active tab (decides the git vs files column layout).
    pub tab: Tab,
    /// The display-ordered sections with their entry service indices.
    pub groups: &'a [SectionGroup],
    /// All service entries (indexed by the groups' items).
    pub repos: &'a [Repo],
    /// The resolved settings (branch column cap).
    pub config: &'a Config,
    /// Whether to show example git info instead of live status.
    pub example_mode: bool,
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
    /// While a refresh runs: the still-in-flight paths and the current spinner
    /// glyph. Rows whose path is in the set show the spinner. `None` outside a
    /// run.
    pub spinner: Option<(&'a HashSet<PathBuf>, &'a str)>,
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
            .unwrap_or_else(|| self.spinner_glyph().to_string())
    }

    /// The current spinner glyph, or a dash outside a run.
    fn spinner_glyph(&self) -> &str {
        self.spinner.map_or("-", |(_, glyph)| glyph)
    }

    /// Whether `repo` is still in flight in the running refresh.
    fn is_in_flight(&self, repo: &Repo) -> bool {
        self.spinner
            .is_some_and(|(in_flight, _)| in_flight.contains(&repo.path))
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
    // Keep the Slug column reserved so the statistics titles stay aligned.
    let slug_w = slug_col_width(view);
    if slug_w > 0 {
        spans.push(Span::raw("  "));
        spans.push(Span::raw(pad("Slug", slug_w)));
    }
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

/// One entry row: a shared marker/fav/name prefix, then the git or files
/// columns, tinted when part of the multi-selection.
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
    let name_field = name_field_spans(&repo.display_name(), name_width);
    let mut spans = vec![
        lead,
        marker_span(repo, view),
        fav_span(repo, view),
        Span::raw(" "),
    ];
    spans.extend(name_field);
    // The slug is its own dim-italic column right after the name; `slug_extra`
    // is the cells it consumes (its gap + width) so the flexible path/github
    // columns still fit.
    let slug_w = slug_col_width(view);
    let slug_extra = if view.show_slugs { 2 + slug_w } else { 0 };
    if view.show_slugs {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            pad(shown_slug(view, repo).unwrap_or(""), slug_w),
            slug_style(view.colors),
        ));
    }
    let prefix = name_width + slug_extra;
    if view.columns.is_statistics() {
        spans.extend(stat_spans(view, repo));
    } else {
        match view.tab.kind() {
            TabKind::Files => {
                spans.extend(files_spans(repo, view, prefix, width))
            }
            TabKind::Git => spans.extend(git_spans(repo, view, prefix, width)),
        }
    }
    let item = ListItem::new(Line::from(spans));
    if selected {
        item.style(Style::default().bg(view.colors.multi_select_bg))
    } else {
        item
    }
}

/// The Files-tab columns after the name/slug prefix: type, the dim path and the
/// ZIP date. `prefix` is the name column plus the optional slug column.
fn files_spans(
    repo: &Repo,
    view: &SectionedView,
    prefix: usize,
    width: usize,
) -> Vec<Span<'static>> {
    let kind = pad(type_label(repo), TYPE_WIDTH);
    // lead(2) + marker(1) + fav(1) + space(1) + prefix + gap(2) + type + gap(2);
    // the path then fills up to the trailing gap(2) + ZIP column at the edge.
    let used = 2 + 1 + 1 + 1 + prefix + 2 + TYPE_WIDTH + 2 + 2 + ZIP_WIDTH;
    let path = pad(&repo.path.to_string_lossy(), width.saturating_sub(used));
    let zip = format!("{:>ZIP_WIDTH$}", zip_cell_text(repo, view));
    vec![
        Span::raw("  "),
        Span::raw(kind),
        Span::raw("  "),
        Span::styled(path, Style::default().fg(view.colors.dim)),
        Span::raw("  "),
        Span::styled(zip, Style::default().fg(view.colors.dim)),
    ]
}

/// The git-tab columns after the name: branch and GitHub name in the foreground
/// (like the name), the coloured status, and the dim ZIP date. GitHub flexes
/// into the leftover width.
fn git_spans(
    repo: &Repo,
    view: &SectionedView,
    prefix: usize,
    width: usize,
) -> Vec<Span<'static>> {
    let (branch_w, status_w, github_w) = git_column_widths(view, prefix, width);
    let info = effective_info(repo, view.example_mode);
    let branch = pad(&branch_text(info), branch_w);
    let github = pad(&github_text(info), github_w);
    let zip = format!("{:>ZIP_WIDTH$}", zip_cell_text(repo, view));
    let fg = Style::default().fg(view.colors.foreground);
    let mut spans =
        vec![Span::raw("  "), Span::styled(branch, fg), Span::raw("  ")];
    spans.extend(git_status_spans(repo, view, info, status_w));
    spans.extend([
        Span::raw("  "),
        Span::styled(github, fg),
        Span::raw("  "),
        Span::styled(zip, Style::default().fg(view.colors.dim)),
    ]);
    spans
}

/// The status spans padded to `width`: the spinner while the row is in flight,
/// otherwise the shared coloured status followed by padding.
fn git_status_spans(
    repo: &Repo,
    view: &SectionedView,
    info: Option<&crate::domain::repo::GitInfo>,
    width: usize,
) -> Vec<Span<'static>> {
    if view.is_in_flight(repo) {
        return vec![Span::styled(
            pad(view.spinner_glyph(), width),
            Style::default().fg(view.colors.accent),
        )];
    }
    let text = status_display(info, view.icons);
    let used = UnicodeWidthStr::width(text.as_str()).min(width);
    vec![
        status_span(info, view.icons, view.colors),
        Span::raw(" ".repeat(width.saturating_sub(used))),
    ]
}

/// The git columns' widths: branch and status sized to their bounded content,
/// GitHub taking the leftover width (so the row fills without overflowing).
fn git_column_widths(
    view: &SectionedView,
    prefix: usize,
    width: usize,
) -> (usize, usize, usize) {
    let branch = col_content(view, |r| {
        branch_text(effective_info(r, view.example_mode))
    })
    .clamp(BRANCH_MIN, BRANCH_MAX);
    let status = col_content(view, |r| {
        status_display(effective_info(r, view.example_mode), view.icons)
    })
    .clamp(STATUS_MIN, STATUS_MAX);
    // lead(2) + marker(1) + fav(1) + space(1) + prefix + gap(2) + branch + gap(2)
    // + status + gap(2) + github + gap(2) + ZIP; GitHub flexes into the rest.
    // `prefix` = the name column plus the optional slug column.
    let used =
        2 + 1 + 1 + 1 + prefix + 2 + branch + 2 + status + 2 + 2 + ZIP_WIDTH;
    let github = width.saturating_sub(used);
    (branch, status, github)
}

/// The widest rendered cell (display columns) over every entry for one column.
fn col_content<F>(view: &SectionedView, cell: F) -> usize
where
    F: Fn(&Repo) -> String,
{
    view.groups
        .iter()
        .flat_map(|group| group.items.iter())
        .map(|&index| UnicodeWidthStr::width(cell(&view.repos[index]).as_str()))
        .max()
        .unwrap_or(0)
}

/// The error marker glyph (red) when the entry has a path/repo error, else
/// blank: a git entry is flagged from its gathered info, a path entry once the
/// existence check flagged it.
fn marker_span(repo: &Repo, view: &SectionedView) -> Span<'static> {
    let errored = git_marker_errored(repo, view.example_mode)
        || view.missing.contains(&repo.path);
    if errored {
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

/// The auto-sized name column width: the widest display name, bounded.
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

/// The Slug column width: the widest entry slug (bounded), or 0 when slugs are
/// hidden or no entry has one.
fn slug_col_width(view: &SectionedView) -> usize {
    if !view.show_slugs {
        return 0;
    }
    view.groups
        .iter()
        .flat_map(|group| group.items.iter())
        .map(|&index| {
            view.repos[index]
                .slug
                .as_deref()
                .map_or(0, UnicodeWidthStr::width)
        })
        .max()
        .unwrap_or(0)
        .min(SLUG_MAX)
}

/// The slug to display for `repo`, or `None` when slugs are hidden.
fn shown_slug<'a>(view: &SectionedView, repo: &'a Repo) -> Option<&'a str> {
    repo.slug.as_deref().filter(|_| view.show_slugs)
}

/// The name-column spans: the name, padded to `width`.
fn name_field_spans(name: &str, width: usize) -> Vec<Span<'static>> {
    vec![Span::raw(pad(name, width))]
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

/// The ZIP Backup cell text for `repo` (the shared git-columns helper).
fn zip_cell_text(repo: &Repo, view: &SectionedView) -> String {
    zip_date_text(repo, view.icons, view.zip_backups)
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
    fn name_field_spans_pads_the_name() {
        // The slug is its own column now; the name field is a single padded
        // span sized to the name column width.
        let spans = name_field_spans("hop", 10);
        assert_eq!(spans.len(), 1);
    }
}
