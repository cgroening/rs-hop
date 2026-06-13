//! Finds the closest existing ancestor of a missing path.
//!
//! When a repository's path no longer exists, the path picker should open as
//! near the original location as possible. The walk is pure: it takes an
//! `exists` predicate so it can be tested without touching the filesystem.

use std::path::{Path, PathBuf};

/// The closest ancestor of `start` (including `start` itself) for which
/// `exists` returns true, or `None` when no ancestor exists.
pub fn nearest_existing<F>(start: &Path, exists: F) -> Option<PathBuf>
where
    F: Fn(&Path) -> bool,
{
    start
        .ancestors()
        .find(|candidate| {
            !candidate.as_os_str().is_empty() && exists(candidate)
        })
        .map(Path::to_path_buf)
}

/// Convenience wrapper using the real filesystem.
pub fn nearest_existing_on_disk(start: &Path) -> Option<PathBuf> {
    nearest_existing(start, |candidate| candidate.exists())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn exists_set(paths: &[&str]) -> HashSet<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn returns_start_when_it_exists() {
        let present = exists_set(&["/a/b/c"]);
        let found =
            nearest_existing(Path::new("/a/b/c"), |p| present.contains(p));
        assert_eq!(found, Some(PathBuf::from("/a/b/c")));
    }

    #[test]
    fn walks_up_to_first_existing_parent() {
        let present = exists_set(&["/a", "/a/b"]);
        let found =
            nearest_existing(Path::new("/a/b/c/d"), |p| present.contains(p));
        assert_eq!(found, Some(PathBuf::from("/a/b")));
    }

    #[test]
    fn none_when_no_ancestor_exists() {
        let found = nearest_existing(Path::new("/x/y/z"), |_| false);
        assert_eq!(found, None);
    }
}
