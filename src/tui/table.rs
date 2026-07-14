//! Renders the repository list as a table, with columns depending on the tab.
//!
//! The git tabs show name, branch, status and GitHub name; the Files and
//! Folders tab shows name, type and path. A leading marker column flags entries
//! whose path no longer exists, followed by the favourite star.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use chrono::{DateTime, Local};
use ratada::text::truncate;
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Row, Table, TableState};
use unicode_width::UnicodeWidthStr;

use crate::config::{ColumnWidth, Config};
use crate::domain::filter::{Tab, TabKind};
use crate::domain::repo::{GitInfo, Repo, is_dir_target};
use crate::domain::sections::UNGROUPED;
use crate::domain::stats::{CodeEntry, GitStats};
use crate::theme::Skin;
use crate::tui::columns::{
    CellSource, ColumnSet, StatColumn, cell_text, stat_columns,
};
use crate::tui::git_columns::{
    branch_text, effective_info, git_marker_errored, github_text,
    status_display, zip_date_text,
};
use crate::tui::presentation::{
    IconSet, highlight_name, slug_style, status_span,
};
use crate::tui::skin::Colors;

/// The styling context for a table render, bundled to keep the parameter count
/// low.
pub struct TableView<'a> {
    /// The active tab (decides the columns).
    pub tab: Tab,
    /// The resolved settings (column widths).
    pub config: &'a Config,
    /// The active theme, for the scrollbar.
    pub skin: &'a Skin,
    /// The colour roles resolved from the active theme.
    pub colors: &'a Colors,
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
    /// Whether to show the slug in its own dim-italic column after the name.
    pub show_slugs: bool,
    /// Whether to show a leading Section column (the flat view, when grouping is
    /// toggled off, so an entry's group is still visible without header bars).
    pub show_section: bool,
    /// The active fuzzy query, to highlight matched characters in the name.
    pub query: Option<&'a str>,
    /// Last ZIP-backup time per repo path, for the "ZIP Backup" column.
    pub zip_backups: &'a HashMap<PathBuf, DateTime<Local>>,
    /// The scroll offset carried across frames, so moving the cursor up while it
    /// is still visible does not scroll the list.
    pub offset: &'a std::cell::Cell<usize>,
    /// Which columns to show.
    pub columns: ColumnSet,
    /// Cached code and size statistics, keyed by entry path.
    pub code: &'a HashMap<PathBuf, CodeEntry>,
    /// Cached history statistics, keyed by entry path.
    pub git: &'a HashMap<PathBuf, GitStats>,
    /// Paths a statistics worker has not reported yet (drive the cell spinner).
    pub computing: &'a HashSet<PathBuf>,
    /// The reference time for ages, in unix seconds.
    pub now: i64,
}

impl TableView<'_> {
    /// What the statistics cells of `repo` read from.
    fn cell_source<'b>(&'b self, repo: &Repo) -> CellSource<'b> {
        CellSource {
            code: self.code.get(&repo.path),
            git: self.git.get(&repo.path),
            open_count: repo.open_count,
            last_used: repo.last_used,
            pending: self.computing.contains(&repo.path),
            now: self.now,
        }
    }

    /// The rendered text of one statistics cell: the value, a spinner while its
    /// worker runs, or a dash when nothing will ever fill it.
    fn stat_text(&self, repo: &Repo, column: StatColumn) -> String {
        match cell_text(column, self.cell_source(repo)) {
            Some(text) => text,
            None => self.spinner_frame().to_string(),
        }
    }

    /// The current spinner glyph, or a dash outside a run.
    fn spinner_frame(&self) -> &str {
        self.spinner.map_or("-", |(_, glyph)| glyph)
    }
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

