//! Measures what an entry occupies on disk, reading metadata only.
//!
//! [`measure`] answers three questions in one walk: how many bytes the entry
//! occupies in total, how many of those sit in its `.git` directory, and how
//! many sit in an excluded build directory. `archive::collect_files` cannot
//! serve here: it prunes the build directories and keeps `.git`, which is
//! exactly the opposite of what has to be counted.
//!
//! No file contents are read, so the walk stays cheap even on a large tree.
//! Symlinks are counted by their own size and never followed, so a link loop
//! cannot hang the walk and a linked tree is not counted twice.

use std::path::Path;

use crate::domain::stats::DiskStats;
use crate::util::archive::is_excluded;

/// The directory whose bytes are reported separately as the repository's
/// history rather than its content.
const GIT_DIR: &str = ".git";

/// Which subtree the walk is currently inside, so a file's bytes land in the
/// right bucket. The state is sticky: once inside `.git`, everything below it
/// counts as `.git`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bucket {
    /// Ordinary content.
    Content,
    /// Inside the `.git` directory.
    Git,
    /// Inside a directory excluded by the configured prefixes.
    Excluded,
}

/// The bytes and file count under `root`, split into total, `.git` and the
/// directories excluded by `exclude_dirs` (matched as a name prefix, the same
/// rule the ZIP backup uses).
///
/// A `root` that is a single file reports that file's size. A `root` that does
/// not exist reports zeroes rather than failing: a missing path is already
/// surfaced as an entry error elsewhere.
pub fn measure(root: &Path, exclude_dirs: &[String]) -> DiskStats {
    let mut stats = DiskStats::default();
    let Ok(metadata) = std::fs::symlink_metadata(root) else {
        return stats;
    };
    if !metadata.is_dir() {
        stats.total_bytes = metadata.len();
        stats.files = 1;
        return stats;
    }
    walk(root, exclude_dirs, Bucket::Content, &mut stats);
    stats
}

/// Recurses into `dir`, adding each file's size to the total and to `bucket`.
fn walk(
    dir: &Path,
    exclude_dirs: &[String],
    bucket: Bucket,
    stats: &mut DiskStats,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        // `file_type()` does not follow symlinks, so a link is its own entry.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            let name = entry.file_name();
            let child =
                child_bucket(&name.to_string_lossy(), bucket, exclude_dirs);
            walk(&entry.path(), exclude_dirs, child, stats);
        } else if let Ok(metadata) = entry.metadata() {
            add_file(metadata.len(), bucket, stats);
        }
    }
}

/// The bucket a subdirectory `name` belongs to, given its parent's bucket. Once
/// inside `.git` or an excluded directory, everything below stays there.
fn child_bucket(name: &str, parent: Bucket, exclude_dirs: &[String]) -> Bucket {
    if parent != Bucket::Content {
        return parent;
    }
    if name == GIT_DIR {
        return Bucket::Git;
    }
    if is_excluded(name, exclude_dirs) {
        return Bucket::Excluded;
    }
    Bucket::Content
}

/// Adds one file's `size` to the total and, when it sits in a special subtree,
/// to that subtree's bucket.
fn add_file(size: u64, bucket: Bucket, stats: &mut DiskStats) {
    stats.total_bytes = stats.total_bytes.saturating_add(size);
    stats.files = stats.files.saturating_add(1);
    match bucket {
        Bucket::Content => {}
        Bucket::Git => stats.git_bytes = stats.git_bytes.saturating_add(size),
        Bucket::Excluded => {
            stats.excluded_bytes = stats.excluded_bytes.saturating_add(size);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// A temp tree: `src/lib.rs` (10 B), `.git/HEAD` (20 B),
    /// `target.nosync/out` (40 B).
    fn tree(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir()
            .join(format!("hop-diskusage-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join("target.nosync")).unwrap();
        fs::write(root.join("src/lib.rs"), vec![b'x'; 10]).unwrap();
        fs::write(root.join(".git/HEAD"), vec![b'x'; 20]).unwrap();
        fs::write(root.join("target.nosync/out"), vec![b'x'; 40]).unwrap();
        root
    }

    #[test]
    fn measure_splits_git_and_excluded_bytes_out_of_the_total() {
        let root = tree("split");
        let stats = measure(&root, &["target".to_string()]);
        assert_eq!(stats.total_bytes, 70, "every byte is in the total");
        assert_eq!(stats.git_bytes, 20);
        // The prefix rule reaches `target.nosync`, as it does for the backup.
        assert_eq!(stats.excluded_bytes, 40);
        assert_eq!(stats.files, 3);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn measure_without_exclusions_counts_everything_as_content() {
        let root = tree("plain");
        let stats = measure(&root, &[]);
        assert_eq!(stats.total_bytes, 70);
        assert_eq!(stats.excluded_bytes, 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn measure_reports_a_single_file_and_tolerates_a_missing_path() {
        let root = tree("file");
        let stats = measure(&root.join("src/lib.rs"), &[]);
        assert_eq!((stats.total_bytes, stats.files), (10, 1));
        assert_eq!(
            measure(Path::new("/no/such/path"), &[]),
            DiskStats::default()
        );
        let _ = fs::remove_dir_all(&root);
    }
}
