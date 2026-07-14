//! The main table's swappable column sets, plus the bar and totals row below.
//!
//! `c` cycles the sets. `Standard` is what hop always showed and costs nothing
//! to render; `Code` and `Activity` read the statistics the background workers
//! compute. Only outside `Standard` do the totals row and the tinted column bar
//! appear - so a user who never opens them never pays for a source walk, and
//! the list keeps every row it has today.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::domain::filter::{Tab, TabKind};
use crate::domain::sort::SortMode;
use crate::domain::stats::{
    CodeEntry, GitStats, Totals, format_age, format_bytes, format_count,
    top_language,
};
use crate::tui::skin::Colors;

/// The label in front of the set names in the bar.
const BAR_LABEL: &str = "Columns";
/// Separator between the set names.
const BAR_SEPARATOR: &str = " \u{203a} ";
/// Rows the bar occupies: a blank line, the tinted line, a blank line.
pub const BAR_ROWS: u16 = 3;
/// Rows the totals block occupies: a rule and the totals line.
pub const TOTALS_ROWS: u16 = 2;

/// The content height below which the bar and the totals row are dropped, so a
/// short terminal keeps a usable list instead of a decorated empty one.
const MIN_CONTENT_ROWS: u16 = 12;

/// Which columns the main table shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColumnSet {
    /// Branch, status, GitHub name and the ZIP-backup date, as hop always was.
    #[default]
    Standard,
    /// Lines of code, top language, source files and size on disk.
    Code,
    /// Commits, last commit, authors, branches and the hop usage counters.
    Activity,
}

impl ColumnSet {
    /// The sets available on `tab`, in cycle order.
    ///
    /// The files tabs omit `Activity`: commits and branches say nothing about a
    /// folder or a file, and an empty column is worse than no column. The git
    /// tabs (active and archive) keep all three.
    pub fn all(tab: Tab) -> &'static [ColumnSet] {
        match tab.kind() {
            TabKind::Files => &[ColumnSet::Standard, ColumnSet::Code],
            TabKind::Git => {
                &[ColumnSet::Standard, ColumnSet::Code, ColumnSet::Activity]
            }
        }
    }

    /// The next set in `tab`'s cycle.
    pub fn next(self, tab: Tab) -> Self {
        let sets = ColumnSet::all(tab);
        let index = sets.iter().position(|set| *set == self).unwrap_or(0);
        sets[(index + 1) % sets.len()]
    }

    /// The set itself when `tab` offers it, else the standard one. Guards the
    /// persisted choice against a tab that does not have it.
    pub fn available_on(self, tab: Tab) -> Self {
        if ColumnSet::all(tab).contains(&self) {
            self
        } else {
            ColumnSet::Standard
        }
    }

    /// The name shown in the bar.
    pub fn title(self) -> &'static str {
        match self {
            ColumnSet::Standard => "Standard",
            ColumnSet::Code => "Code",
            ColumnSet::Activity => "Activity",
        }
    }

    /// The stable persisted value.
    pub fn as_key(self) -> &'static str {
        match self {
            ColumnSet::Standard => "standard",
            ColumnSet::Code => "code",
            ColumnSet::Activity => "activity",
        }
    }

    /// Parses a persisted [`as_key`](Self::as_key) value, defaulting to
    /// `Standard`.
    pub fn from_key(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "code" => ColumnSet::Code,
            "activity" => ColumnSet::Activity,
            _ => ColumnSet::Standard,
        }
    }

    /// Whether the set reads the background-computed statistics. The bar and
    /// the totals row appear exactly for these.
    pub fn is_statistics(self) -> bool {
        self != ColumnSet::Standard
    }

    /// Whether this set needs the code worker (lines of code and size).
    pub fn needs_code_stats(self) -> bool {
        self == ColumnSet::Code
    }

    /// Whether this set needs the git-history worker.
    pub fn needs_git_stats(self) -> bool {
        self == ColumnSet::Activity
    }

    /// The sort modes offered for this set's columns, after the four general
    /// modes. A user can only sort by a number that is on screen.
    pub fn sort_modes(self) -> &'static [SortMode] {
        match self {
            ColumnSet::Standard => &[],
            ColumnSet::Code => {
                &[SortMode::Loc, SortMode::Size, SortMode::Files]
            }
            ColumnSet::Activity => &[
                SortMode::Commits,
                SortMode::LastCommit,
                SortMode::Authors,
                SortMode::Branches,
                SortMode::Opens,
                SortMode::LastUsed,
            ],
        }
    }
}

