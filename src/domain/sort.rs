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

/// Compares two entries under `ctx`.
fn compare(a: &Repo, b: &Repo, ctx: &SortContext) -> std::cmp::Ordering {
    if ctx.mode.is_statistic() {
        return compare_statistic(a, b, ctx);
    }
    let ordering = compare_identity(a, b, ctx);
    match ctx.dir {
        SortDir::Asc => ordering,
        SortDir::Desc => ordering.reverse(),
    }
}

/// Compares by what an entry is: name, recency, frecency or the stored order.
fn compare_identity(
    a: &Repo,
    b: &Repo,
    ctx: &SortContext,
) -> std::cmp::Ordering {
    match ctx.mode {
        // Favourites on top, each group case-insensitive by name.
        SortMode::Name => b.fav.cmp(&a.fav).then_with(|| name_cmp(a, b)),
        // Pure recency (favourites not pinned), name as tiebreak.
        SortMode::Recent => recency_key(b)
            .cmp(&recency_key(a))
            .then_with(|| name_cmp(a, b)),
        // Favourites on top, then highest frecency, name as tiebreak.
        SortMode::Frecency => b
            .fav
            .cmp(&a.fav)
            .then_with(|| {
                frecency(b, ctx.now)
                    .partial_cmp(&frecency(a, ctx.now))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| name_cmp(a, b)),
        // Stable on the index order, only favourites floated to the top.
        _ => b.fav.cmp(&a.fav),
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
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::domain::stats::{CodeStats, DiskStats, LangCount};

    const NOW: i64 = 1_000_000;

    fn repo(name: &str, fav: bool, last_used: Option<i64>) -> Repo {
        let mut repo = Repo::new(PathBuf::from(format!("/r/{name}")));
        repo.name = Some(name.to_string());
        repo.fav = fav;
        repo.last_used = last_used;
        repo
    }

    /// An empty lookup, for the modes that do not read statistics.
    fn no_stats() -> (HashMap<PathBuf, CodeEntry>, HashMap<PathBuf, GitStats>) {
        (HashMap::new(), HashMap::new())
    }

    fn context<'a>(
        mode: SortMode,
        dir: SortDir,
        maps: &'a (HashMap<PathBuf, CodeEntry>, HashMap<PathBuf, GitStats>),
    ) -> SortContext<'a> {
        SortContext {
            mode,
            dir,
            now: NOW,
            stats: StatsLookup {
                code: &maps.0,
                git: &maps.1,
            },
        }
    }

    /// The display names of `repos` after sorting under `ctx`.
    fn ordered(repos: &[Repo], ctx: &SortContext) -> Vec<String> {
        let mut indices: Vec<usize> = (0..repos.len()).collect();
        sort_indices(repos, &mut indices, ctx);
        indices.iter().map(|&i| repos[i].display_name()).collect()
    }

    /// A code entry with `code` lines in one file of `bytes` bytes.
    fn code_entry(code: usize, bytes: u64) -> CodeEntry {
        let languages = std::collections::BTreeMap::from([(
            "Rust".to_string(),
            LangCount {
                code,
                comments: 0,
                blanks: 0,
                files: 1,
            },
        )]);
        CodeEntry {
            code: CodeStats { languages },
            disk: DiskStats {
                total_bytes: bytes,
                ..DiskStats::default()
            },
        }
    }

    #[test]
    fn name_puts_favourites_first_then_alphabetical() {
        let repos = vec![
            repo("zebra", false, None),
            repo("alpha", false, None),
            repo("beta", true, None),
        ];
        let maps = no_stats();
        let names =
            ordered(&repos, &context(SortMode::Name, SortDir::Asc, &maps));
        assert_eq!(names, vec!["beta", "alpha", "zebra"]);
    }

    #[test]
    fn name_sorts_case_insensitively_within_a_group() {
        let repos =
            vec![repo("Banana", false, None), repo("apple", false, None)];
        let maps = no_stats();
        let names =
            ordered(&repos, &context(SortMode::Name, SortDir::Asc, &maps));
        assert_eq!(names, vec!["apple", "Banana"]);
    }

    #[test]
    fn descending_reverses_an_identity_sort() {
        let repos =
            vec![repo("alpha", false, None), repo("zebra", false, None)];
        let maps = no_stats();
        let names =
            ordered(&repos, &context(SortMode::Name, SortDir::Desc, &maps));
        assert_eq!(names, vec!["zebra", "alpha"]);
    }

    #[test]
    fn recent_ignores_favourites() {
        let repos = vec![
            repo("old-fav", true, Some(100)),
            repo("fresh", false, Some(900)),
            repo("never", false, None),
        ];
        let maps = no_stats();
        let names =
            ordered(&repos, &context(SortMode::Recent, SortDir::Asc, &maps));
        assert_eq!(names, vec!["fresh", "old-fav", "never"]);
    }

    #[test]
    fn frecency_weights_count_by_recency() {
        let mut recent_few = repo("recent-few", false, Some(NOW - 3600));
        recent_few.open_count = 2; // weight 4 -> 8
        let mut old_many = repo("old-many", false, Some(NOW - 60 * 86_400));
        old_many.open_count = 10; // weight 0.5 -> 5
        let never = repo("never", false, None); // 0
        let repos = vec![old_many, never, recent_few];
        let maps = no_stats();
        let names =
            ordered(&repos, &context(SortMode::Frecency, SortDir::Asc, &maps));
        assert_eq!(names, vec!["recent-few", "old-many", "never"]);
    }

    #[test]
    fn custom_keeps_input_order_with_favourites_on_top() {
        let repos = vec![
            repo("zebra", false, None),
            repo("alpha", true, None),
            repo("mid", false, None),
            repo("beta", true, None),
        ];
        let maps = no_stats();
        let names =
            ordered(&repos, &context(SortMode::Custom, SortDir::Asc, &maps));
        assert_eq!(names, vec!["alpha", "beta", "zebra", "mid"]);
    }

    #[test]
    fn loc_orders_by_lines_and_ignores_favourites() {
        let repos = vec![
            repo("small-fav", true, None),
            repo("big", false, None),
            repo("mid", false, None),
        ];
        let code = HashMap::from([
            (repos[0].path.clone(), code_entry(10, 100)),
            (repos[1].path.clone(), code_entry(900, 5)),
            (repos[2].path.clone(), code_entry(100, 50)),
        ]);
        let maps = (code, HashMap::new());
        assert_eq!(
            ordered(&repos, &context(SortMode::Loc, SortDir::Desc, &maps)),
            vec!["big", "mid", "small-fav"]
        );
        assert_eq!(
            ordered(&repos, &context(SortMode::Loc, SortDir::Asc, &maps)),
            vec!["small-fav", "mid", "big"]
        );
        // Size orders independently of the line count.
        assert_eq!(
            ordered(&repos, &context(SortMode::Size, SortDir::Desc, &maps)),
            vec!["small-fav", "mid", "big"]
        );
    }

    #[test]
    fn entries_without_a_computed_value_sort_last_in_both_directions() {
        // The background worker has only reported "known" so far.
        let repos =
            vec![repo("pending", false, None), repo("known", false, None)];
        let code = HashMap::from([(repos[1].path.clone(), code_entry(5, 5))]);
        let maps = (code, HashMap::new());
        for dir in [SortDir::Asc, SortDir::Desc] {
            assert_eq!(
                ordered(&repos, &context(SortMode::Loc, dir, &maps)),
                vec!["known", "pending"],
                "an uncomputed entry must never jump to the top"
            );
        }
    }

    #[test]
    fn commits_read_the_git_half_of_the_lookup() {
        let repos = vec![repo("few", false, None), repo("many", false, None)];
        let git = HashMap::from([
            (
                repos[0].path.clone(),
                GitStats {
                    commits: 3,
                    ..GitStats::default()
                },
            ),
            (
                repos[1].path.clone(),
                GitStats {
                    commits: 300,
                    ..GitStats::default()
                },
            ),
        ]);
        let maps = (HashMap::new(), git);
        assert_eq!(
            ordered(&repos, &context(SortMode::Commits, SortDir::Desc, &maps)),
            vec!["many", "few"]
        );
    }

    #[test]
    fn labels_round_trip_and_unknown_falls_back() {
        for mode in [
            SortMode::Name,
            SortMode::Recent,
            SortMode::Frecency,
            SortMode::Custom,
            SortMode::Loc,
            SortMode::Size,
            SortMode::Files,
            SortMode::Commits,
            SortMode::LastCommit,
            SortMode::Authors,
            SortMode::Branches,
            SortMode::Opens,
            SortMode::LastUsed,
        ] {
            assert_eq!(SortMode::from_config_value(mode.label()), mode);
        }
        assert_eq!(SortMode::from_config_value("bogus"), SortMode::Name);
        assert_eq!(SortDir::from_config_value("desc"), SortDir::Desc);
        assert_eq!(SortDir::from_config_value("bogus"), SortDir::Asc);
        assert_eq!(SortDir::Asc.flip(), SortDir::Desc);
    }

    #[test]
    fn only_the_column_modes_are_statistics() {
        assert!(!SortMode::Name.is_statistic());
        assert!(!SortMode::Custom.is_statistic());
        assert!(SortMode::Loc.is_statistic());
        assert!(SortMode::LastUsed.is_statistic());
    }
}
