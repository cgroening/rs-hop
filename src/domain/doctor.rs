//! Pure diagnostics over the managed entries, backing the `hop doctor` command.
//!
//! Filesystem checks are injected as predicates so the logic stays pure and
//! testable; the CLI passes the real `Path::exists` / `.git` checks.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use crate::domain::repo::{Repo, RepoKind};
use crate::domain::slug;

/// A problem found with an entry (or across entries).
#[derive(Debug, PartialEq, Eq)]
pub enum Issue {
    /// The entry's path no longer exists.
    MissingPath {
        /// Display name of the affected entry.
        name: String,
        /// The path that no longer exists.
        path: String,
    },
    /// A git entry whose path is not a git repository.
    NotGitRepo {
        /// Display name of the affected entry.
        name: String,
        /// The path that is not a git repository.
        path: String,
    },
    /// A slug that breaks the format/reserved rules.
    InvalidSlug {
        /// Display name of the affected entry.
        name: String,
        /// The offending slug.
        slug: String,
        /// Why the slug is invalid.
        reason: String,
    },
    /// A slug used by more than one entry.
    DuplicateSlug {
        /// The slug shared by several entries.
        slug: String,
        /// Display names of the entries sharing the slug.
        names: Vec<String>,
    },
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Issue::MissingPath { name, path } => {
                write!(f, "missing path: {name} ({path})")
            }
            Issue::NotGitRepo { name, path } => {
                write!(f, "not a git repository: {name} ({path})")
            }
            Issue::InvalidSlug { name, slug, reason } => {
                write!(f, "invalid slug '{slug}' on {name}: {reason}")
            }
            Issue::DuplicateSlug { slug, names } => {
                write!(f, "duplicate slug '{slug}': {}", names.join(", "))
            }
        }
    }
}

/// Diagnoses `repos`: missing paths, git entries that are not repositories,
/// malformed slugs and slugs shared by several entries. `exists` and
/// `is_git_repo` are injected so the function is pure (the CLI wires in the
/// real filesystem checks).
pub fn diagnose(
    repos: &[Repo],
    exists: impl Fn(&Path) -> bool,
    is_git_repo: impl Fn(&Path) -> bool,
) -> Vec<Issue> {
    let mut issues = Vec::new();
    for repo in repos {
        if !exists(&repo.path) {
            issues.push(Issue::MissingPath {
                name: repo.display_name(),
                path: repo.path.to_string_lossy().into_owned(),
            });
        } else if repo.kind == RepoKind::Git && !is_git_repo(&repo.path) {
            issues.push(Issue::NotGitRepo {
                name: repo.display_name(),
                path: repo.path.to_string_lossy().into_owned(),
            });
        }
        if let Some(slug) = &repo.slug
            && let Err(error) = slug::validate_format(slug)
        {
            issues.push(Issue::InvalidSlug {
                name: repo.display_name(),
                slug: slug.clone(),
                reason: error.to_string(),
            });
        }
    }
    issues.extend(duplicate_slugs(repos));
    issues
}

/// The slugs shared by more than one entry, as `DuplicateSlug` issues.
fn duplicate_slugs(repos: &[Repo]) -> Vec<Issue> {
    let mut by_slug: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for repo in repos {
        if let Some(slug) = &repo.slug {
            by_slug.entry(slug).or_default().push(repo.display_name());
        }
    }
    by_slug
        .into_iter()
        .filter(|(_, names)| names.len() > 1)
        .map(|(slug, names)| Issue::DuplicateSlug {
            slug: slug.to_string(),
            names,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn repo(name: &str, kind: RepoKind, slug: Option<&str>) -> Repo {
        let mut repo = Repo::new(PathBuf::from(format!("/r/{name}")));
        repo.name = Some(name.to_string());
        repo.kind = kind;
        repo.slug = slug.map(str::to_string);
        repo
    }

    #[test]
    fn flags_missing_not_repo_bad_and_duplicate() {
        let repos = vec![
            repo("gone", RepoKind::Git, None),
            repo("plainfolder", RepoKind::Git, None),
            repo("a", RepoKind::Path, Some("dup")),
            repo("b", RepoKind::Path, Some("dup")),
            repo("c", RepoKind::Path, Some("Bad Slug")),
        ];
        // "gone" is missing; "plainfolder" exists but is not a repo; the rest
        // exist and are repos.
        let exists = |p: &Path| !p.ends_with("gone");
        let is_git_repo = |p: &Path| !p.ends_with("plainfolder");
        let issues = diagnose(&repos, exists, is_git_repo);

        assert!(issues.contains(&Issue::MissingPath {
            name: "gone".to_string(),
            path: "/r/gone".to_string(),
        }));
        assert!(issues.contains(&Issue::NotGitRepo {
            name: "plainfolder".to_string(),
            path: "/r/plainfolder".to_string(),
        }));
        assert!(issues.iter().any(|i| matches!(
            i,
            Issue::InvalidSlug { slug, .. } if slug == "Bad Slug"
        )));
        assert!(issues.iter().any(|i| matches!(
            i,
            Issue::DuplicateSlug { slug, names } if slug == "dup" && names.len() == 2
        )));
    }

    #[test]
    fn clean_entries_have_no_issues() {
        let repos = vec![
            repo("hop", RepoKind::Git, Some("hop")),
            repo("notes", RepoKind::Path, None),
        ];
        let issues = diagnose(&repos, |_| true, |_| true);
        assert!(issues.is_empty());
    }
}
