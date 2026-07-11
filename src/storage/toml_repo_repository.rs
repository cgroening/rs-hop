//! TOML-backed [`RepoRepository`]: reads the `[[repos]]` array and writes it
//! back through the comment-preserving [`crate::config::writer`].

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

use crate::config::writer;
use crate::domain::error::{Error, Result};
use crate::domain::repo::{GitInfo, Repo, RepoKind};
use crate::storage::repository::RepoRepository;
use crate::util::paths::expand_tilde;

/// Reads/writes entries in the `config.toml` at `path`.
pub struct TomlRepoRepository {
    path: PathBuf,
}

impl TomlRepoRepository {
    /// Creates a repository backed by the file at `path`.
    pub fn new(path: PathBuf) -> Self {
        TomlRepoRepository { path }
    }
}

/// The `[[repos]]` array and `sections` order wrapper for reading.
#[derive(Debug, Default, Deserialize)]
struct RawFile {
    #[serde(default)]
    repos: Vec<RawRepo>,
    #[serde(default)]
    sections: Vec<String>,
}

/// One stored entry as read from TOML.
#[derive(Debug, Deserialize)]
struct RawRepo {
    name: Option<String>,
    path: String,
    slug: Option<String>,
    #[serde(default)]
    fav: bool,
    #[serde(default)]
    archived: bool,
    section: Option<String>,
    /// `None` (field absent) resolves to a kind-dependent default in
    /// [`RawRepo::into_repo`]; an explicit value overrides it.
    include_in_backup: Option<bool>,
    kind: Option<String>,
    example_git_info: Option<RawGitInfo>,
}

/// Stored demo git info for an entry.
#[derive(Debug, Deserialize)]
struct RawGitInfo {
    current_branch_name: Option<String>,
    status: Option<String>,
    github_repo_name: Option<String>,
}

impl RawRepo {
    /// Builds a domain [`Repo`], expanding `~` and resolving the kind.
    fn into_repo(self) -> Repo {
        let mut repo = Repo::new(expand_tilde(&self.path));
        repo.name = self.name;
        repo.slug = self.slug;
        repo.fav = self.fav;
        repo.archived = self.archived;
        repo.section = self.section.filter(|s| !s.trim().is_empty());
        repo.kind = self
            .kind
            .map_or(RepoKind::Git, |k| RepoKind::from_config_value(&k));
        repo.include_in_backup =
            self.include_in_backup.unwrap_or(repo.kind == RepoKind::Git);
        repo.example_git_info =
            self.example_git_info.map(RawGitInfo::into_info);
        repo
    }
}

impl RawGitInfo {
    /// Builds a [`GitInfo`] carrying the verbatim status string.
    fn into_info(self) -> GitInfo {
        GitInfo {
            valid: true,
            current_branch_name: self.current_branch_name,
            raw_status: self.status,
            github_repo_name: self.github_repo_name,
            ..GitInfo::default()
        }
    }
}

impl RepoRepository for TomlRepoRepository {
    fn find_all(&self) -> Result<Vec<Repo>> {
        Ok(self
            .read_raw()?
            .repos
            .into_iter()
            .map(RawRepo::into_repo)
            .collect())
    }

    fn save_all(&self, repos: &[Repo]) -> Result<()> {
        writer::save_repos(&self.path, repos)
    }

    fn find_sections(&self) -> Result<Vec<String>> {
        Ok(self.read_raw()?.sections)
    }

    fn save_sections(&self, sections: &[String]) -> Result<()> {
        writer::save_sections(&self.path, sections)
    }
}

impl TomlRepoRepository {
    /// Parses the config file into the raw wrapper, treating a missing file as
    /// empty.
    fn read_raw(&self) -> Result<RawFile> {
        if !self.path.exists() {
            return Ok(RawFile::default());
        }
        let text = fs::read_to_string(&self.path).map_err(|e| {
            Error::config(self.path.display().to_string(), e.to_string())
        })?;
        toml::from_str(&text).map_err(|e| {
            Error::config(self.path.display().to_string(), e.to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_entries_with_defaults_and_kinds() {
        let raw: RawFile = toml::from_str(
            r#"
[[repos]]
name = "hop"
path = "/code/hop"
slug = "hop"
fav = true

[[repos]]
path = "/notes"
kind = "folder"
archived = true
example_git_info = { current_branch_name = "main", status = "✓" }
"#,
        )
        .unwrap();
        let repos: Vec<Repo> =
            raw.repos.into_iter().map(RawRepo::into_repo).collect();
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].slug.as_deref(), Some("hop"));
        assert!(repos[0].fav);
        assert_eq!(repos[0].kind, RepoKind::Git);
        assert_eq!(repos[1].kind, RepoKind::Path);
        assert!(repos[1].archived);
        // Absent backup flag defaults per kind: git included, path excluded.
        assert!(repos[0].include_in_backup);
        assert!(!repos[1].include_in_backup);
        let info = repos[1].example_git_info.as_ref().unwrap();
        assert_eq!(info.raw_status.as_deref(), Some("✓"));
    }

    #[test]
    fn reads_explicit_include_in_backup_overriding_kind_default() {
        let raw: RawFile = toml::from_str(
            r#"
[[repos]]
path = "/code/hop"
include_in_backup = false

[[repos]]
path = "/notes"
kind = "folder"
include_in_backup = true
"#,
        )
        .unwrap();
        let repos: Vec<Repo> =
            raw.repos.into_iter().map(RawRepo::into_repo).collect();
        assert!(!repos[0].include_in_backup);
        assert!(repos[1].include_in_backup);
    }

    #[test]
    fn reads_section_field_and_sections_order() {
        let raw: RawFile = toml::from_str(
            r#"
sections = ["Work", "Personal"]

[[repos]]
path = "/notes"
kind = "folder"
section = "Work"

[[repos]]
path = "/scratch"
kind = "folder"
section = "   "
"#,
        )
        .unwrap();
        assert_eq!(raw.sections, ["Work", "Personal"]);
        let repos: Vec<Repo> =
            raw.repos.into_iter().map(RawRepo::into_repo).collect();
        assert_eq!(repos[0].section.as_deref(), Some("Work"));
        // Blank sections normalise to None (Ungrouped).
        assert_eq!(repos[1].section, None);
    }
}
