//! Project statistics: the pure types, aggregation and formatting.
//!
//! Two independent families, gathered by two independent workers because the
//! table's `Code` columns never need git and its `Activity` columns never need
//! a source walk: [`CodeEntry`] (lines of code per language plus the size on
//! disk) and [`GitStats`] (the repository's history). Everything here is pure -
//! collecting the numbers lives in `service::stats_service`.

use std::collections::BTreeMap;

/// Line counts for one language within one entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LangCount {
    /// Lines carrying code.
    pub code: usize,
    /// Lines that are comments.
    pub comments: usize,
    /// Blank lines.
    pub blanks: usize,
    /// Source files tokei recognised for this language.
    pub files: usize,
}

impl LangCount {
    /// Adds `other`'s counts into this one.
    fn add(&mut self, other: &LangCount) {
        self.code += other.code;
        self.comments += other.comments;
        self.blanks += other.blanks;
        self.files += other.files;
    }
}

/// The per-language line counts of one entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodeStats {
    /// Language name (as tokei names it) to its counts.
    pub languages: BTreeMap<String, LangCount>,
}

impl CodeStats {
    /// The counts summed over every language.
    pub fn total(&self) -> LangCount {
        let mut total = LangCount::default();
        for count in self.languages.values() {
            total.add(count);
        }
        total
    }
}

/// What an entry occupies on disk, from a metadata-only walk.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiskStats {
    /// Every byte under the entry, including `.git` and excluded directories.
    pub total_bytes: u64,
    /// Bytes inside a `.git` directory.
    pub git_bytes: u64,
    /// Bytes inside a directory excluded by the configured prefixes.
    pub excluded_bytes: u64,
    /// Every file under the entry, whatever its type.
    pub files: u64,
}

/// The code-and-size half of an entry's statistics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodeEntry {
    /// Lines of code per language.
    pub code: CodeStats,
    /// Size and file count on disk.
    pub disk: DiskStats,
}

/// The history half of a git entry's statistics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GitStats {
    /// Commits reachable from `HEAD`.
    pub commits: u64,
    /// Distinct commit authors.
    pub contributors: u64,
    /// Local branches.
    pub branches: u64,
    /// Tags.
    pub tags: u64,
    /// Unix timestamp of the earliest root commit.
    pub first_commit: Option<i64>,
    /// Unix timestamp of the most recent commit.
    pub last_commit: Option<i64>,
    /// Commits within the recent window (see `RECENT_DAYS`).
    pub commits_recent: u64,
}

/// The window, in days, the "recent commits" count covers.
pub const RECENT_DAYS: i64 = 30;

/// The summed statistics of the entries currently shown, for the totals row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    /// How many entries the totals cover.
    pub entries: usize,
    /// Lines of code, summed.
    pub code: usize,
    /// Recognised source files, summed.
    pub files: usize,
    /// Bytes on disk, summed.
    pub bytes: u64,
}

/// Sums the code and disk statistics of `entries`.
pub fn totals<'a>(entries: impl Iterator<Item = &'a CodeEntry>) -> Totals {
    let mut totals = Totals::default();
    for entry in entries {
        let total = entry.code.total();
        totals.entries += 1;
        totals.code += total.code;
        totals.files += total.files;
        totals.bytes += entry.disk.total_bytes;
    }
    totals
}

/// The language with the most lines of code, if any were counted.
pub fn top_language(stats: &CodeStats) -> Option<&str> {
    stats
        .languages
        .iter()
        .filter(|(_, count)| count.code > 0)
        .max_by_key(|(_, count)| count.code)
        .map(|(name, _)| name.as_str())
}

/// One language's share of an entry's code, for the detail panel's bars.
#[derive(Debug, Clone, PartialEq)]
pub struct LanguageShare {
    /// The language name.
    pub name: String,
    /// Its lines of code.
    pub code: usize,
    /// Its fraction of the entry's total code, in `0.0..=1.0`.
    pub share: f32,
}

/// The `limit` languages with the most code, largest first, each with its share
/// of the total. Languages with no code are skipped.
pub fn language_shares(stats: &CodeStats, limit: usize) -> Vec<LanguageShare> {
    let total = stats.total().code;
    if total == 0 {
        return Vec::new();
    }
    let mut ranked: Vec<(&str, usize)> = stats
        .languages
        .iter()
        .filter(|(_, count)| count.code > 0)
        .map(|(name, count)| (name.as_str(), count.code))
        .collect();
    // Most code first; ties keep the alphabetical order the map gives us.
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    ranked
        .into_iter()
        .take(limit)
        .map(|(name, code)| LanguageShare {
            name: name.to_string(),
            code,
            share: code as f32 / total as f32,
        })
        .collect()
}

/// Byte-size units, each 1024 times the one before.
const SIZE_UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

