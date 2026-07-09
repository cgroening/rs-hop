//! Pins the two exclusion rules the line count must obey.
//!
//! hop counts lines through `tokei`, which honours `.gitignore` by itself, and
//! hands it the configured `zip_exclude_dirs` as directory globs on top. Both
//! must bite. The glob translation (`target` -> `target*/`) is the one part of
//! the design that cannot be read off tokei's documentation, so it is proven
//! here against a real fixture rather than assumed.
//!
//! Note the `.git` directory in the fixture: tokei's ignore layer only applies
//! `.gitignore` inside an actual repository. A plain folder entry therefore
//! relies on the configured exclusions alone - which is why they exist.

use std::fs;
use std::path::{Path, PathBuf};

use hop::domain::stats::top_language;
use hop::service::stats_service::{CodeUpdate, spawn_code_stats};

/// Builds a fixture tree with four Rust files of one code line each:
///
/// - `src/kept.rs`          counted
/// - `generated/skip.rs`    ignored via `.gitignore`
/// - `target.nosync/out.rs` ignored via the `target` exclude prefix
/// - `vendor/dep.rs`        ignored via the `vendor` exclude prefix
///
/// A `.git` directory is created because the ignore layer only reads
/// `.gitignore` inside a repository.
fn fixture(tag: &str) -> PathBuf {
    let root = std::env::temp_dir()
        .join(format!("hop-code-stats-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    for dir in ["src", "generated", "target.nosync", "vendor", ".git"] {
        fs::create_dir_all(root.join(dir)).expect("create fixture dir");
    }
    let line = "fn main() {}\n";
    fs::write(root.join("src/kept.rs"), line).expect("write");
    fs::write(root.join("generated/skip.rs"), line).expect("write");
    fs::write(root.join("target.nosync/out.rs"), line).expect("write");
    fs::write(root.join("vendor/dep.rs"), line).expect("write");
    fs::write(root.join(".gitignore"), "generated/\n").expect("write");
    fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").expect("write");
    root
}

/// Runs the background worker over `root` to completion and returns the counts.
fn count(root: &Path, exclude_dirs: &[&str]) -> hop::domain::stats::CodeEntry {
    let excludes = exclude_dirs.iter().map(|d| (*d).to_string()).collect();
    let rx = spawn_code_stats(vec![root.to_path_buf()], excludes);
    let mut result = None;
    for update in rx {
        if let CodeUpdate::Done { stats, .. } = update {
            result = Some(*stats);
        }
    }
    result.expect("the worker reports one Done per path")
}

#[test]
fn gitignored_and_excluded_directories_are_both_skipped() {
    let root = fixture("both");
    let entry = count(&root, &["target", "vendor"]);

    let rust = entry
        .code
        .languages
        .get("Rust")
        .copied()
        .expect("the kept file is Rust");
    assert_eq!(
        rust.files, 1,
        "only src/kept.rs may be counted: .gitignore must hide generated/, \
         and the exclude prefixes must hide target.nosync/ and vendor/"
    );
    assert_eq!(rust.code, 1);
    assert_eq!(top_language(&entry.code), Some("Rust"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_exclude_prefix_reaches_a_suffixed_directory() {
    // `target` must hide `target.nosync`, exactly as it does for the ZIP
    // backup. Without the trailing `*` in the glob it would not.
    let root = fixture("prefix");
    let with_prefix = count(&root, &["target"]);
    let without = count(&root, &[]);

    let counted = |entry: &hop::domain::stats::CodeEntry| {
        entry.code.languages.get("Rust").map_or(0, |c| c.files)
    };
    // Without exclusions: src, target.nosync and vendor (generated stays
    // hidden by .gitignore).
    assert_eq!(counted(&without), 3);
    assert_eq!(counted(&with_prefix), 2, "target.nosync must be excluded");

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_disk_measurement_separates_excluded_bytes() {
    let root = fixture("disk");
    let entry = count(&root, &["target"]);
    // The size walk sees every byte, including the excluded directory, but
    // reports the excluded ones separately.
    assert!(entry.disk.total_bytes > entry.disk.excluded_bytes);
    assert!(entry.disk.excluded_bytes > 0, "target.nosync has one file");
    assert!(entry.disk.git_bytes > 0, "the fixture has a .git/HEAD");
    // The `.git` bytes are inside the total, not on top of it.
    assert!(entry.disk.total_bytes > entry.disk.git_bytes);

    let _ = fs::remove_dir_all(&root);
}
