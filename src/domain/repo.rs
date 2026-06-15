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
    /// A file or folder, told apart at open time by [`classify_path`].
    Path,
}

impl RepoKind {
    /// Parses the config string value, defaulting to [`RepoKind::Git`].
    /// The legacy `folder`/`file`/`dir` values fold into [`RepoKind::Path`].
    pub fn from_config_value(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "folder" | "dir" | "file" | "path" => RepoKind::Path,
            _ => RepoKind::Git,
        }
    }

    /// The lowercase config string for this kind.
    pub fn as_config_value(self) -> &'static str {
        match self {
            RepoKind::Git => "git",
            RepoKind::Path => "path",
        }
    }
}

/// How a [`RepoKind::Path`] entry resolves on disk, deciding how it opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathClass {
    /// A directory: a `cd` target.
    Folder,
    /// A text file: opened in the editor.
    TextFile,
    /// Any other file: opened with the platform's default application.
    OtherFile,
}

/// Classifies `path` for opening: a directory (existing, or a non-existent
/// path written with a trailing `/`) is a folder; otherwise a file, split into
/// text (editor) and other (default app) by [`is_text_file`].
pub fn classify_path(path: &Path, editor_extensions: &[String]) -> PathClass {
    if is_dir_target(path) {
        return PathClass::Folder;
    }
    if is_text_file(path, editor_extensions) {
        PathClass::TextFile
    } else {
        PathClass::OtherFile
    }
}

/// Whether `path` points at a folder: an existing directory, or a
/// non-existent path written with a trailing separator.
pub fn is_dir_target(path: &Path) -> bool {
    if path.is_dir() {
        return true;
    }
    if path.is_file() {
        return false;
    }
    path.to_string_lossy().ends_with(['/', '\\'])
}

/// Whether `path` is treated as a text file (opened in the editor): a file with
/// no extension, or whose extension is in `editor_extensions` (case-insensitive).
pub fn is_text_file(path: &Path, editor_extensions: &[String]) -> bool {
    match path.extension() {
        None => true,
        Some(extension) => {
            let extension = extension.to_string_lossy();
            editor_extensions
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(&extension))
        }
    }
}

/// The error recorded in [`GitInfo`] when an entry's path does not exist. Set
/// by the status refresh and matched on to distinguish a missing path from a
/// path that merely is not a git repository, without re-statting on each frame.
pub const PATH_NOT_FOUND: &str = "path not found";

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

impl GitInfo {
    /// Whether the working tree is clean: no override status, and no changes,
    /// ahead or behind counts.
    pub fn is_clean(&self) -> bool {
        self.raw_status.is_none()
            && self.changes.unwrap_or(0) == 0
            && self.ahead.unwrap_or(0) == 0
            && self.behind.unwrap_or(0) == 0
    }

    /// Whether the gathered info reports the entry's path as missing on disk.
    pub fn is_path_missing(&self) -> bool {
        !self.valid && self.error.as_deref() == Some(PATH_NOT_FOUND)
    }
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
    /// The user section this entry belongs to (Files tab grouping); `None`
    /// means the implicit "Ungrouped" section.
    pub section: Option<String>,
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
            section: None,
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

    /// A short error describing why a git entry is unusable, or `None` when it
    /// is fine. Derived purely from the gathered [`GitInfo`] (populated by the
    /// background status refresh), so it does no filesystem I/O and is cheap to
    /// call once per row each frame: an invalid info yields its message (a
    /// missing path or a path that is not a git repository); a missing or valid
    /// info, or a non-git entry, yields `None`.
    pub fn entry_error(&self) -> Option<String> {
        if self.kind != RepoKind::Git {
            return None;
        }
        let info = self.git_info.as_ref()?;
        if info.valid {
            return None;
        }
        Some(
            info.error
                .clone()
                .unwrap_or_else(|| "not a git repository".to_string()),
        )
    }

    /// The error to display in example mode, derived from `example_git_info`
    /// only: a `None` info or a valid one means no error, an invalid one yields
    /// its message. The real path and live git info are ignored, since example
    /// mode shows curated demo data rather than the filesystem.
    pub fn example_error(&self) -> Option<String> {
        let info = self.example_git_info.as_ref()?;
        if info.valid {
            return None;
        }
        Some(
            info.error
                .clone()
                .unwrap_or_else(|| "not a git repository".to_string()),
        )
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
    fn entry_error_flags_missing_and_invalid() {
        // A git entry without gathered info yet is not flagged (the background
        // refresh drives the error, so there is no per-frame filesystem stat).
        let pending = Repo::new(PathBuf::from("/nope/does-not-exist-xyz"));
        assert!(pending.entry_error().is_none());

        // A git entry whose refresh found the path missing reports it.
        let mut missing = Repo::new(PathBuf::from("/nope"));
        missing.git_info = Some(GitInfo {
            valid: false,
            error: Some(PATH_NOT_FOUND.to_string()),
            ..GitInfo::default()
        });
        assert_eq!(missing.entry_error().as_deref(), Some(PATH_NOT_FOUND));
        assert!(missing.git_info.as_ref().unwrap().is_path_missing());

        // A git path with invalid info reports its error.
        let mut invalid = Repo::new(PathBuf::from("/"));
        invalid.git_info = Some(GitInfo {
            valid: false,
            error: Some("not a git repository".to_string()),
            ..GitInfo::default()
        });
        assert_eq!(
            invalid.entry_error().as_deref(),
            Some("not a git repository")
        );
        assert!(!invalid.git_info.as_ref().unwrap().is_path_missing());

        // A non-git (path) entry has no git error.
        let mut folder = Repo::new(PathBuf::from("/"));
        folder.kind = RepoKind::Path;
        assert!(folder.entry_error().is_none());
    }

    #[test]
    fn repo_kind_round_trips_through_config_value() {
        for kind in [RepoKind::Git, RepoKind::Path] {
            assert_eq!(
                RepoKind::from_config_value(kind.as_config_value()),
                kind
            );
        }
        assert_eq!(RepoKind::from_config_value("unknown"), RepoKind::Git);
        // Legacy values fold into Path.
        assert_eq!(RepoKind::from_config_value("folder"), RepoKind::Path);
        assert_eq!(RepoKind::from_config_value("file"), RepoKind::Path);
    }

    #[test]
    fn classify_path_splits_folder_text_and_other() {
        let exts = vec!["rs".to_string(), "md".to_string()];
        // An existing directory is a folder.
        assert_eq!(classify_path(Path::new("/"), &exts), PathClass::Folder);
        // A non-existent path with a trailing slash is a folder.
        assert_eq!(
            classify_path(Path::new("/nope/dir/"), &exts),
            PathClass::Folder
        );
        // Text extension and no extension open in the editor.
        assert_eq!(
            classify_path(Path::new("/x/main.rs"), &exts),
            PathClass::TextFile
        );
        assert_eq!(
            classify_path(Path::new("/x/Makefile"), &exts),
            PathClass::TextFile
        );
        // Other extensions go to the default app.
        assert_eq!(
            classify_path(Path::new("/x/photo.png"), &exts),
            PathClass::OtherFile
        );
    }
}
