//! Renders the repository list as a table, with columns depending on the tab.
//!
//! The git tabs show name, branch, status and GitHub name; the Files and
//! Folders tab shows name, type and path. A leading marker column flags entries
//! whose path no longer exists, followed by the favourite star.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Row, Table, TableState};
use unicode_width::UnicodeWidthStr;

use crate::config::{ColumnWidth, Config};
use crate::domain::filter::Tab;
use crate::domain::repo::{GitInfo, Repo, RepoKind, is_dir_target};
use crate::tui::colors::{
    ACCENT, CHANGES, DANGER, DIM, FAVOURITE, MULTI_SELECT_BG, POSITIVE,
    header_style, selection_style,
};
use crate::tui::presentation::{
    IconSet, highlight_name, name_plain, render_scrollbar, slug_style,
    status_text, truncate,
};

/// The styling context for a table render, bundled to keep the parameter count
/// low.
pub struct TableView<'a> {
    /// The active tab (decides the columns).
    pub tab: Tab,
    /// The resolved settings (column widths).
    pub config: &'a Config,
    /// The glyph set.
    pub icons: &'a IconSet,
    /// Whether to show example git info instead of live status.
    pub example_mode: bool,
    /// While a refresh runs: the still-in-flight paths and the current spinner
    /// frame glyph. Rows whose path is in the set show the spinner. `None` when
    /// no refresh is running.
    pub spinner: Option<(&'a HashSet<PathBuf>, &'a str)>,
    /// Per visible row: whether it is part of the multi-selection.
    pub selected: &'a [bool],
    /// Whether a multi-selection is active (shows the leading marker column).
    pub has_selection: bool,
    /// Paths flagged missing by the on-demand file/folder existence check.
    pub missing: &'a HashSet<PathBuf>,
    /// Whether to show each entry's slug (dim, italic) after its name.
    pub show_slugs: bool,
    /// The active fuzzy query, to highlight matched characters in the name.
    pub query: Option<&'a str>,
    /// Last ZIP-backup time per repo path, for the "ZIP Backup" column.
    pub zip_backups: &'a HashMap<PathBuf, DateTime<Local>>,
    /// The scroll offset carried across frames, so moving the cursor up while it
    /// is still visible does not scroll the list.
    pub offset: &'a std::cell::Cell<usize>,
}

/// Predicts the scroll offset: keep the saved one unless the cursor fell off an
/// edge of the viewport, then clamp it back just far enough to reveal it.
fn settled_offset(
    saved: usize,
    cursor: usize,
    row_count: usize,
    viewport: usize,
) -> usize {
    let mut offset = saved.min(row_count.saturating_sub(1));
    if cursor < offset {
        offset = cursor;
    } else if viewport > 0 && cursor >= offset + viewport {
        offset = cursor + 1 - viewport;
    }
    offset
}

/// The slug to display after `repo`'s name, or `None` when slugs are hidden.
fn shown_slug<'a>(repo: &'a Repo, view: &TableView) -> Option<&'a str> {
    repo.slug.as_deref().filter(|_| view.show_slugs)
}

/// The name-cell spans: the name (fuzzy-highlighted when a query is active),
/// plus the dim-italic slug when shown.
fn name_cell_spans(repo: &Repo, view: &TableView) -> Vec<Span<'static>> {
    let name = repo.display_name();
    let mut spans = match view.query {
        Some(query) if !query.trim().is_empty() => highlight_name(&name, query),
        _ => vec![Span::raw(name)],
    };
    if let Some(slug) = shown_slug(repo, view) {
        spans.push(Span::styled(format!(" {slug}"), slug_style()));
    }
    spans
}

/// The git info to display for `repo`: example info in example mode, otherwise
/// the live info (which may still be loading).
fn effective_info(repo: &Repo, example_mode: bool) -> Option<&GitInfo> {
    if example_mode {
        repo.example_git_info.as_ref()
    } else {
        repo.git_info.as_ref()
    }
}