/// One statistics column: what it shows and how wide it needs to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatColumn {
    /// Lines of code.
    Loc,
    /// The language with the most code.
    Language,
    /// Recognised source files.
    Files,
    /// Bytes on disk.
    Size,
    /// Commits reachable from `HEAD`.
    Commits,
    /// Age of the most recent commit.
    LastCommit,
    /// Distinct commit authors.
    Authors,
    /// Local branches.
    Branches,
    /// How often the entry was opened from hop.
    Opens,
    /// Age of the last open from hop.
    LastUsed,
}

impl StatColumn {
    /// The header label.
    pub fn title(self) -> &'static str {
        match self {
            StatColumn::Loc => "LOC",
            StatColumn::Language => "Language",
            StatColumn::Files => "Files",
            StatColumn::Size => "Size",
            StatColumn::Commits => "Commits",
            StatColumn::LastCommit => "Last",
            StatColumn::Authors => "Authors",
            StatColumn::Branches => "Branches",
            StatColumn::Opens => "Opens",
            StatColumn::LastUsed => "Last used",
        }
    }

    /// The column width in cells, wide enough for the header and a big number.
    pub fn width(self) -> u16 {
        match self {
            StatColumn::Loc => 9,
            StatColumn::Language => 10,
            StatColumn::Files => 6,
            StatColumn::Size => 9,
            StatColumn::Commits => 8,
            StatColumn::LastCommit => 6,
            StatColumn::Authors => 8,
            StatColumn::Branches => 8,
            StatColumn::Opens => 6,
            StatColumn::LastUsed => 10,
        }
    }

    /// Whether the column's numbers are right-aligned (every count and size).
    pub fn is_numeric(self) -> bool {
        self != StatColumn::Language
    }
}

/// The statistics columns of `set`, in display order. Empty for `Standard`,
/// which keeps hop's original columns.
pub fn stat_columns(set: ColumnSet) -> &'static [StatColumn] {
    match set {
        ColumnSet::Standard => &[],
        ColumnSet::Code => &[
            StatColumn::Loc,
            StatColumn::Language,
            StatColumn::Files,
            StatColumn::Size,
        ],
        ColumnSet::Activity => &[
            StatColumn::Commits,
            StatColumn::LastCommit,
            StatColumn::Authors,
            StatColumn::Branches,
            StatColumn::Opens,
            StatColumn::LastUsed,
        ],
    }
}

/// What a statistics cell shows for one entry.
#[derive(Debug, Clone, Copy)]
pub struct CellSource<'a> {
    /// The entry's code and size statistics, when computed.
    pub code: Option<&'a CodeEntry>,
    /// The entry's history statistics, when computed.
    pub git: Option<&'a GitStats>,
    /// How often the entry was opened from hop.
    pub open_count: u64,
    /// When the entry was last opened, in unix seconds.
    pub last_used: Option<i64>,
    /// Whether a worker is currently computing this entry.
    pub pending: bool,
    /// The reference time for ages, in unix seconds.
    pub now: i64,
}

/// Shown where a value does not exist (a folder has no commits).
const NO_VALUE: &str = "-";

/// The text for `column`, or `None` when a worker still owes the value and the
/// caller should draw a spinner instead.
pub fn cell_text(column: StatColumn, source: CellSource) -> Option<String> {
    let text = match column {
        StatColumn::Loc => {
            return numeric(source, |c, _| {
                format_count(c.code.total().code as u64)
            });
        }
        StatColumn::Files => {
            return numeric(source, |c, _| {
                format_count(c.code.total().files as u64)
            });
        }
        StatColumn::Size => {
            return numeric(source, |c, _| format_bytes(c.disk.total_bytes));
        }
        StatColumn::Language => {
            return numeric(source, |c, _| {
                top_language(&c.code).unwrap_or(NO_VALUE).to_string()
            });
        }
        StatColumn::Commits => {
            return history(source, |g, _| format_count(g.commits));
        }
        StatColumn::Authors => {
            return history(source, |g, _| format_count(g.contributors));
        }
        StatColumn::Branches => {
            return history(source, |g, _| format_count(g.branches));
        }
        StatColumn::LastCommit => {
            return history(source, |g, now| match g.last_commit {
                Some(at) => format_age(at, now),
                None => NO_VALUE.to_string(),
            });
        }
        StatColumn::Opens => format_count(source.open_count),
        StatColumn::LastUsed => match source.last_used {
            Some(at) => format_age(at, source.now),
            None => NO_VALUE.to_string(),
        },
    };
    Some(text)
}

