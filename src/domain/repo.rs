//! Core entities: a [`Repo`] entry and its computed [`GitInfo`].
//!
//! These are plain data with pure helpers only - no I/O, no config, no UI.
//! Git status is gathered by the storage layer and the display string is built
//! by the presentation layer; here we only model the values.

use std::path::{Path, PathBuf};

/// What an entry points at, deciding how it is opened and which tab shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepoKind {
    /// A git repository (opened in the configured git tool).
    #[default]
    Git,
    /// An arbitrary folder (a `cd` target).
    Folder,
    /// A single file (opened in `$EDITOR`).
    File,
}

impl RepoKind {
    /// Parses the config string value, defaulting to [`RepoKind::Git`].
    pub fn from_config_value(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "folder" | "dir" => RepoKind::Folder,
            "file" => RepoKind::File,
            _ => RepoKind::Git,
        }
    }

    /// The lowercase config string for this kind.
    pub fn as_config_value(self) -> &'static str {
        match self {
            RepoKind::Git => "git",
            RepoKind::Folder => "folder",
            RepoKind::File => "file",
        }
    }
}

/// Git status gathered for a repository.
///
/// `raw_status` carries a verbatim status string (used by example mode and the
/// cache); when it is `None` the presentation layer builds the string from the
/// structured `changes`/`ahead`/`behind` counts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitInfo {
    /// Whether the path is a usable git repository.
    pub valid: bool,
    /// Why the repository could not be read, when `valid` is false.
    pub error: Option<String>,
    /// Current branch name (or detached marker), when known.
    pub current_branch_name: Option<String>,
    /// Number of uncommitted changes, when known.
    pub changes: Option<u32>,
    /// Commits ahead of the upstream, when an upstream is configured.
    pub ahead: Option<u32>,
    /// Commits behind the upstream, when an upstream is configured.
    pub behind: Option<u32>,
    /// GitHub repo name (already stripped of the configured username), if any.
    pub github_repo_name: Option<String>,
    /// A verbatim status string overriding the structured counts.
    pub raw_status: Option<String>,
}

/// A managed entry: a git repository, an arbitrary folder, or a file.
///
/// `git_info`, `last_used` and `open_count` are runtime fields hydrated from the
/// status cache and the usage state; they are not part of the stored config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    /// Display name; when `None`, [`Repo::display_name`] falls back to the path.
    pub name: Option<String>,
    /// The filesystem path this entry points at.
    pub path: PathBuf,
    /// Optional shortcut used by `hop <slug>` and validated for uniqueness.
    pub slug: Option<String>,
    /// Whether the entry is a favourite (sorted first).
    pub fav: bool,
    /// Whether the entry lives in the archive tab.
    pub archived: bool,
    /// What the entry points at.
    pub kind: RepoKind,
    /// Demo git info shown when `example_mode` is on.
    pub example_git_info: Option<GitInfo>,
    /// Live git info (runtime; not persisted).
    pub git_info: Option<GitInfo>,
    /// Last-used time as unix seconds (runtime; from the usage state).
    pub last_used: Option<i64>,
    /// How often the entry was opened (runtime; from the usage state).
    pub open_count: u64,
}

impl Repo {
    /// Creates a git entry at `path` with all other fields at their defaults.
    pub fn new(path: PathBuf) -> Self {
        Repo {
            name: None,
            path,
            slug: None,
            fav: false,
            archived: false,
            kind: RepoKind::Git,
            example_git_info: None,
            git_info: None,
            last_used: None,
            open_count: 0,
        }
    }

    /// The name to show: the explicit `name`, else the path's final component,
    /// else the whole path as a string.
    pub fn display_name(&self) -> String {
        if let Some(name) = &self.name
            && !name.trim().is_empty()
        {
            return name.clone();
        }
        basename(&self.path)
    }

    /// Whether the path currently exists on disk.
    pub fn path_exists(&self) -> bool {
        self.path.exists()
    }
}

/// The final path component as a string, or the whole path when it has none
/// (e.g. a root). A trailing slash is ignored so `~/foo/` yields `foo`.
pub fn basename(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_prefers_explicit_name() {
        let mut repo = Repo::new(PathBuf::from("/a/b/project"));
        repo.name = Some("My Project".to_string());
        assert_eq!(repo.display_name(), "My Project");
    }

    #[test]
    fn display_name_falls_back_to_basename() {
        let repo = Repo::new(PathBuf::from("/a/b/project"));
        assert_eq!(repo.display_name(), "project");
    }

    #[test]
    fn display_name_ignores_trailing_slash() {
        let repo = Repo::new(PathBuf::from("/a/b/project/"));
        assert_eq!(repo.display_name(), "project");
    }

    #[test]
    fn display_name_ignores_blank_name() {
        let mut repo = Repo::new(PathBuf::from("/a/b/project"));
        repo.name = Some("   ".to_string());
        assert_eq!(repo.display_name(), "project");
    }

    #[test]
    fn repo_kind_round_trips_through_config_value() {
        for kind in [RepoKind::Git, RepoKind::Folder, RepoKind::File] {
            assert_eq!(
                RepoKind::from_config_value(kind.as_config_value()),
                kind
            );
        }
        assert_eq!(RepoKind::from_config_value("unknown"), RepoKind::Git);
    }
}