/// Renders the table of `repos` (already filtered and sorted) into `area`,
/// highlighting `cursor`.
pub fn render_table(
    frame: &mut Frame,
    area: Rect,
    repos: &[&Repo],
    cursor: usize,
    view: &TableView,
) {
    // Reserve the last column for a scrollbar when the list overflows.
    let viewport = area.height.saturating_sub(1) as usize; // minus header row
    let overflow = viewport > 0 && repos.len() > viewport;
    let table_area = if overflow {
        Rect {
            width: area.width.saturating_sub(1),
            ..area
        }
    } else {
        area
    };

    let rows: Vec<Row> = repos
        .iter()
        .enumerate()
        .map(|(row, repo)| {
            let selected = view.selected.get(row).copied().unwrap_or(false);
            let built = row_for(repo, view, selected);
            if selected {
                built.style(Style::default().bg(MULTI_SELECT_BG))
            } else {
                built
            }
        })
        .collect();
    let table = Table::new(rows, widths(repos, view, table_area.width))
        .header(header_row(view))
        .column_spacing(1)
        .row_highlight_style(selection_style());
    // Carry the offset across frames so the cursor moves within the viewport
    // without scrolling until it reaches an edge.
    let offset =
        settled_offset(view.offset.get(), cursor, repos.len(), viewport);
    view.offset.set(offset);
    let mut state = TableState::default()
        .with_offset(offset)
        .with_selected(Some(cursor));
    frame.render_stateful_widget(table, table_area, &mut state);

    if overflow {
        // Align the bar with the rows (below the header); it follows the same
        // offset the table scrolled to.
        let bar_area = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: area.height.saturating_sub(1),
        };
        render_scrollbar(frame, bar_area, repos.len(), offset, viewport);
    }
}

/// Per-column width constraints sized to the visible content, so no column is
/// wider than it needs to be. Each text column is as wide as its longest cell
/// (and header), floored at the configured minimum and capped at the configured
/// maximum; remaining width is left unused. The Files tab keeps the path column
/// flexible to use the leftover space. On the git tabs the GitHub column yields
/// width to the Name column when `available` is tight (Name has priority).
fn widths(
    repos: &[&Repo],
    view: &TableView,
    available: u16,
) -> Vec<Constraint> {
    let cols = &view.config.column_widths;
    let name = sized(
        content_width(repos, |r| {
            name_plain(&r.display_name(), shown_slug(r, view))
        }),
        4,
        cols.name,
    );
    // A 2-cell selection-marker column, only while a selection is active.
    let lead: &[Constraint] = if view.has_selection {
        &[
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(1),
        ]
    } else {
        &[Constraint::Length(1), Constraint::Length(1)]
    };
    match view.tab {
        Tab::FilesAndFolders => [
            lead,
            &[
                Constraint::Length(name),
                Constraint::Length(6),
                Constraint::Min(20),
            ],
        ]
        .concat(),
        _ => {
            let branch = sized(
                content_width(repos, |r| {
                    branch_text(effective_info(r, view.example_mode))
                }),
                6,
                cols.current_branch_name,
            );
            let status = sized(
                content_width(repos, |r| status_display(r, view)),
                6,
                cols.status,
            );
            let github_desired = sized(
                content_width(repos, |r| {
                    github_text(effective_info(r, view.example_mode))
                }),
                6,
                cols.github_repo_name,
            );
            let zip = sized(
                content_width(repos, |r| zip_date_text(r, view)),
                "ZIP Backup".len(),
                cols.zip_backup,
            );
            // GitHub is the column that yields: it keeps its content width when
            // there is room, but shrinks so the rigid columns (above all Name)
            // are never squeezed.
            let lead_cells: u16 = if view.has_selection { 3 } else { 2 };
            let lead_width: u16 = if view.has_selection { 4 } else { 2 };
            let spacing = (lead_cells + 5).saturating_sub(1); // column_spacing 1
            let fixed = lead_width + name + branch + status + zip + spacing;
            let github = github_width(github_desired, fixed, available);
            [
                lead,
                &[
                    Constraint::Length(name),
                    Constraint::Length(branch),
                    Constraint::Length(status),
                    Constraint::Length(github),
                    Constraint::Length(zip),
                ],
            ]
            .concat()
        }
    }
}

/// The widest rendered cell (display columns) over `repos` for one column.
fn content_width<F>(repos: &[&Repo], cell: F) -> usize
where
    F: Fn(&Repo) -> String,
{
    repos
        .iter()
        .map(|repo| UnicodeWidthStr::width(cell(repo).as_str()))
        .max()
        .unwrap_or(0)
}