/// A cell fed by the code worker: pending until it has reported.
fn numeric(
    source: CellSource,
    render: impl Fn(&CodeEntry, i64) -> String,
) -> Option<String> {
    match source.code {
        Some(code) => Some(render(code, source.now)),
        None if source.pending => None,
        None => Some(NO_VALUE.to_string()),
    }
}

/// A cell fed by the git worker: pending until it has reported.
fn history(
    source: CellSource,
    render: impl Fn(&GitStats, i64) -> String,
) -> Option<String> {
    match source.git {
        Some(git) => Some(render(git, source.now)),
        None if source.pending => None,
        None => Some(NO_VALUE.to_string()),
    }
}

/// How many rows the bar and totals block need below the list, given the
/// content height available. Returns `0` when the terminal is too short to
/// spare them.
pub fn footer_rows(set: ColumnSet, content_height: u16) -> u16 {
    if !set.is_statistics() || content_height < MIN_CONTENT_ROWS {
        return 0;
    }
    BAR_ROWS + TOTALS_ROWS
}

/// Renders the totals block (a rule and the summed numbers) and, below it, the
/// tinted column bar naming every set with the active one accented.
pub fn render_footer(
    frame: &mut ratatui::Frame,
    area: Rect,
    (set, tab): (ColumnSet, Tab),
    (totals, colors): (Totals, &Colors),
) {
    if area.height < BAR_ROWS + TOTALS_ROWS {
        return;
    }
    let rule = Rect {
        height: TOTALS_ROWS,
        ..area
    };
    frame
        .render_widget(Paragraph::new(totals_lines(set, totals, colors)), rule);

    // The bar sits on the middle of its three rows; the blank rows above and
    // below stay untinted, as does the one-cell margin on each side.
    let bar = Rect {
        x: area.x + 1,
        y: area.y + TOTALS_ROWS + 1,
        width: area.width.saturating_sub(2),
        height: 1,
    };
    frame.render_widget(
        Block::default().style(Style::default().bg(colors.surface_bg)),
        bar,
    );
    frame.render_widget(Paragraph::new(bar_line(set, tab, colors)), bar);
}

/// The rule and the totals line, column-aligned under the table's numbers is
/// impossible without the table's widths, so the numbers are labelled instead.
fn totals_lines(
    set: ColumnSet,
    totals: Totals,
    colors: &Colors,
) -> Vec<Line<'static>> {
    let dim = Style::default().fg(colors.dim);
    let value = Style::default().fg(colors.foreground);
    let mut spans = vec![
        Span::styled(
            format!("  {} projects", totals.entries),
            Style::default().fg(colors.muted),
        ),
        Span::styled("   ", dim),
    ];
    if set == ColumnSet::Code {
        spans.extend([
            Span::styled(format_count(totals.code as u64), value),
            Span::styled(" lines   ", dim),
            Span::styled(format_count(totals.files as u64), value),
            Span::styled(" files   ", dim),
            Span::styled(format_bytes(totals.bytes), value),
        ]);
    }
    vec![Line::from(Span::styled("", dim)), Line::from(spans)]
}