/// The name-cell spans: the name, fuzzy-highlighted when a query is active. The
/// slug is its own column (see `row_for`), not appended here.
fn name_cell_spans(repo: &Repo, view: &TableView) -> Vec<Span<'static>> {
    let name = repo.display_name();
    match view.query {
        Some(query) if !query.trim().is_empty() => {
            highlight_name(&name, query, view.colors)
        }
        _ => vec![Span::raw(name)],
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
                built.style(Style::default().bg(view.colors.multi_select_bg))
            } else {
                built
            }
        })
        .collect();
    let table = Table::new(rows, widths(repos, view, table_area.width))
        .header(header_row(view))
        .column_spacing(1)
        .row_highlight_style(view.colors.selection_style());
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
        ratada::scroll::render_scrollbar(
            frame,
            bar_area,
            view.skin,
            ratada::nav::ScrollView {
                total: repos.len(),
                offset,
                viewport,
            },
        );
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
    let name = sized(content_width(repos, |r| r.display_name()), 4, cols.name);
    // The lead: a 2-cell selection marker (only while a selection is active),
    // the marker + fav cells, and the leading Section column when grouping is
    // toggled off. The Name column and the optional Slug column follow.
    let mut cells: Vec<Constraint> = if view.has_selection {
        vec![
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(1),
        ]
    } else {
        vec![Constraint::Length(1), Constraint::Length(1)]
    };
    if view.show_section {
        cells.push(Constraint::Length(section_width(repos)));
    }
    cells.push(Constraint::Length(name));
    if view.show_slugs {
        cells.push(Constraint::Length(slug_width(repos)));
    }
    if view.columns.is_statistics() {
        cells.extend(
            stat_columns(view.columns)
                .iter()
                .map(|column| Constraint::Length(column.width())),
        );
        // Leave the remainder unused rather than stretching a number column.
        cells.push(Constraint::Min(0));
        return cells;
    }
    match view.tab.kind() {
        TabKind::Files => {
            cells.push(Constraint::Length(6));
            cells.push(Constraint::Min(20));
        }
        TabKind::Git => {
            let branch = sized(
                content_width(repos, |r| {
                    branch_text(effective_info(r, view.example_mode))
                }),
                6,
                cols.current_branch_name,
            );
            let status = sized(
                content_width(repos, |r| {
                    status_display(
                        effective_info(r, view.example_mode),
                        view.icons,
                    )
                }),
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
                content_width(repos, |r| {
                    zip_date_text(r, view.icons, view.zip_backups)
                }),
                "ZIP Backup".len(),
                cols.zip_backup,
            );
            // GitHub is the column that yields: it keeps its content width when
            // there is room, but shrinks so the rigid columns (above all Name)
            // are never squeezed. The optional Section and Slug columns count as
            // part of the fixed lead.
            let section = if view.show_section {
                section_width(repos)
            } else {
                0
            };
            let slug = if view.show_slugs {
                slug_width(repos)
            } else {
                0
            };
            let lead_cells: u16 = (if view.has_selection { 3 } else { 2 })
                + section_cells(view)
                + slug_cells(view);
            let lead_width: u16 =
                (if view.has_selection { 4 } else { 2 }) + section + slug;
            let spacing = (lead_cells + 5).saturating_sub(1); // column_spacing 1
            let fixed = lead_width + name + branch + status + zip + spacing;
            let github = github_width(github_desired, fixed, available);
            cells.push(Constraint::Length(branch));
            cells.push(Constraint::Length(status));
            cells.push(Constraint::Length(github));
            cells.push(Constraint::Length(zip));
        }
    }
    cells
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

/// The header label and floor width of the leading Section column.
const SECTION_HEADER: &str = "Section";
/// The upper bound of the Section column, so a long name cannot dominate.
const SECTION_MAX: usize = 24;

/// The Section column width, sized to the widest section label (floored at the
/// header, capped at [`SECTION_MAX`]).
fn section_width(repos: &[&Repo]) -> u16 {
    content_width(repos, section_label)
        .max(SECTION_HEADER.len())
        .min(SECTION_MAX) as u16
}

/// Whether the layout adds a Section column cell (0 or 1), for lead-cell counts.
fn section_cells(view: &TableView) -> u16 {
    u16::from(view.show_section)
}

/// The section label for `repo`: its section name, or the Ungrouped label.
fn section_label(repo: &Repo) -> String {
    repo.section
        .clone()
        .unwrap_or_else(|| UNGROUPED.to_string())
}

/// The header label and floor width of the Slug column.
const SLUG_HEADER: &str = "Slug";
/// The upper bound of the Slug column, so a long slug cannot dominate.
const SLUG_MAX: usize = 20;

/// The Slug column width, sized to the widest slug (floored at the header,
/// capped at [`SLUG_MAX`]). Only meaningful while `show_slugs` is on.
fn slug_width(repos: &[&Repo]) -> u16 {
    content_width(repos, slug_text)
        .max(SLUG_HEADER.len())
        .min(SLUG_MAX) as u16
}

/// Whether the layout adds a Slug column cell (0 or 1), for lead-cell counts.
fn slug_cells(view: &TableView) -> u16 {
    u16::from(view.show_slugs)
}

/// The slug text for `repo` (empty when it has none).
fn slug_text(repo: &Repo) -> String {
    repo.slug.clone().unwrap_or_default()
}