/// The column width for `content`: at least the `header` label and the
/// configured minimum, capped at the configured maximum.
fn sized(content: usize, header: usize, width: ColumnWidth) -> u16 {
    let floor = width.min.max(header);
    let mut chosen = content.max(floor);
    if let Some(max) = width.max {
        chosen = chosen.min(max.max(floor));
    }
    chosen as u16
}

/// The GitHub column width: its `desired` content width, but capped at the space
/// left after the rigid columns (`fixed`) so Name and friends are never
/// squeezed. Shrinks toward 0 when tight; never overflows the row.
fn github_width(desired: u16, fixed: u16, available: u16) -> u16 {
    desired.min(available.saturating_sub(fixed))
}

/// The status text used both to render and to size the status column.
fn status_display(repo: &Repo, view: &TableView) -> String {
    let info = effective_info(repo, view.example_mode);
    match info {
        None => "\u{2026}".to_string(),
        Some(info) if info.is_path_missing() => "-".to_string(),
        Some(info) => status_text(info, view.icons),
    }
}

/// The header row for `tab` (a leading blank cell for the selection column when
/// a selection is active).
fn header_row(view: &TableView) -> Row<'static> {
    let titles: Vec<&str> = match view.tab {
        Tab::FilesAndFolders => vec!["", "", "Name", "Type", "Path"],
        _ => vec!["", "", "Name", "Branch", "Status", "GitHub", "ZIP Backup"],
    };
    let mut cells: Vec<Cell> = Vec::new();
    if view.has_selection {
        cells.push(Cell::from(""));
    }
    cells.extend(titles.into_iter().map(Cell::from));
    Row::new(cells).style(header_style())
}

/// Builds the data row for one entry; `selected` drives the marker column.
fn row_for<'a>(repo: &Repo, view: &TableView, selected: bool) -> Row<'a> {
    let mut cells: Vec<Cell> = Vec::new();
    if view.has_selection {
        cells.push(selection_cell(selected));
    }
    cells.push(marker_cell(repo, view));
    cells.push(fav_cell(repo, view.icons));
    cells.push(Cell::from(Line::from(name_cell_spans(repo, view))));
    match view.tab {
        Tab::FilesAndFolders => {
            cells.push(Cell::from(type_label(repo)));
            cells.push(Cell::from(Span::styled(
                repo.path.to_string_lossy().into_owned(),
                Style::default().fg(DIM),
            )));
        }
        _ => {
            let info = effective_info(repo, view.example_mode);
            // Only ellipsize a branch that exceeds its configured maximum; the
            // column itself is sized to the content otherwise.
            let branch_cap = view
                .config
                .column_widths
                .current_branch_name
                .max
                .unwrap_or(usize::MAX);
            cells.push(Cell::from(truncate(&branch_text(info), branch_cap)));
            cells.push(status_cell(repo, info, view));
            cells.push(Cell::from(github_text(info)));
            cells.push(Cell::from(Span::styled(
                zip_date_text(repo, view),
                Style::default().fg(DIM),
            )));
        }
    }
    Row::new(cells)
}

/// The selection-marker cell: a bold accent `▸` when selected, else blank.
fn selection_cell<'a>(selected: bool) -> Cell<'a> {
    let symbol = if selected { "\u{25b8} " } else { "  " };
    Cell::from(Span::styled(
        symbol,
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    ))
}

/// The marker cell: a red warning glyph when the entry has a path error, else
/// blank. A git entry is flagged from its gathered git info (missing or invalid
/// repository); a file/folder entry only once the on-demand existence check
/// flagged its path.
fn marker_cell<'a>(repo: &Repo, view: &TableView) -> Cell<'a> {
    let errored = match repo.kind {
        RepoKind::Git if view.example_mode => repo.example_error().is_some(),
        RepoKind::Git => repo.entry_error().is_some(),
        RepoKind::Path => view.missing.contains(&repo.path),
    };
    if errored {
        Cell::from(Span::styled(
            view.icons.missing.to_string(),
            Style::default().fg(DANGER).add_modifier(Modifier::BOLD),
        ))
    } else {
        Cell::from(" ")
    }
}

/// The favourite cell: a star when favourited, else blank.
fn fav_cell<'a>(repo: &Repo, icons: &IconSet) -> Cell<'a> {
    if repo.fav {
        Cell::from(Span::styled(
            icons.favourite.to_string(),
            Style::default().fg(FAVOURITE),
        ))
    } else {
        Cell::from(" ")
    }
}

