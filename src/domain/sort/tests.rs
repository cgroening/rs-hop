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
    floated(mode, dir, maps, true)
}

fn floated<'a>(
    mode: SortMode,
    dir: SortDir,
    maps: &'a (HashMap<PathBuf, CodeEntry>, HashMap<PathBuf, GitStats>),
    float_favs: bool,
) -> SortContext<'a> {
    SortContext {
        mode,
        dir,
        float_favs,
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
    let names = ordered(&repos, &context(SortMode::Name, SortDir::Asc, &maps));
    assert_eq!(names, vec!["beta", "alpha", "zebra"]);
}

#[test]
fn name_sorts_case_insensitively_within_a_group() {
    let repos = vec![repo("Banana", false, None), repo("apple", false, None)];
    let maps = no_stats();
    let names = ordered(&repos, &context(SortMode::Name, SortDir::Asc, &maps));
    assert_eq!(names, vec!["apple", "Banana"]);
}

#[test]
fn descending_reverses_an_identity_sort() {
    let repos = vec![repo("alpha", false, None), repo("zebra", false, None)];
    let maps = no_stats();
    let names = ordered(&repos, &context(SortMode::Name, SortDir::Desc, &maps));
    assert_eq!(names, vec!["zebra", "alpha"]);
}

#[test]
fn recent_without_float_ignores_favourites() {
    let repos = vec![
        repo("old-fav", true, Some(100)),
        repo("fresh", false, Some(900)),
        repo("never", false, None),
    ];
    let maps = no_stats();
    let ctx = floated(SortMode::Recent, SortDir::Asc, &maps, false);
    assert_eq!(ordered(&repos, &ctx), vec!["fresh", "old-fav", "never"]);
}

#[test]
fn float_favs_pins_favourites_in_every_mode() {
    let repos = vec![
        repo("old-fav", true, Some(100)),
        repo("fresh", false, Some(900)),
        repo("never", false, None),
    ];
    let maps = no_stats();
    // Recency would rank "fresh" first, but the favourite floats on top
    // and non-favourites keep their recency order below it.
    let ctx = floated(SortMode::Recent, SortDir::Asc, &maps, true);
    assert_eq!(ordered(&repos, &ctx), vec!["old-fav", "fresh", "never"]);
}

#[test]
fn float_favs_off_gives_pure_order() {
    let repos = vec![repo("zebra", true, None), repo("alpha", false, None)];
    let maps = no_stats();
    let ctx = floated(SortMode::Name, SortDir::Asc, &maps, false);
    // The favourite is not pinned; pure alphabetical order.
    assert_eq!(ordered(&repos, &ctx), vec!["alpha", "zebra"]);
}

#[test]
fn float_favs_pins_in_a_statistics_mode() {
    let repos = vec![repo("small-fav", true, None), repo("big", false, None)];
    let code = HashMap::from([
        (repos[0].path.clone(), code_entry(10, 100)),
        (repos[1].path.clone(), code_entry(900, 5)),
    ]);
    let maps = (code, HashMap::new());
    // Largest first would rank "big", but the favourite floats on top.
    let ctx = floated(SortMode::Loc, SortDir::Desc, &maps, true);
    assert_eq!(ordered(&repos, &ctx), vec!["small-fav", "big"]);
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
fn loc_orders_by_lines_without_float() {
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
    let desc = floated(SortMode::Loc, SortDir::Desc, &maps, false);
    assert_eq!(ordered(&repos, &desc), vec!["big", "mid", "small-fav"]);
    let asc = floated(SortMode::Loc, SortDir::Asc, &maps, false);
    assert_eq!(ordered(&repos, &asc), vec!["small-fav", "mid", "big"]);
    // Size orders independently of the line count.
    let size = floated(SortMode::Size, SortDir::Desc, &maps, false);
    assert_eq!(ordered(&repos, &size), vec!["small-fav", "mid", "big"]);
}

#[test]
fn entries_without_a_computed_value_sort_last_in_both_directions() {
    // The background worker has only reported "known" so far.
    let repos = vec![repo("pending", false, None), repo("known", false, None)];
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