/// The header row for `tab` (a leading blank cell for the selection column when
/// a selection is active, and a Section header when the flat section column is
/// shown).
fn header_row(view: &TableView) -> Row<'static> {
    let titles: Vec<String> = if view.columns.is_statistics() {
        let mut titles = vec![String::new(), String::new(), "Name".to_string()];
        titles.extend(
            stat_columns(view.columns)
                .iter()
                .map(|column| column.title().to_string()),
        );
        titles.push(String::new());
        titles
    } else {
        match view.tab.kind() {
            TabKind::Files => vec!["", "", "Name", "Type", "Path"],
            TabKind::Git => {
                vec!["", "", "Name", "Branch", "Status", "GitHub", "ZIP Backup"]
            }
        }
        .into_iter()
        .map(str::to_string)
        .collect()
    };
    let mut cells: Vec<Cell> = Vec::new();
    if view.has_selection {
        cells.push(Cell::from(""));
    }
    let mut titles = titles;
    // Name sits at index 2 (after the marker + fav blanks). The Slug header
    // goes right after it, the Section header right before it; insert Slug first
    // so the Section insert keeps Name at index 2.
    if view.show_slugs {
        titles.insert(3, SLUG_HEADER.to_string());
    }
    if view.show_section {
        titles.insert(2, SECTION_HEADER.to_string());
    }
    cells.extend(titles.into_iter().map(Cell::from));
    Row::new(cells).style(view.colors.header_style())
}

/// Builds the data row for one entry; `selected` drives the marker column.
fn row_for<'a>(repo: &Repo, view: &TableView, selected: bool) -> Row<'a> {
    let mut cells: Vec<Cell> = Vec::new();
    if view.has_selection {
        cells.push(selection_cell(selected, view.colors));
    }
    cells.push(marker_cell(repo, view));
    cells.push(fav_cell(repo, view));
    if view.show_section {
        cells.push(Cell::from(Span::styled(
            truncate(&section_label(repo), SECTION_MAX),
            Style::default().fg(view.colors.dim),
        )));
    }
    cells.push(Cell::from(Line::from(name_cell_spans(repo, view))));
    if view.show_slugs {
        cells.push(Cell::from(Span::styled(
            slug_text(repo),
            slug_style(view.colors),
        )));
    }
    if view.columns.is_statistics() {
        for column in stat_columns(view.columns) {
            let text = view.stat_text(repo, *column);
            let cell = if column.is_numeric() {
                Cell::from(Line::from(text).right_aligned())
            } else {
                Cell::from(text)
            };
            cells.push(cell);
        }
        cells.push(Cell::from(""));
        return Row::new(cells);
    }
    match view.tab.kind() {
        TabKind::Files => {
            cells.push(Cell::from(type_label(repo)));
            cells.push(Cell::from(Span::styled(
                repo.path.to_string_lossy().into_owned(),
                Style::default().fg(view.colors.dim),
            )));
        }
        TabKind::Git => {
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
                zip_date_text(repo, view.icons, view.zip_backups),
                Style::default().fg(view.colors.dim),
            )));
        }
    }
    Row::new(cells)
}

/// The selection-marker cell: a bold accent `▸` when selected, else blank.
fn selection_cell<'a>(selected: bool, colors: &Colors) -> Cell<'a> {
    let symbol = if selected { "\u{25b8} " } else { "  " };
    Cell::from(Span::styled(
        symbol,
        Style::default()
            .fg(colors.accent)
            .add_modifier(Modifier::BOLD),
    ))
}

/// The marker cell: a red warning glyph when the entry has a path error, else
/// blank. A git entry is flagged from its gathered git info (missing or invalid
/// repository); a file/folder entry only once the on-demand existence check
/// flagged its path.
fn marker_cell<'a>(repo: &Repo, view: &TableView) -> Cell<'a> {
    let errored = git_marker_errored(repo, view.example_mode)
        || view.missing.contains(&repo.path);
    if errored {
        Cell::from(Span::styled(
            view.icons.missing.to_string(),
            Style::default()
                .fg(view.colors.danger)
                .add_modifier(Modifier::BOLD),
        ))
    } else {
        Cell::from(" ")
    }
}

/// The favourite cell: a star when favourited, else blank.
fn fav_cell<'a>(repo: &Repo, view: &TableView) -> Cell<'a> {
    if repo.fav {
        Cell::from(Span::styled(
            view.icons.favourite.to_string(),
            Style::default().fg(view.colors.favourite),
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
            Style::default().fg(view.colors.accent),
        ));
    }
    // Otherwise the shared coloured status span (loading / clean / changes).
    Cell::from(status_span(info, view.icons, view.colors))
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