/// The status cell: a pending marker while this row still awaits a running
/// refresh, dim while first loading, green when clean, yellow when there are
/// uncommitted changes, default otherwise (e.g. only ahead/behind).
fn status_cell<'a>(
    repo: &Repo,
    info: Option<&GitInfo>,
    view: &TableView,
) -> Cell<'a> {
    // A refresh is running and this row is still in flight: animated spinner.
    if let Some((in_flight, glyph)) = view.spinner
        && in_flight.contains(&repo.path)
    {
        return Cell::from(Span::styled(
            glyph.to_string(),
            Style::default().fg(ACCENT),
        ));
    }
    let Some(info) = info else {
        return Cell::from(Span::styled("…", Style::default().fg(DIM)));
    };
    if info.is_path_missing() {
        return Cell::from(Span::styled("-", Style::default().fg(DIM)));
    }
    // The status column is sized to its content, so no truncation is needed.
    let text = status_text(info, view.icons);
    let style = if info.is_clean() {
        Style::default().fg(POSITIVE)
    } else if has_changes(info) {
        Style::default().fg(CHANGES)
    } else {
        Style::default()
    };
    Cell::from(Span::styled(text, style))
}

/// Whether the info reports uncommitted changes (a non-clean working tree).
fn has_changes(info: &GitInfo) -> bool {
    info.changes.unwrap_or(0) > 0
}

/// The branch text, or a loading marker / dash.
fn branch_text(info: Option<&GitInfo>) -> String {
    match info {
        None => "…".to_string(),
        Some(info) => info
            .current_branch_name
            .clone()
            .unwrap_or_else(|| "-".to_string()),
    }
}

/// The GitHub name text, or a dash.
fn github_text(info: Option<&GitInfo>) -> String {
    info.and_then(|info| info.github_repo_name.clone())
        .unwrap_or_else(|| "-".to_string())
}

/// The ZIP Backup cell text for `repo`: the excluded marker when the entry opts
/// out of the "backup all" run, else the last-backup date (`YYYY-MM-DD`) or a
/// dash when never backed up. Read from the precomputed map (no filesystem I/O).
fn zip_date_text(repo: &Repo, view: &TableView) -> String {
    if !repo.include_in_backup {
        return view.icons.excluded.to_string();
    }
    view.zip_backups
        .get(&repo.path)
        .map_or_else(|| "-".to_string(), |dt| dt.format("%Y-%m-%d").to_string())
}

/// The detected type label for the Files and Folders tab.
fn type_label(repo: &Repo) -> &'static str {
    if is_dir_target(&repo.path) {
        "folder"
    } else {
        "file"
    }
}

#[cfg(test)]
mod tests {
    use super::{github_width, settled_offset};

    #[test]
    fn settled_offset_stays_put_when_cursor_moves_up_but_is_visible() {
        // Viewport of 5 showing rows 4..9; the cursor moves up from 8 to 5. It
        // stays visible, so the offset must not move (the eager-scroll bug).
        assert_eq!(settled_offset(4, 5, 20, 5), 4);
        assert_eq!(settled_offset(4, 4, 20, 5), 4);
    }

    #[test]
    fn settled_offset_scrolls_up_only_at_the_top_edge() {
        // Only once the cursor crosses above the top visible row does it scroll.
        assert_eq!(settled_offset(4, 3, 20, 5), 3);
    }

    #[test]
    fn settled_offset_pages_down_at_the_bottom_edge() {
        // Cursor past the bottom row scrolls down just enough to reveal it.
        assert_eq!(settled_offset(4, 9, 20, 5), 5);
    }

    #[test]
    fn github_keeps_content_width_when_there_is_room() {
        // Plenty of space: GitHub shows its full content width.
        assert_eq!(github_width(20, 60, 200), 20);
    }

    #[test]
    fn github_yields_width_when_tight() {
        // 90 wide, 64 taken by the rigid columns: GitHub shrinks from 39 to 26
        // so Name (inside `fixed`) keeps its width.
        assert_eq!(github_width(39, 64, 90), 26);
    }

    #[test]
    fn github_collapses_rather_than_overflowing() {
        // No room left: GitHub goes to 0 instead of pushing the row over.
        assert_eq!(github_width(39, 95, 90), 0);
    }
}
