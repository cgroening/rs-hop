//! Sort modes for the repository list and the pure ordering they imply.

use crate::domain::repo::Repo;

/// How the visible list is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMode {
    /// Favourites first, then case-insensitive by display name (the default,
    /// matching git-repo-jumper).
    #[default]
    FavThenName,
    /// Most recently used first; never-used entries last, then by name.
    LastUsed,
    /// Case-insensitive by display name only.
    Name,
}

impl SortMode {
    /// The next mode in the cycle, for a single toggle key.
    pub fn next(self) -> Self {
        match self {
            SortMode::FavThenName => SortMode::LastUsed,
            SortMode::LastUsed => SortMode::Name,
            SortMode::Name => SortMode::FavThenName,
        }
    }

    /// A short label for the header line.
    pub fn label(self) -> &'static str {
        match self {
            SortMode::FavThenName => "favourites",
            SortMode::LastUsed => "recent",
            SortMode::Name => "name",
        }
    }
}

/// Sorts `repos` in place according to `mode`. The ordering is total and stable
/// on equal keys (Rust's `sort_by` is stable), so equal names keep input order.
pub fn sort_repos(repos: &mut [Repo], mode: SortMode) {
    match mode {
        SortMode::FavThenName => {
            repos.sort_by(|a, b| b.fav.cmp(&a.fav).then_with(|| name_cmp(a, b)))
        }
        SortMode::Name => repos.sort_by(name_cmp),
        SortMode::LastUsed => repos.sort_by(|a, b| {
            // Most recent first: a larger timestamp should come earlier, and a
            // missing timestamp sorts last.
            recency_key(b)
                .cmp(&recency_key(a))
                .then_with(|| name_cmp(a, b))
        }),
    }
}

/// Sorts `indices` (positions into `repos`) according to `mode`, leaving the
/// entries themselves untouched. Used by the view, which keeps service indices.
pub fn sort_indices(repos: &[Repo], indices: &mut [usize], mode: SortMode) {
    let compare = |a: &usize, b: &usize| {
        let (ra, rb) = (&repos[*a], &repos[*b]);
        match mode {
            SortMode::FavThenName => {
                rb.fav.cmp(&ra.fav).then_with(|| name_cmp(ra, rb))
            }
            SortMode::Name => name_cmp(ra, rb),
            SortMode::LastUsed => recency_key(rb)
                .cmp(&recency_key(ra))
                .then_with(|| name_cmp(ra, rb)),
        }
    };
    indices.sort_by(compare);
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn repo(name: &str, fav: bool, last_used: Option<i64>) -> Repo {
        let mut repo = Repo::new(PathBuf::from(format!("/r/{name}")));
        repo.name = Some(name.to_string());
        repo.fav = fav;
        repo.last_used = last_used;
        repo
    }

    #[test]
    fn fav_then_name_puts_favourites_first_alphabetically() {
        let mut repos = vec![
            repo("zebra", false, None),
            repo("alpha", false, None),
            repo("beta", true, None),
        ];
        sort_repos(&mut repos, SortMode::FavThenName);
        let names: Vec<_> = repos.iter().map(Repo::display_name).collect();
        assert_eq!(names, vec!["beta", "alpha", "zebra"]);
    }

    #[test]
    fn name_sorts_case_insensitively() {
        let mut repos =
            vec![repo("Banana", false, None), repo("apple", false, None)];
        sort_repos(&mut repos, SortMode::Name);
        let names: Vec<_> = repos.iter().map(Repo::display_name).collect();
        assert_eq!(names, vec!["apple", "Banana"]);
    }

    #[test]
    fn last_used_orders_recent_first_unused_last() {
        let mut repos = vec![
            repo("old", false, Some(100)),
            repo("never", false, None),
            repo("fresh", false, Some(900)),
        ];
        sort_repos(&mut repos, SortMode::LastUsed);
        let names: Vec<_> = repos.iter().map(Repo::display_name).collect();
        assert_eq!(names, vec!["fresh", "old", "never"]);
    }

    #[test]
    fn mode_cycles_through_all_three() {
        let mode = SortMode::default();
        assert_eq!(mode, SortMode::FavThenName);
        assert_eq!(mode.next(), SortMode::LastUsed);
        assert_eq!(mode.next().next(), SortMode::Name);
        assert_eq!(mode.next().next().next(), SortMode::FavThenName);
    }
}
