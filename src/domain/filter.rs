//! Pure predicates for tab membership and the live fuzzy filter.
//!
//! Tab membership decides which entries a tab shows; the fuzzy filter ranks
//! entries against a query across all their visible text. The actual matching
//! uses `nucleo-matcher`, but every input here is plain data so it stays
//! testable.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::domain::repo::{GitInfo, Repo, RepoKind};

/// The three top-level views.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Tab {
    /// Git repositories that are not archived.
    #[default]
    GitRepos,
    /// Arbitrary folders and files that are not archived.
    FilesAndFolders,
    /// Archived entries of any kind.
    Archive,
}

impl Tab {
    /// All tabs in display (number-key) order.
    pub const ALL: [Tab; 3] =
        [Tab::GitRepos, Tab::FilesAndFolders, Tab::Archive];

    /// The tab title shown in the header bar.
    pub fn title(self) -> &'static str {
        match self {
            Tab::GitRepos => "Git Repos",
            Tab::FilesAndFolders => "Files and Folders",
            Tab::Archive => "Archiv",
        }
    }

    /// A stable key used to persist the active tab.
    pub fn as_key(self) -> &'static str {
        match self {
            Tab::GitRepos => "git",
            Tab::FilesAndFolders => "files",
            Tab::Archive => "archive",
        }
    }

    /// Parses a persisted [`as_key`](Self::as_key) value, defaulting to the
    /// first tab.
    pub fn from_key(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "files" => Tab::FilesAndFolders,
            "archive" => Tab::Archive,
            _ => Tab::GitRepos,
        }
    }
}

/// Whether `repo` is shown in `tab`. Archived entries appear only in the
/// archive tab; otherwise git entries go to Git Repos and folders/files to
/// Files and Folders.
pub fn belongs_to_tab(repo: &Repo, tab: Tab) -> bool {
    match tab {
        Tab::Archive => repo.archived,
        Tab::GitRepos => !repo.archived && repo.kind == RepoKind::Git,
        Tab::FilesAndFolders => !repo.archived && repo.kind == RepoKind::Path,
    }
}

/// The text a fuzzy query is matched against: name, slug, branch, status,
/// GitHub name and path, joined by spaces.
pub fn searchable_text(repo: &Repo) -> String {
    let mut parts = vec![repo.display_name()];
    if let Some(slug) = &repo.slug {
        parts.push(slug.clone());
    }
    if let Some(info) =
        repo.git_info.as_ref().or(repo.example_git_info.as_ref())
    {
        push_git_parts(&mut parts, info);
    }
    parts.push(repo.path.to_string_lossy().into_owned());
    parts.join(" ")
}

/// Appends the searchable parts of `info` (branch, GitHub name) to `parts`.
fn push_git_parts(parts: &mut Vec<String>, info: &GitInfo) {
    if let Some(branch) = &info.current_branch_name {
        parts.push(branch.clone());
    }
    if let Some(name) = &info.github_repo_name {
        parts.push(name.clone());
    }
}

/// Returns the indices of `repos` matching `query`, best match first. An empty
/// query keeps the input order and includes everything.
pub fn fuzzy_indices(repos: &[Repo], query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..repos.len()).collect();
    }
    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern =
        Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);

    let mut scored: Vec<(usize, u32)> = Vec::new();
    let mut buf = Vec::new();
    for (index, repo) in repos.iter().enumerate() {
        let haystack = searchable_text(repo);
        let utf32 = Utf32Str::new(&haystack, &mut buf);
        if let Some(score) = pattern.score(utf32, &mut matcher) {
            scored.push((index, score));
        }
    }
    // Highest score first; ties keep the earlier (already sorted) entry.
    scored.sort_by(|a, b| b.1.cmp(&a.1));
    scored.into_iter().map(|(index, _)| index).collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn git(name: &str, archived: bool) -> Repo {
        let mut repo = Repo::new(PathBuf::from(format!("/code/{name}")));
        repo.name = Some(name.to_string());
        repo.archived = archived;
        repo
    }

    #[test]
    fn git_tab_excludes_archived_and_non_git() {
        let mut folder = git("notes", false);
        folder.kind = RepoKind::Path;
        assert!(belongs_to_tab(&git("hop", false), Tab::GitRepos));
        assert!(!belongs_to_tab(&git("hop", true), Tab::GitRepos));
        assert!(!belongs_to_tab(&folder, Tab::GitRepos));
        assert!(belongs_to_tab(&folder, Tab::FilesAndFolders));
    }

    #[test]
    fn archive_tab_shows_only_archived() {
        assert!(belongs_to_tab(&git("old", true), Tab::Archive));
        assert!(!belongs_to_tab(&git("new", false), Tab::Archive));
    }

    #[test]
    fn searchable_text_includes_name_and_path() {
        let text = searchable_text(&git("mdtask", false));
        assert!(text.contains("mdtask"));
        assert!(text.contains("/code/mdtask"));
    }

    #[test]
    fn empty_query_keeps_all_in_order() {
        let repos = vec![git("a", false), git("b", false)];
        assert_eq!(fuzzy_indices(&repos, "   "), vec![0, 1]);
    }

    #[test]
    fn fuzzy_query_filters_and_ranks() {
        let repos =
            vec![git("alpha", false), git("beta", false), git("gamma", false)];
        let hits = fuzzy_indices(&repos, "beta");
        assert_eq!(hits.first(), Some(&1));
        assert!(!hits.contains(&0) || hits[0] == 1);
    }
}
