//! Recursive discovery of git work trees under a directory (for `hop scan`).
//!
//! A hand-rolled `std::fs` walk (no extra dependency): it skips hidden
//! directories and `node_modules`/`target`, never follows symlinks, and by
//! default stops descending once a repository is found. `ScanOptions::nested`
//! keeps descending into found repositories to pick up nested repos/submodules.

use std::path::{Path, PathBuf};

/// How the walk behaves.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScanOptions {
    /// Maximum recursion depth below `root` (`None` = unlimited).
    pub max_depth: Option<usize>,
    /// Keep descending into a found repository (find nested repos/submodules).
    pub nested: bool,
}

/// The git work trees (directories containing a `.git`) under `root`, sorted.
pub fn find_git_repos(root: &Path, opts: ScanOptions) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(root, 0, opts, &mut found);
    found.sort();
    found.dedup();
    found
}

/// Recurses into `dir`, recording repositories and descending per `opts`.
fn walk(dir: &Path, depth: usize, opts: ScanOptions, found: &mut Vec<PathBuf>) {
    let is_repo = dir.join(".git").exists();
    if is_repo {
        found.push(dir.to_path_buf());
        // Stop at the repository unless asked to look for nested ones.
        if !opts.nested {
            return;
        }
    }
    if opts.max_depth.is_some_and(|max| depth >= max) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        // Never follow symlinks (avoids loops and escaping the tree).
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name();
        if should_skip_dir(&name.to_string_lossy()) {
            continue;
        }
        walk(&entry.path(), depth + 1, opts, found);
    }
}

/// Whether a directory `name` is pruned from the walk: hidden (a leading dot),
/// or a known heavy/uninteresting directory.
fn should_skip_dir(name: &str) -> bool {
    name.starts_with('.') || matches!(name, "node_modules" | "target")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_hidden_and_heavy_dirs() {
        assert!(should_skip_dir(".git"));
        assert!(should_skip_dir(".cache"));
        assert!(should_skip_dir("node_modules"));
        assert!(should_skip_dir("target"));
        assert!(!should_skip_dir("src"));
        assert!(!should_skip_dir("my-repo"));
    }
}
