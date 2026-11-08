//! Sort modes for the repository list and the pure ordering they imply.
//!
//! Four modes order by what an entry *is* (name, recency, frecency, the stored
//! order); the rest order by a statistic shown in the active column set. The
//! statistics arrive asynchronously, so an entry whose number has not been
//! computed yet sorts **last in either direction** rather than counting as
//! zero - otherwise the list would reshuffle as the background workers report.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::domain::repo::Repo;
use crate::domain::stats::{CodeEntry, GitStats};

/// How the visible list is ordered.
///
/// Favourites are pinned to the top in [`SortMode::Name`], [`SortMode::
/// Frecency`] and [`SortMode::Custom`]; [`SortMode::Recent`] orders purely by
/// recency. The statistics modes do not pin favourites: they answer "which is
/// the biggest", and a pinned favourite would hide the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMode {
    /// Favourites first, then case-insensitive by display name (the default).
    #[default]
    Name,
    /// Most recently used first; never-used entries last, then by name.
    /// Favourites are not pinned.
    Recent,
    /// Frequency weighted by recency (frecency); favourites pinned on top.
    Frecency,
    /// Manual order (the stored `[[repos]]` order), favourites still on top.
    Custom,
    /// Lines of code.
    Loc,
    /// Bytes on disk.
    Size,
    /// Recognised source files.
    Files,
    /// Commits reachable from `HEAD`.
    Commits,
    /// Timestamp of the most recent commit.
    LastCommit,
    /// Distinct commit authors.
    Authors,
    /// Local branches.
    Branches,
    /// How often the entry was opened from hop.
    Opens,
    /// When the entry was last opened from hop.
    LastUsed,
}

/// Which way a sort runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortDir {
    /// Smallest, earliest or alphabetically first at the top.
    #[default]
    Asc,
    /// Largest, latest or alphabetically last at the top.
    Desc,
}

impl SortDir {
    /// The opposite direction, for re-picking the active column.
    pub fn flip(self) -> Self {
        match self {
            SortDir::Asc => SortDir::Desc,
            SortDir::Desc => SortDir::Asc,
        }
    }

    /// The stable persisted value.
    pub fn label(self) -> &'static str {
        match self {
            SortDir::Asc => "asc",
            SortDir::Desc => "desc",
        }
    }

    /// Parses a stored [`label`](Self::label) value, defaulting to `Asc`.
    pub fn from_config_value(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "desc" => SortDir::Desc,
            _ => SortDir::Asc,
        }
    }

    /// The arrow marking this direction in the sort picker.
    pub fn arrow(self) -> &'static str {
        match self {
            SortDir::Asc => "\u{2191}",
            SortDir::Desc => "\u{2193}",
        }
    }
}

impl SortMode {
    /// A short label for the header line (also the persisted value).
    pub fn label(self) -> &'static str {
        match self {
            SortMode::Name => "name",
            SortMode::Recent => "recent",
            SortMode::Frecency => "frecency",
            SortMode::Custom => "custom",
            SortMode::Loc => "loc",
            SortMode::Size => "size",
            SortMode::Files => "files",
            SortMode::Commits => "commits",
            SortMode::LastCommit => "last_commit",
            SortMode::Authors => "authors",
            SortMode::Branches => "branches",
            SortMode::Opens => "opens",
            SortMode::LastUsed => "last_used",
        }
    }

    /// The human name shown in the sort picker.
    pub fn title(self) -> &'static str {
        match self {
            SortMode::Name => "Name",
            SortMode::Recent => "Recently used",
            SortMode::Frecency => "Frecency",
            SortMode::Custom => "Custom order",
            SortMode::Loc => "Lines of code",
            SortMode::Size => "Size",
            SortMode::Files => "Files",
            SortMode::Commits => "Commits",
            SortMode::LastCommit => "Last commit",
            SortMode::Authors => "Authors",
            SortMode::Branches => "Branches",
            SortMode::Opens => "Opens",
            SortMode::LastUsed => "Last used",
        }
    }

    /// Parses a stored [`label`](Self::label) value, defaulting to `Name`.
    pub fn from_config_value(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "recent" => SortMode::Recent,
            "frecency" => SortMode::Frecency,
            "custom" => SortMode::Custom,
            "loc" => SortMode::Loc,
            "size" => SortMode::Size,
            "files" => SortMode::Files,
            "commits" => SortMode::Commits,
            "last_commit" => SortMode::LastCommit,
            "authors" => SortMode::Authors,
            "branches" => SortMode::Branches,
            "opens" => SortMode::Opens,
            "last_used" => SortMode::LastUsed,
            _ => SortMode::Name,
        }
    }

    /// Whether the mode orders by a background-computed statistic. Those modes
    /// do not pin favourites and default to descending when first picked.
    pub fn is_statistic(self) -> bool {
        !matches!(
            self,
            SortMode::Name
                | SortMode::Recent
                | SortMode::Frecency
                | SortMode::Custom
        )
    }
}

/// The computed statistics a sort may order by, borrowed from the app's caches.
#[derive(Debug, Clone, Copy)]
pub struct StatsLookup<'a> {
    /// Lines of code and size on disk, per entry path.
    pub code: &'a HashMap<PathBuf, CodeEntry>,
    /// Repository history, per entry path.
    pub git: &'a HashMap<PathBuf, GitStats>,
}