/// `Columns  Standard › Code › Activity`, the active set accented and bold.
fn bar_line(set: ColumnSet, tab: Tab, colors: &Colors) -> Line<'static> {
    let mut spans = vec![Span::styled(
        format!(" {BAR_LABEL}  "),
        Style::default().fg(colors.muted),
    )];
    for (index, candidate) in ColumnSet::all(tab).iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(
                BAR_SEPARATOR,
                Style::default().fg(colors.dim),
            ));
        }
        let style = if *candidate == set {
            Style::default()
                .fg(colors.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(colors.dim)
        };
        spans.push(Span::styled(candidate.title().to_string(), style));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_files_tabs_skip_the_activity_set() {
        // Both a files active and files archive view omit Activity.
        assert_eq!(ColumnSet::all(Tab::FilesActive).len(), 2);
        assert_eq!(ColumnSet::all(Tab::FilesArchive).len(), 2);
        // Both git views keep all three.
        assert_eq!(ColumnSet::all(Tab::GitActive).len(), 3);
        assert_eq!(ColumnSet::all(Tab::GitArchive).len(), 3);
        // Cycling on a files tab returns to Standard after Code.
        let set = ColumnSet::Standard.next(Tab::FilesActive);
        assert_eq!(set, ColumnSet::Code);
        assert_eq!(set.next(Tab::FilesActive), ColumnSet::Standard);
    }

    #[test]
    fn cycling_the_git_tab_visits_every_set() {
        let tab = Tab::GitActive;
        let mut set = ColumnSet::Standard;
        set = set.next(tab);
        assert_eq!(set, ColumnSet::Code);
        set = set.next(tab);
        assert_eq!(set, ColumnSet::Activity);
        assert_eq!(set.next(tab), ColumnSet::Standard);
    }

    #[test]
    fn a_set_the_tab_does_not_offer_falls_back_to_standard() {
        // Persisted "activity", then the user opens a files tab.
        let set = ColumnSet::Activity.available_on(Tab::FilesActive);
        assert_eq!(set, ColumnSet::Standard);
        assert_eq!(
            ColumnSet::Code.available_on(Tab::FilesActive),
            ColumnSet::Code
        );
    }

    #[test]
    fn keys_round_trip_and_unknown_falls_back() {
        for set in [ColumnSet::Standard, ColumnSet::Code, ColumnSet::Activity] {
            assert_eq!(ColumnSet::from_key(set.as_key()), set);
        }
        assert_eq!(ColumnSet::from_key("bogus"), ColumnSet::Standard);
    }

    #[test]
    fn each_set_declares_exactly_the_worker_it_needs() {
        // Switching to Activity must not trigger a source walk.
        assert!(!ColumnSet::Activity.needs_code_stats());
        assert!(ColumnSet::Activity.needs_git_stats());
        assert!(ColumnSet::Code.needs_code_stats());
        assert!(!ColumnSet::Code.needs_git_stats());
        assert!(!ColumnSet::Standard.needs_code_stats());
        assert!(!ColumnSet::Standard.needs_git_stats());
    }

    #[test]
    fn the_footer_disappears_in_standard_and_on_a_short_terminal() {
        assert_eq!(footer_rows(ColumnSet::Standard, 40), 0);
        assert_eq!(footer_rows(ColumnSet::Code, 40), BAR_ROWS + TOTALS_ROWS);
        // Below the minimum the list keeps all its rows.
        assert_eq!(footer_rows(ColumnSet::Code, MIN_CONTENT_ROWS - 1), 0);
    }

    fn source(pending: bool) -> CellSource<'static> {
        CellSource {
            code: None,
            git: None,
            open_count: 42,
            last_used: None,
            pending,
            now: 1_000_000,
        }
    }

    #[test]
    fn a_cell_awaiting_its_worker_has_no_text_so_a_spinner_can_show() {
        assert_eq!(cell_text(StatColumn::Loc, source(true)), None);
        // Once no worker is coming, the cell says so instead of spinning.
        assert_eq!(
            cell_text(StatColumn::Loc, source(false)),
            Some("-".to_string())
        );
        // Usage counters are always known, even while a worker runs.
        assert_eq!(
            cell_text(StatColumn::Opens, source(true)),
            Some("42".to_string())
        );
    }

    #[test]
    fn only_the_statistics_sets_offer_column_sort_modes() {
        assert!(ColumnSet::Standard.sort_modes().is_empty());
        assert!(ColumnSet::Code.sort_modes().contains(&SortMode::Loc));
        assert!(
            ColumnSet::Activity
                .sort_modes()
                .contains(&SortMode::Commits)
        );
    }
}
