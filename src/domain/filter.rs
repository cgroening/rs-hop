//! Pure predicates for tab membership and the live fuzzy filter.
//!
//! Tab membership decides which entries a tab shows; the fuzzy filter ranks
//! entries against a query across all their visible text. The actual matching
//! uses `nucleo-matcher`, but every input here is plain data so it stays
//! testable.

use std::cmp::Reverse;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::domain::repo::{GitInfo, Repo, RepoKind};

/// The kind of entry a tab groups: the namespace for sections and per-tab view
/// settings. A kind's active and archive views share this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TabKind {
    /// Git repositories.
    Git,
    /// Arbitrary folders and files.
    Files,
}

impl TabKind {
    /// The entry kind this tab holds.
    pub fn repo_kind(self) -> RepoKind {
        match self {
            TabKind::Git => RepoKind::Git,
            TabKind::Files => RepoKind::Path,
        }
    }

    /// The active (non-archived) tab of this kind.
    pub fn active_tab(self) -> Tab {
        match self {
            TabKind::Git => Tab::GitActive,
            TabKind::Files => Tab::FilesActive,
        }
    }

    /// The index used to key per-kind view settings (0 = git, 1 = files).
    pub fn index(self) -> usize {
        match self {
            TabKind::Git => 0,
            TabKind::Files => 1,
        }
    }
}

/// A top-level view: one of two kinds, each in its active or archive state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Tab {
    /// Git repositories that are not archived.
    #[default]
    GitActive,
    /// Archived git repositories.
    GitArchive,
    /// Folders and files that are not archived.
    FilesActive,
    /// Archived folders and files.
    FilesArchive,
}

impl Tab {
    /// The two active tabs, cycled by `Tab`/`Shift+Tab` in this order.
    pub const ACTIVE: [Tab; 2] = [Tab::GitActive, Tab::FilesActive];

    /// All four views, for per-tab focus/first-visit state and persistence.
    pub const ALL: [Tab; 4] = [
        Tab::GitActive,
        Tab::GitArchive,
        Tab::FilesActive,
        Tab::FilesArchive,
    ];

    /// The kind (git or files) this view belongs to.
    pub fn kind(self) -> TabKind {
        match self {
            Tab::GitActive | Tab::GitArchive => TabKind::Git,
            Tab::FilesActive | Tab::FilesArchive => TabKind::Files,
        }
    }

    /// The entry kind this view holds.
    pub fn repo_kind(self) -> RepoKind {
        self.kind().repo_kind()
    }

    /// Whether this view shows archived entries.
    pub fn is_archived(self) -> bool {
        matches!(self, Tab::GitArchive | Tab::FilesArchive)
    }

    /// The active sibling of the same kind (itself when already active).
    pub fn active(self) -> Tab {
        self.kind().active_tab()
    }

    /// The archive sibling of the same kind.
    pub fn archive(self) -> Tab {
        match self.kind() {
            TabKind::Git => Tab::GitArchive,
            TabKind::Files => Tab::FilesArchive,
        }
    }

    /// The same kind's other state (active <-> archive).
    pub fn toggle_archived(self) -> Tab {
        if self.is_archived() {
            self.active()
        } else {
            self.archive()
        }
    }

    /// The index used to key per-kind view settings (0 = git, 1 = files).
    pub fn kind_index(self) -> usize {
        self.kind().index()
    }

    /// The tab title shown in the header bar (the archive suffix is added by
    /// the header renderer, not here).
    pub fn title(self) -> &'static str {
        match self.kind() {
            TabKind::Git => "Git Repos",
            TabKind::Files => "Files and Folders",
        }
    }

    /// A stable key used to persist the active tab.
    pub fn as_key(self) -> &'static str {
        match self {
            Tab::GitActive => "git",
            Tab::GitArchive => "git-archive",
            Tab::FilesActive => "files",
            Tab::FilesArchive => "files-archive",
        }
    }

    /// Parses a persisted [`as_key`](Self::as_key) value. The legacy unified
    /// `"archive"` maps to the git archive; unknown values fall back to the
    /// first tab.
    pub fn from_key(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "git-archive" | "archive" => Tab::GitArchive,
            "files" => Tab::FilesActive,
            "files-archive" => Tab::FilesArchive,
            _ => Tab::GitActive,
        }
    }
}

/// Whether `repo` is shown in `tab`: its kind must match and its archived state
/// must match, so each kind has its own active and archive view.
pub fn belongs_to_tab(repo: &Repo, tab: Tab) -> bool {
    repo.kind == tab.repo_kind() && repo.archived == tab.is_archived()
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
    scored.sort_by_key(|&(_, score)| Reverse(score));
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
        assert!(belongs_to_tab(&git("hop", false), Tab::GitActive));
        assert!(!belongs_to_tab(&git("hop", true), Tab::GitActive));
        assert!(!belongs_to_tab(&folder, Tab::GitActive));
        assert!(belongs_to_tab(&folder, Tab::FilesActive));
    }

    #[test]
    fn archived_entries_go_to_their_kind_archive() {
        let mut folder = git("notes", true);
        folder.kind = RepoKind::Path;
        // An archived git repo lands in the git archive, not the files one.
        assert!(belongs_to_tab(&git("old", true), Tab::GitArchive));
        assert!(!belongs_to_tab(&git("old", true), Tab::FilesArchive));
        assert!(!belongs_to_tab(&git("new", false), Tab::GitArchive));
        // An archived folder lands in the files archive.
        assert!(belongs_to_tab(&folder, Tab::FilesArchive));
        assert!(!belongs_to_tab(&folder, Tab::GitArchive));
    }

    #[test]
    fn tab_helpers_and_keys_round_trip() {
        assert_eq!(Tab::GitActive.toggle_archived(), Tab::GitArchive);
        assert_eq!(Tab::GitArchive.toggle_archived(), Tab::GitActive);
        assert_eq!(Tab::FilesArchive.active(), Tab::FilesActive);
        assert!(Tab::FilesArchive.is_archived());
        assert_eq!(Tab::GitArchive.kind(), TabKind::Git);
        for tab in Tab::ALL {
            assert_eq!(Tab::from_key(tab.as_key()), tab);
        }
        // The legacy unified archive maps to the git archive.
        assert_eq!(Tab::from_key("archive"), Tab::GitArchive);
        assert_eq!(Tab::from_key("bogus"), Tab::GitActive);
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
