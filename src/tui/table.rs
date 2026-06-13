//! Renders the repository list as a table, with columns depending on the tab.
//!
//! The git tabs show name, branch, status and GitHub name; the Files and
//! Folders tab shows name, type and path. A leading marker column flags entries
//! whose path no longer exists, followed by the favourite star.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Span, Text};
use ratatui::widgets::{Cell, Row, Table, TableState};

use crate::config::{ColumnWidth, Config};
use crate::domain::filter::Tab;
use crate::domain::repo::{GitInfo, Repo, RepoKind};
use crate::tui::colors::{
    CHANGES, DANGER, DIM, FAVOURITE, POSITIVE, header_style, selection_style,
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

    let rows: Vec<Row> = repos.iter().map(|repo| row_for(repo, view)).collect();
    let table = Table::new(rows, widths(view.tab, view.config))
        .header(header_row(view.tab))
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

/// The column width constraints for `tab`.
fn widths(tab: Tab, config: &Config) -> Vec<Constraint> {
    let cols = &config.column_widths;
    match tab {
        Tab::FilesAndFolders => vec![
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(min(cols.name)),
            Constraint::Length(6),
            Constraint::Min(20),
        ],
        _ => vec![
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(min(cols.name)),
            Constraint::Length(min(cols.current_branch_name)),
            Constraint::Length(min(cols.status)),
            Constraint::Min(min(cols.github_repo_name)),
        ],
    }
}

/// The minimum width of a column as a `u16`.
fn min(width: ColumnWidth) -> u16 {
    width.min as u16
}

/// The header row for `tab`.
fn header_row(tab: Tab) -> Row<'static> {
    let titles: Vec<&str> = match tab {
        Tab::FilesAndFolders => vec!["", "", "Name", "Type", "Path"],
        _ => vec!["", "", "Name", "Branch", "Status", "GitHub"],
    };
    Row::new(titles.into_iter().map(Cell::from).collect::<Vec<_>>())
        .style(header_style())
}

/// Builds the data row for one entry.
fn row_for<'a>(repo: &Repo, view: &TableView) -> Row<'a> {
    let marker = marker_cell(repo, view.icons);
    let fav = fav_cell(repo, view.icons);
    let name = Cell::from(repo.display_name());
    match view.tab {
        Tab::FilesAndFolders => Row::new(vec![
            marker,
            fav,
            name,
            Cell::from(kind_label(repo.kind)),
            Cell::from(Span::styled(
                repo.path.to_string_lossy().into_owned(),
                Style::default().fg(DIM),
            )),
        ]),
        _ => {
            let info = effective_info(repo, view.example_mode);
            let branch_width =
                min(view.config.column_widths.current_branch_name) as usize;
            Row::new(vec![
                marker,
                fav,
                name,
                Cell::from(truncate(&branch_text(info), branch_width)),
                status_cell(repo, info, view.icons, view.config),
                Cell::from(github_text(info)),
            ])
        }
    }
}

/// The marker cell: a red warning glyph when the path is missing, else blank.
fn marker_cell<'a>(repo: &Repo, icons: &IconSet) -> Cell<'a> {
    if repo.path_exists() {
        Cell::from(" ")
    } else {
        Cell::from(Span::styled(
            icons.missing.to_string(),
            Style::default().fg(DANGER).add_modifier(Modifier::BOLD),
        ))
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

/// The status cell: dim while loading, green when clean, yellow when there are
/// uncommitted changes, default otherwise (e.g. only ahead/behind).
fn status_cell<'a>(
    repo: &Repo,
    info: Option<&GitInfo>,
    icons: &IconSet,
    config: &Config,
) -> Cell<'a> {
    let width = min(config.column_widths.status) as usize;
    let Some(info) = info else {
        return Cell::from(Span::styled("…", Style::default().fg(DIM)));
    };
    if !info.valid && !repo.path_exists() {
        return Cell::from(Span::styled("-", Style::default().fg(DIM)));
    }
    let text = truncate(&status_text(info, icons), width);
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
fn github_text<'a>(info: Option<&GitInfo>) -> Text<'a> {
    let value = info
        .and_then(|info| info.github_repo_name.clone())
        .unwrap_or_else(|| "-".to_string());
    Text::from(value)
}

/// The kind label for the Files and Folders tab.
fn kind_label(kind: RepoKind) -> &'static str {
    match kind {
        RepoKind::Git => "git",
        RepoKind::Folder => "folder",
        RepoKind::File => "file",
    }
}
