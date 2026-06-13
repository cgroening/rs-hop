//! Renders the repository list as a table, with columns depending on the tab.
//!
//! The git tabs show name, branch, status and GitHub name; the Files and
//! Folders tab shows name, type and path. A leading marker column flags entries
//! whose path no longer exists, followed by the favourite star.

use std::collections::HashSet;
use std::path::PathBuf;

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
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
    IconSet, render_scrollbar, status_text, truncate,
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
    let table = Table::new(rows, widths(repos, view))
        .header(header_row(view))
        .column_spacing(1)
        .row_highlight_style(selection_style());
    let mut state = TableState::default().with_selected(Some(cursor));
    frame.render_stateful_widget(table, table_area, &mut state);

    if overflow {
        // Align the bar with the rows (below the header); its position follows
        // the same bottom-anchored offset the table scrolls to.
        let offset = cursor.saturating_sub(viewport.saturating_sub(1));
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
/// flexible to use the leftover space.
fn widths(repos: &[&Repo], view: &TableView) -> Vec<Constraint> {
    let cols = &view.config.column_widths;
    let name = sized(content_width(repos, |r| r.display_name()), 4, cols.name);
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
            let github = sized(
                content_width(repos, |r| {
                    github_text(effective_info(r, view.example_mode))
                }),
                6,
                cols.github_repo_name,
            );
            [
                lead,
                &[
                    Constraint::Length(name),
                    Constraint::Length(branch),
                    Constraint::Length(status),
                    Constraint::Length(github),
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

/// The status text used both to render and to size the status column.
fn status_display(repo: &Repo, view: &TableView) -> String {
    let info = effective_info(repo, view.example_mode);
    match info {
        None => "\u{2026}".to_string(),
        Some(info) if !info.valid && !repo.path_exists() => "-".to_string(),
        Some(info) => status_text(info, view.icons),
    }
}

/// The header row for `tab` (a leading blank cell for the selection column when
/// a selection is active).
fn header_row(view: &TableView) -> Row<'static> {
    let titles: Vec<&str> = match view.tab {
        Tab::FilesAndFolders => vec!["", "", "Name", "Type", "Path"],
        _ => vec!["", "", "Name", "Branch", "Status", "GitHub"],
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
    cells.push(Cell::from(repo.display_name()));
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
/// blank. A git entry is flagged live (missing or invalid repository); a
/// file/folder entry only once the on-demand existence check flagged its path.
fn marker_cell<'a>(repo: &Repo, view: &TableView) -> Cell<'a> {
    let errored = match repo.kind {
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
    if !info.valid && !repo.path_exists() {
        return Cell::from(Span::styled("-", Style::default().fg(DIM)));
    }
    // The status column is sized to its content, so no truncation is needed.
    let text = status_text(info, view.icons);
    let style = if is_clean(info) {
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

/// Whether the structured info reports a clean tree (no override).
fn is_clean(info: &GitInfo) -> bool {
    info.raw_status.is_none()
        && info.changes.unwrap_or(0) == 0
        && info.ahead.unwrap_or(0) == 0
        && info.behind.unwrap_or(0) == 0
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

/// The detected type label for the Files and Folders tab.
fn type_label(repo: &Repo) -> &'static str {
    if is_dir_target(&repo.path) {
        "folder"
    } else {
        "file"
    }
}