/// Everything a sort needs, grouped so the entry point keeps three parameters.
#[derive(Debug, Clone, Copy)]
pub struct SortContext<'a> {
    /// What to order by.
    pub mode: SortMode,
    /// Which way round.
    pub dir: SortDir,
    /// Whether favourites float to the top (and are then ordered among
    /// themselves by the mode); off orders every entry purely by the mode.
    pub float_favs: bool,
    /// The reference time for frecency, in unix seconds.
    pub now: i64,
    /// The statistics the column modes read.
    pub stats: StatsLookup<'a>,
}

/// Sorts `indices` (positions into `repos`) according to `ctx`, leaving the
/// entries themselves untouched. Used by the view, which keeps service indices.
pub fn sort_indices(repos: &[Repo], indices: &mut [usize], ctx: &SortContext) {
    indices.sort_by(|a, b| compare(&repos[*a], &repos[*b], ctx));
}

/// Compares two entries under `ctx`. Favourites float to the top in every mode
/// when `float_favs` is on (and are then ordered among themselves by the mode);
/// the float sits outside the direction reversal, so favourites stay on top in
/// both directions.
fn compare(a: &Repo, b: &Repo, ctx: &SortContext) -> std::cmp::Ordering {
    let base = if ctx.mode.is_statistic() {
        compare_statistic(a, b, ctx)
    } else {
        apply_dir(compare_identity(a, b, ctx), ctx.dir)
    };
    if ctx.float_favs {
        b.fav.cmp(&a.fav).then(base)
    } else {
        base
    }
}

/// Applies the sort direction to an ascending-oriented ordering.
fn apply_dir(ordering: std::cmp::Ordering, dir: SortDir) -> std::cmp::Ordering {
    match dir {
        SortDir::Asc => ordering,
        SortDir::Desc => ordering.reverse(),
    }
}

/// Compares by what an entry is: name, recency, frecency or the stored order.
/// Favourite floating is applied by [`compare`], not here.
fn compare_identity(
    a: &Repo,
    b: &Repo,
    ctx: &SortContext,
) -> std::cmp::Ordering {
    match ctx.mode {
        // Case-insensitive by name.
        SortMode::Name => name_cmp(a, b),
        // Recency, name as tiebreak.
        SortMode::Recent => recency_key(b)
            .cmp(&recency_key(a))
            .then_with(|| name_cmp(a, b)),
        // Highest frecency, name as tiebreak.
        SortMode::Frecency => frecency(b, ctx.now)
            .partial_cmp(&frecency(a, ctx.now))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| name_cmp(a, b)),
        // Stable on the stored index order.
        _ => std::cmp::Ordering::Equal,
    }
}

/// Compares by a computed statistic. An entry whose value is not (yet) known
/// sorts last in **both** directions, so a running background pass does not
/// make the list jump; the direction only orders the entries that do have one.
fn compare_statistic(
    a: &Repo,
    b: &Repo,
    ctx: &SortContext,
) -> std::cmp::Ordering {
    match (statistic(a, ctx), statistic(b, ctx)) {
        (None, None) => name_cmp(a, b),
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(left), Some(right)) => {
            let ordering = match ctx.dir {
                SortDir::Asc => left.cmp(&right),
                SortDir::Desc => right.cmp(&left),
            };
            ordering.then_with(|| name_cmp(a, b))
        }
    }
}

/// The value `ctx.mode` orders by, or `None` when it is not known for `repo`.
fn statistic(repo: &Repo, ctx: &SortContext) -> Option<i64> {
    let code = || ctx.stats.code.get(&repo.path);
    let git = || ctx.stats.git.get(&repo.path);
    match ctx.mode {
        SortMode::Loc => code().map(|e| e.code.total().code as i64),
        SortMode::Files => code().map(|e| e.code.total().files as i64),
        SortMode::Size => code().map(|e| e.disk.total_bytes as i64),
        SortMode::Commits => git().map(|g| g.commits as i64),
        SortMode::Authors => git().map(|g| g.contributors as i64),
        SortMode::Branches => git().map(|g| g.branches as i64),
        SortMode::LastCommit => git().and_then(|g| g.last_commit),
        SortMode::Opens => Some(repo.open_count as i64),
        SortMode::LastUsed => repo.last_used,
        _ => None,
    }
}

/// Case-insensitive comparison of two repos by display name.
fn name_cmp(a: &Repo, b: &Repo) -> std::cmp::Ordering {
    a.display_name()
        .to_lowercase()
        .cmp(&b.display_name().to_lowercase())
}

/// The recency key: the last-used timestamp, or `i64::MIN` when never used so
/// it sorts last under the "most recent first" comparison.
fn recency_key(repo: &Repo) -> i64 {
    repo.last_used.unwrap_or(i64::MIN)
}

/// A Firefox-style frecency score: the open count weighted by how recently the
/// entry was used. Never-used entries score 0.
fn frecency(repo: &Repo, now: i64) -> f64 {
    let Some(last_used) = repo.last_used else {
        return 0.0;
    };
    let age = now.saturating_sub(last_used);
    let weight = if age < 86_400 {
        4.0
    } else if age < 7 * 86_400 {
        2.0
    } else if age < 30 * 86_400 {
        1.0
    } else {
        0.5
    };
    repo.open_count as f64 * weight
}

#[cfg(test)]
mod tests;
