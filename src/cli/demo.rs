//! Built-in demo dataset for `hop --demo`: a curated, self-contained spread of
//! entries used to take screenshots without touching real repositories.
//!
//! Every entry carries `example_git_info` so the git tabs render a controlled
//! status (`example_mode` skips all real git calls and path checks). The paths
//! are fake but plausible; nothing is read from or written to disk.

use std::path::PathBuf;

use crate::domain::repo::{GitInfo, Repo, RepoKind};

/// The demo entries, spanning the Git, Files and Archive tabs with a varied mix
/// of statuses, favourites, slugs, sections and one deliberately broken repo.
pub fn repos() -> Vec<Repo> {
    vec![
        git(
            "acme-api",
            "/Users/you/Code/acme-api",
            Some("api"),
            true,
            clean("main", "acme-api"),
        ),
        git(
            "web-dashboard",
            "/Users/you/Code/web-dashboard",
            Some("web"),
            false,
            changes("main", "web-dashboard", 3),
        ),
        git(
            "payments-service",
            "/Users/you/Code/payments-service",
            None,
            false,
            diverged("main", "payments-service", 2, 1),
        ),
        git(
            "design-system",
            "/Users/you/Code/design-system",
            Some("ds"),
            false,
            busy("feature/tokens", "design-system", 7, 1),
        ),
        git(
            "dotfiles",
            "/Users/you/Code/dotfiles",
            None,
            false,
            behind("main", "dotfiles", 4),
        ),
        git(
            "legacy-cms",
            "/Users/you/Code/legacy-cms",
            None,
            false,
            broken(),
        ),
        folder("Projects", "/Users/you/Code", Some("code"), Some("Code")),
        folder("Rust workspace", "/Users/you/Code/rust", None, Some("Code")),
        file(
            "Architecture notes",
            "/Users/you/Docs/architecture.md",
            None,
            Some("Docs"),
        ),
        file(
            "Meeting notes",
            "/Users/you/Docs/meetings.md",
            Some("notes"),
            Some("Docs"),
        ),
        folder("Downloads", "/Users/you/Downloads", None, None),
        archived_git(
            "old-prototype",
            "/Users/you/Code/old-prototype",
            clean("main", "old-prototype"),
        ),
        archived_git(
            "deprecated-api",
            "/Users/you/Code/deprecated-api",
            clean("main", "deprecated-api"),
        ),
    ]
}

/// The Files-tab section order for the demo.
pub fn sections() -> Vec<String> {
    vec!["Code".to_string(), "Docs".to_string()]
}

/// Builds a git entry with demo status, an optional slug and favourite flag.
fn git(
    name: &str,
    path: &str,
    slug: Option<&str>,
    fav: bool,
    info: GitInfo,
) -> Repo {
    let mut repo = Repo::new(PathBuf::from(path));
    repo.name = Some(name.to_string());
    repo.slug = slug.map(str::to_string);
    repo.fav = fav;
    repo.kind = RepoKind::Git;
    repo.example_git_info = Some(info);
    repo
}

/// Builds an archived git entry with demo status.
fn archived_git(name: &str, path: &str, info: GitInfo) -> Repo {
    let mut repo = git(name, path, None, false, info);
    repo.archived = true;
    repo
}

/// Builds a folder entry in an optional section with an optional slug.
fn folder(
    name: &str,
    path: &str,
    slug: Option<&str>,
    section: Option<&str>,
) -> Repo {
    path_entry(name, path, slug, section)
}

/// Builds a file entry in an optional section with an optional slug.
fn file(
    name: &str,
    path: &str,
    slug: Option<&str>,
    section: Option<&str>,
) -> Repo {
    path_entry(name, path, slug, section)
}

/// Builds a [`RepoKind::Path`] entry (folders and files share this shape).
fn path_entry(
    name: &str,
    path: &str,
    slug: Option<&str>,
    section: Option<&str>,
) -> Repo {
    let mut repo = Repo::new(PathBuf::from(path));
    repo.name = Some(name.to_string());
    repo.slug = slug.map(str::to_string);
    repo.section = section.map(str::to_string);
    repo.kind = RepoKind::Path;
    repo
}

/// A clean working tree on `branch` with a GitHub name.
fn clean(branch: &str, github: &str) -> GitInfo {
    GitInfo {
        valid: true,
        current_branch_name: Some(branch.to_string()),
        github_repo_name: Some(github.to_string()),
        ..GitInfo::default()
    }
}

/// `count` uncommitted changes on `branch`.
fn changes(branch: &str, github: &str, count: u32) -> GitInfo {
    GitInfo {
        changes: Some(count),
        ..clean(branch, github)
    }
}

/// `ahead`/`behind` the upstream with a clean tree.
fn diverged(branch: &str, github: &str, ahead: u32, behind: u32) -> GitInfo {
    GitInfo {
        ahead: Some(ahead),
        behind: Some(behind),
        ..clean(branch, github)
    }
}

/// Uncommitted changes plus commits ahead of the upstream.
fn busy(branch: &str, github: &str, changes: u32, ahead: u32) -> GitInfo {
    GitInfo {
        changes: Some(changes),
        ahead: Some(ahead),
        ..clean(branch, github)
    }
}

/// `count` commits behind the upstream with a clean tree.
fn behind(branch: &str, github: &str, count: u32) -> GitInfo {
    GitInfo {
        behind: Some(count),
        ..clean(branch, github)
    }
}

/// An invalid repository, shown with the red `!` error marker.
fn broken() -> GitInfo {
    GitInfo {
        valid: false,
        error: Some("not a git repository".to_string()),
        ..GitInfo::default()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::domain::filter::{Tab, belongs_to_tab};
    use crate::domain::slug;

    #[test]
    fn demo_spans_all_three_tabs() {
        let repos = repos();
        for tab in Tab::ALL {
            assert!(
                repos.iter().any(|repo| belongs_to_tab(repo, tab)),
                "no demo entry for tab {tab:?}"
            );
        }
    }

    #[test]
    fn demo_slugs_are_valid_and_unique() {
        let mut seen = HashSet::new();
        for repo in repos() {
            let Some(slug) = repo.slug else { continue };
            assert!(
                slug::validate_format(&slug).is_ok(),
                "invalid demo slug: {slug}"
            );
            assert!(seen.insert(slug.clone()), "duplicate demo slug: {slug}");
        }
    }

    #[test]
    fn every_git_entry_has_example_info() {
        for repo in repos() {
            if repo.kind == RepoKind::Git {
                assert!(
                    repo.example_git_info.is_some(),
                    "git demo entry {} lacks example_git_info",
                    repo.display_name()
                );
            }
        }
    }

    #[test]
    fn demo_includes_a_broken_repo() {
        assert!(repos().iter().any(|repo| repo.example_error().is_some()));
    }

    #[test]
    fn path_entry_sections_are_declared() {
        let declared: HashSet<String> = sections().into_iter().collect();
        for repo in repos() {
            if let Some(section) = repo.section {
                assert!(
                    declared.contains(&section),
                    "section {section} not declared in sections()"
                );
            }
        }
    }
}