/// Formats `bytes` with a binary unit, e.g. `812 KB` or `3.2 MB`. Values below
/// a kilobyte keep no decimal, larger ones keep one when it says something.
pub fn format_bytes(bytes: u64) -> String {
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < SIZE_UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 || value >= 100.0 {
        return format!("{} {}", value.round() as u64, SIZE_UNITS[unit]);
    }
    format!("{value:.1} {}", SIZE_UNITS[unit])
}

/// Formats `count` with thousands separators, e.g. `12,480`.
pub fn format_count(count: u64) -> String {
    let digits = count.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Seconds in a day, the unit the age thresholds are expressed in.
const DAY: i64 = 86_400;

/// Formats the span between `then` and `now` (both unix seconds) as a compact
/// age like `3h`, `2d`, `5w` or `1y`. A future timestamp reads as `0m`.
pub fn format_age(then: i64, now: i64) -> String {
    let seconds = (now - then).max(0);
    let units: [(i64, &str); 5] = [
        (365 * DAY, "y"),
        (7 * DAY, "w"),
        (DAY, "d"),
        (3600, "h"),
        (60, "m"),
    ];
    for (size, suffix) in units {
        if seconds >= size {
            return format!("{}{suffix}", seconds / size);
        }
    }
    "0m".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(pairs: &[(&str, usize, usize)]) -> CodeStats {
        let languages = pairs
            .iter()
            .map(|(name, code, files)| {
                (
                    (*name).to_string(),
                    LangCount {
                        code: *code,
                        comments: 0,
                        blanks: 0,
                        files: *files,
                    },
                )
            })
            .collect();
        CodeStats { languages }
    }

    #[test]
    fn total_sums_every_language() {
        let total = stats(&[("Rust", 100, 3), ("TOML", 20, 2)]).total();
        assert_eq!(total.code, 120);
        assert_eq!(total.files, 5);
    }

    #[test]
    fn top_language_picks_the_most_code_and_ignores_empty_ones() {
        let stats =
            stats(&[("Markdown", 0, 1), ("Rust", 90, 2), ("TOML", 5, 1)]);
        assert_eq!(top_language(&stats), Some("Rust"));
        assert_eq!(top_language(&CodeStats::default()), None);
    }

    #[test]
    fn language_shares_rank_by_code_and_sum_to_one() {
        let shares =
            language_shares(&stats(&[("Rust", 90, 1), ("TOML", 10, 1)]), 5);
        assert_eq!(shares[0].name, "Rust");
        assert!((shares[0].share - 0.9).abs() < 1e-6);
        assert!(
            (shares.iter().map(|s| s.share).sum::<f32>() - 1.0).abs() < 1e-6
        );
        // A limit truncates, and an empty entry yields nothing.
        assert_eq!(
            language_shares(&stats(&[("Rust", 1, 1), ("C", 2, 1)]), 1).len(),
            1
        );
        assert!(language_shares(&CodeStats::default(), 3).is_empty());
    }

    #[test]
    fn totals_sum_code_files_and_bytes() {
        let entries = [
            CodeEntry {
                code: stats(&[("Rust", 100, 2)]),
                disk: DiskStats {
                    total_bytes: 1000,
                    ..DiskStats::default()
                },
            },
            CodeEntry {
                code: stats(&[("Rust", 5, 1)]),
                disk: DiskStats {
                    total_bytes: 24,
                    ..DiskStats::default()
                },
            },
        ];
        let sum = totals(entries.iter());
        assert_eq!(sum.entries, 2);
        assert_eq!(sum.code, 105);
        assert_eq!(sum.files, 3);
        assert_eq!(sum.bytes, 1024);
    }

    #[test]
    fn format_bytes_scales_and_keeps_one_decimal_where_it_helps() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(812), "812 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        // Three digits leave no room for a decimal.
        assert_eq!(format_bytes(512 * 1024), "512 KB");
        assert_eq!(format_bytes(3 * 1024 * 1024 + 209_715), "3.2 MB");
    }

    #[test]
    fn format_count_groups_thousands() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1000), "1,000");
        assert_eq!(format_count(12_480), "12,480");
        assert_eq!(format_count(1_234_567), "1,234,567");
    }

    #[test]
    fn format_age_picks_the_largest_fitting_unit() {
        let now = 1_000_000_000;
        assert_eq!(format_age(now, now), "0m");
        assert_eq!(format_age(now - 90, now), "1m");
        assert_eq!(format_age(now - 3 * 3600, now), "3h");
        assert_eq!(format_age(now - 2 * DAY, now), "2d");
        assert_eq!(format_age(now - 21 * DAY, now), "3w");
        assert_eq!(format_age(now - 400 * DAY, now), "1y");
        // A clock skew into the future must not underflow.
        assert_eq!(format_age(now + 5, now), "0m");
    }
}
