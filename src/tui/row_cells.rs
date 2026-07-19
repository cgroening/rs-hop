//! The per-entry cell content both list renderers share: the leading glyphs,
//! the Files type label and the statistics cells.
//!
//! These are the cells that do not depend on the tab's kind, so
//! [`table`](crate::tui::table) and
//! [`sections_view`](crate::tui::sections_view) fill them identically. The
//! git-only column text lives in [`git_columns`](crate::tui::git_columns)
//! instead.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use crate::domain::repo::{Repo, is_dir_target};
use crate::domain::stats::{CodeEntry, GitStats};
use crate::tui::columns::{CellSource, StatColumn, cell_text};
use crate::tui::git_columns::git_marker_errored;
use crate::tui::presentation::IconSet;
use crate::tui::skin::Colors;

/// What the shared entry glyphs need from the calling view.
pub struct GlyphContext<'a> {
    /// Whether to judge git entries by their example info instead of live data.
    pub example_mode: bool,
    /// Paths flagged missing by the on-demand existence check.
    pub missing: &'a HashSet<PathBuf>,
    /// The glyph set.
    pub icons: &'a IconSet,
    /// The colour roles resolved from the active theme.
    pub colors: &'a Colors,
}

/// Returns the error marker glyph (red) when the entry has a path or repository
/// error, else a blank.
///
/// A git entry is flagged from its gathered git info; a file or folder entry
/// only once the on-demand existence check flagged its path.
pub fn marker_span(repo: &Repo, ctx: &GlyphContext) -> Span<'static> {
    let is_errored = git_marker_errored(repo, ctx.example_mode)
        || ctx.missing.contains(&repo.path);
    if !is_errored {
        return Span::raw(" ");
    }
    Span::styled(
        ctx.icons.missing.to_string(),
        Style::default()
            .fg(ctx.colors.danger)
            .add_modifier(Modifier::BOLD),
    )
}

/// Returns the favourite star glyph when the entry is favourited, else a blank.
pub fn fav_span(repo: &Repo, ctx: &GlyphContext) -> Span<'static> {
    if !repo.fav {
        return Span::raw(" ");
    }
    Span::styled(
        ctx.icons.favourite.to_string(),
        Style::default().fg(ctx.colors.favourite),
    )
}

/// Returns the detected type label for an entry on the Files tab.
pub fn type_label(repo: &Repo) -> &'static str {
    if is_dir_target(&repo.path) {
        "folder"
    } else {
        "file"
    }
}

/// What the statistics cells read from, plus the spinner that stands in while a
/// worker still owes a value.
pub struct StatContext<'a> {
    /// Cached code and size statistics, keyed by entry path.
    pub code: &'a HashMap<PathBuf, CodeEntry>,
    /// Cached history statistics, keyed by entry path.
    pub git: &'a HashMap<PathBuf, GitStats>,
    /// Paths a statistics worker has not reported yet.
    pub computing: &'a HashSet<PathBuf>,
    /// While a refresh runs: the still-in-flight paths and the current spinner
    /// frame glyph. `None` when no refresh is running.
    pub spinner: Option<(&'a HashSet<PathBuf>, &'a str)>,
    /// The reference time for ages, in unix seconds.
    pub now: i64,
}

impl<'a> StatContext<'a> {
    /// The rendered text of one statistics cell: the value, a spinner while its
    /// worker runs, or a dash when nothing will ever fill it.
    pub fn text(&self, repo: &Repo, column: StatColumn) -> String {
        cell_text(column, self.source(repo))
            .unwrap_or_else(|| self.spinner_glyph().to_string())
    }

    /// The current spinner glyph, or a dash outside a run.
    ///
    /// The glyph borrows from the spinner, not from `self`, so a caller may
    /// build the context on the fly and still keep the returned frame.
    pub fn spinner_glyph(&self) -> &'a str {
        self.spinner.map_or("-", |(_, glyph)| glyph)
    }

    /// Whether `repo` is still in flight in the running refresh.
    pub fn is_in_flight(&self, repo: &Repo) -> bool {
        self.spinner
            .is_some_and(|(in_flight, _)| in_flight.contains(&repo.path))
    }

    /// What the statistics cells of `repo` read from.
    fn source(&self, repo: &Repo) -> CellSource<'_> {
        CellSource {
            code: self.code.get(&repo.path),
            git: self.git.get(&repo.path),
            open_count: repo.open_count,
            last_used: repo.last_used,
            pending: self.computing.contains(&repo.path),
            now: self.now,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::Config;
    use crate::domain::repo::RepoKind;
    use crate::theme::GlyphVariant;

    /// A path-kind entry, the kind whose marker the existence check drives.
    fn repo_at(path: &str) -> Repo {
        let mut repo = Repo::new(PathBuf::from(path));
        repo.kind = RepoKind::Path;
        repo
    }

    fn icon_set() -> IconSet {
        IconSet::new(GlyphVariant::Unicode)
    }

    fn colour_roles() -> Colors {
        Colors::from_palette(&Config::default().palette())
    }

    fn context<'a>(
        missing: &'a HashSet<PathBuf>,
        icons: &'a IconSet,
        colors: &'a Colors,
    ) -> GlyphContext<'a> {
        GlyphContext {
            example_mode: false,
            missing,
            icons,
            colors,
        }
    }

    #[test]
    fn a_path_entry_is_marked_only_once_the_existence_check_flagged_it() {
        let icons = icon_set();
        let colors = colour_roles();
        let mut missing = HashSet::new();
        let repo = repo_at("/tmp/gone");

        let clean = context(&missing, &icons, &colors);
        assert_eq!(marker_span(&repo, &clean).content, " ");

        missing.insert(PathBuf::from("/tmp/gone"));
        let flagged = context(&missing, &icons, &colors);
        assert_eq!(
            marker_span(&repo, &flagged).content,
            icons.missing.to_string()
        );
    }

    #[test]
    fn only_a_favourited_entry_gets_the_star() {
        let icons = icon_set();
        let colors = colour_roles();
        let missing = HashSet::new();
        let ctx = context(&missing, &icons, &colors);

        let mut repo = repo_at("/tmp/one");
        assert_eq!(fav_span(&repo, &ctx).content, " ");

        repo.fav = true;
        assert_eq!(fav_span(&repo, &ctx).content, icons.favourite.to_string());
    }

    #[test]
    fn the_type_label_follows_the_trailing_slash_for_a_missing_path() {
        // A path that does not exist is a folder only when written with a
        // trailing separator; otherwise it is assumed to be a file.
        assert_eq!(type_label(&repo_at("/tmp/nowhere/")), "folder");
        assert_eq!(type_label(&repo_at("/tmp/nowhere")), "file");
    }

    #[test]
    fn an_unreported_statistic_shows_the_spinner_and_a_dash_outside_a_run() {
        let code = HashMap::new();
        let git = HashMap::new();
        let mut computing = HashSet::new();
        computing.insert(PathBuf::from("/tmp/one"));
        let in_flight = computing.clone();
        let repo = repo_at("/tmp/one");

        let idle = StatContext {
            code: &code,
            git: &git,
            computing: &computing,
            spinner: None,
            now: 0,
        };
        assert_eq!(idle.spinner_glyph(), "-");
        assert!(!idle.is_in_flight(&repo));

        let running = StatContext {
            spinner: Some((&in_flight, "*")),
            ..idle
        };
        assert_eq!(running.spinner_glyph(), "*");
        assert!(running.is_in_flight(&repo));
    }
}
