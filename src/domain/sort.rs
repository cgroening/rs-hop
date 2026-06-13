//! Sort modes for the repository list and the pure ordering they imply.

use crate::domain::repo::Repo;

/// How the visible list is ordered. Favourites are pinned to the top in
/// [`SortMode::Name`] and [`SortMode::Custom`] (sortable among themselves);
/// [`SortMode::Recent`] ignores favourites and orders purely by recency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMode {
    /// Favourites first, then case-insensitive by display name (the default).
    #[default]
    Name,
    /// Most recently used first; never-used entries last, then by name.
    /// Favourites are not pinned.
    Recent,
    /// Manual order (the stored `[[repos]]` order), favourites still on top.
    Custom,
}

impl SortMode {
    /// The next mode in the cycle, for a single toggle key.
    pub fn next(self) -> Self {
        match self {
            SortMode::Name => SortMode::Recent,
            SortMode::Recent => SortMode::Custom,
            SortMode::Custom => SortMode::Name,
        }
    }

    /// A short label for the header line (also the persisted value).
    pub fn label(self) -> &'static str {
        match self {
            SortMode::Name => "name",
            SortMode::Recent => "recent",
            SortMode::Custom => "custom",
        }
    }

    /// Parses a stored [`label`](Self::label) value, defaulting to `Name`.
    pub fn from_config_value(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "recent" => SortMode::Recent,
            "custom" => SortMode::Custom,
            _ => SortMode::Name,
        }
    }
}

/// Sorts `repos` in place according to `mode`. The ordering is total and stable
/// on equal keys (Rust's `sort_by` is stable), so equal names keep input order.
pub fn sort_repos(repos: &mut [Repo], mode: SortMode) {
    match mode {
        // Favourites on top, each group case-insensitive by name.
        SortMode::Name => {
            repos.sort_by(|a, b| b.fav.cmp(&a.fav).then_with(|| name_cmp(a, b)))
        }
        SortMode::Recent => repos.sort_by(|a, b| {
            // Most recent first (favourites are not pinned); a larger timestamp
            // comes earlier and a missing timestamp sorts last.
            recency_key(b)
                .cmp(&recency_key(a))
                .then_with(|| name_cmp(a, b))
        }),
        // Keep the given order, only floating favourites to the top (stable).
        SortMode::Custom => repos.sort_by(|a, b| b.fav.cmp(&a.fav)),
    }
}

/// Sorts `indices` (positions into `repos`) according to `mode`, leaving the
/// entries themselves untouched. Used by the view, which keeps service indices.
pub fn sort_indices(repos: &[Repo], indices: &mut [usize], mode: SortMode) {
    let compare = |a: &usize, b: &usize| {
        let (ra, rb) = (&repos[*a], &repos[*b]);
        match mode {
            // Favourites on top, each group case-insensitive by name.
            SortMode::Name => {
                rb.fav.cmp(&ra.fav).then_with(|| name_cmp(ra, rb))
            }
            // Pure recency (favourites not pinned), name as tiebreak.
            SortMode::Recent => recency_key(rb)
                .cmp(&recency_key(ra))
                .then_with(|| name_cmp(ra, rb)),
            // Stable on the index order, only favourites floated to the top.
            SortMode::Custom => rb.fav.cmp(&ra.fav),
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
    fn name_puts_favourites_first_then_alphabetical() {
        let mut repos = vec![
            repo("zebra", false, None),
            repo("alpha", false, None),
            repo("beta", true, None),
        ];
        sort_repos(&mut repos, SortMode::Name);
        let names: Vec<_> = repos.iter().map(Repo::display_name).collect();
        // The favourite "beta" floats above the alphabetically-sorted rest.
        assert_eq!(names, vec!["beta", "alpha", "zebra"]);
    }

    #[test]
    fn name_sorts_case_insensitively_within_a_group() {
        let mut repos =
            vec![repo("Banana", false, None), repo("apple", false, None)];
        sort_repos(&mut repos, SortMode::Name);
        let names: Vec<_> = repos.iter().map(Repo::display_name).collect();
        assert_eq!(names, vec!["apple", "Banana"]);
    }

    #[test]
    fn recent_ignores_favourites() {
        let mut repos = vec![
            repo("old-fav", true, Some(100)),
            repo("fresh", false, Some(900)),
            repo("never", false, None),
        ];
        sort_repos(&mut repos, SortMode::Recent);
        let names: Vec<_> = repos.iter().map(Repo::display_name).collect();
        // Purely by recency: the newer non-favourite beats the older favourite.
        assert_eq!(names, vec!["fresh", "old-fav", "never"]);
    }

    #[test]
    fn mode_cycles_through_all_modes() {
        let mode = SortMode::default();
        assert_eq!(mode, SortMode::Name);
        assert_eq!(mode.next(), SortMode::Recent);
        assert_eq!(mode.next().next(), SortMode::Custom);
        assert_eq!(mode.next().next().next(), SortMode::Name);
    }

    #[test]
    fn custom_keeps_input_order_with_favourites_on_top() {
        let mut repos = vec![
            repo("zebra", false, None),
            repo("alpha", true, None),
            repo("mid", false, None),
            repo("beta", true, None),
        ];
        let mut indices: Vec<usize> = (0..repos.len()).collect();
        sort_indices(&repos, &mut indices, SortMode::Custom);
        let names: Vec<_> =
            indices.iter().map(|&i| repos[i].display_name()).collect();
        // Favourites first in their input order, then the rest in input order.
        assert_eq!(names, vec!["alpha", "beta", "zebra", "mid"]);
        // Same via the in-place variant.
        sort_repos(&mut repos, SortMode::Custom);
        let names: Vec<_> = repos.iter().map(Repo::display_name).collect();
        assert_eq!(names, vec!["alpha", "beta", "zebra", "mid"]);
    }
}
