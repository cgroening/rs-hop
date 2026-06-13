//! Comment-preserving rewrites of the `[[repos]]` array in `config.toml`.
//!
//! In-app management (add/edit/delete/favourite/archive/slug) persists the full
//! entry list here. Editing goes through `toml_edit`, so the settings block and
//! its comments survive; only the `[[repos]]` array is replaced. Per-entry
//! comments in the array are not preserved (the array is rebuilt). Runtime
//! fields (live git info, usage counters) are never written.

use std::fs;
use std::path::Path;

use toml_edit::{ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};

use crate::domain::error::{Error, Result};
use crate::domain::repo::{GitInfo, Repo};

/// Writes `repos` into the `config.toml` at `path`, preserving the settings
/// block and creating the file (and parent directory) when absent.
///
/// # Errors
/// Returns an error if the file cannot be read, is not valid TOML, or cannot be
/// written.
pub fn save_repos(path: &Path, repos: &[Repo]) -> Result<()> {
    let existing = if path.exists() {
        fs::read_to_string(path).map_err(|e| {
            Error::config(path.display().to_string(), e.to_string())
        })?
    } else {
        String::new()
    };
    let updated = repos_to_toml(&existing, repos)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| Error::io("create config directory", e))?;
    }
    fs::write(path, updated).map_err(|e| Error::io("write config.toml", e))
}

/// Returns `existing` with its `[[repos]]` array replaced by `repos`, leaving
/// the rest of the document (settings and their comments) untouched.
///
/// # Errors
/// Returns [`Error::Config`] when `existing` is not valid TOML.
pub fn repos_to_toml(existing: &str, repos: &[Repo]) -> Result<String> {
    let mut doc = existing
        .parse::<DocumentMut>()
        .map_err(|e| Error::config("config.toml", e.to_string()))?;
    let mut array = ArrayOfTables::new();
    for repo in repos {
        array.push(repo_table(repo));
    }
    doc["repos"] = Item::ArrayOfTables(array);
    Ok(doc.to_string())
}

/// Writes the ordered section names into the `config.toml` at `path`,
/// preserving the rest of the document and creating the file when absent.
///
/// # Errors
/// Returns an error if the file cannot be read, is not valid TOML, or cannot be
/// written.
pub fn save_sections(path: &Path, sections: &[String]) -> Result<()> {
    let existing = if path.exists() {
        fs::read_to_string(path).map_err(|e| {
            Error::config(path.display().to_string(), e.to_string())
        })?
    } else {
        String::new()
    };
    let updated = sections_to_toml(&existing, sections)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| Error::io("create config directory", e))?;
    }
    fs::write(path, updated).map_err(|e| Error::io("write config.toml", e))
}

/// Returns `existing` with its top-level `sections` array set to `sections`
/// (removed when empty), leaving the rest of the document untouched.
///
/// # Errors
/// Returns [`Error::Config`] when `existing` is not valid TOML.
pub fn sections_to_toml(existing: &str, sections: &[String]) -> Result<String> {
    let mut doc = existing
        .parse::<DocumentMut>()
        .map_err(|e| Error::config("config.toml", e.to_string()))?;
    if sections.is_empty() {
        doc.as_table_mut().remove("sections");
        return Ok(doc.to_string());
    }
    let mut array = toml_edit::Array::new();
    for name in sections {
        array.push(name.clone());
    }
    doc["sections"] = Item::Value(Value::Array(array));
    Ok(doc.to_string())
}

/// Builds one `[[repos]]` table from a domain [`Repo`] (stored fields only).
fn repo_table(repo: &Repo) -> Table {
    let mut table = Table::new();
    if let Some(name) = &repo.name {
        table["name"] = toml_edit::value(name.clone());
    }
    table["path"] = toml_edit::value(repo.path.to_string_lossy().into_owned());
    if let Some(slug) = &repo.slug {
        table["slug"] = toml_edit::value(slug.clone());
    }
    table["kind"] = toml_edit::value(repo.kind.as_config_value());
    if repo.fav {
        table["fav"] = toml_edit::value(true);
    }
    if repo.archived {
        table["archived"] = toml_edit::value(true);
    }
    if let Some(section) = &repo.section {
        table["section"] = toml_edit::value(section.clone());
    }
    if let Some(info) = &repo.example_git_info {
        table["example_git_info"] =
            Item::Value(Value::InlineTable(example_info_inline(info)));
    }
    table
}

/// Builds the inline `example_git_info` table from a [`GitInfo`].
fn example_info_inline(info: &GitInfo) -> InlineTable {
    let mut inline = InlineTable::new();
    if let Some(branch) = &info.current_branch_name {
        inline.insert("current_branch_name", Value::from(branch.clone()));
    }
    if let Some(status) = &info.raw_status {
        inline.insert("status", Value::from(status.clone()));
    }
    if let Some(name) = &info.github_repo_name {
        inline.insert("github_repo_name", Value::from(name.clone()));
    }
    inline
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::domain::repo::RepoKind;

    const EXISTING: &str = r#"# hop settings
git_program = "lazygit"  # the tool

[[repos]]
name = "old"
path = "/old"
"#;

    fn repo(name: &str, path: &str) -> Repo {
        let mut repo = Repo::new(PathBuf::from(path));
        repo.name = Some(name.to_string());
        repo
    }

    #[test]
    fn preserves_settings_and_comments() {
        let mut fav = repo("hop", "/code/hop");
        fav.fav = true;
        fav.slug = Some("hop".to_string());
        let out = repos_to_toml(EXISTING, &[fav]).unwrap();
        assert!(out.contains("git_program = \"lazygit\"  # the tool"));
        assert!(out.contains("# hop settings"));
        assert!(out.contains("slug = \"hop\""));
        assert!(out.contains("fav = true"));
        // The previous entry is gone (the array was rebuilt).
        assert!(!out.contains("name = \"old\""));
    }

    #[test]
    fn writes_kind_and_archived() {
        let mut folder = repo("notes", "/notes");
        folder.kind = RepoKind::Folder;
        folder.archived = true;
        let out = repos_to_toml("", &[folder]).unwrap();
        assert!(out.contains("kind = \"folder\""));
        assert!(out.contains("archived = true"));
    }

    #[test]
    fn round_trips_through_a_parser() {
        let out =
            repos_to_toml("", &[repo("a", "/a"), repo("b", "/b")]).unwrap();
        let parsed: toml::Value = toml::from_str(&out).unwrap();
        let repos = parsed.get("repos").and_then(|r| r.as_array()).unwrap();
        assert_eq!(repos.len(), 2);
    }

    #[test]
    fn writes_entry_section() {
        let mut entry = repo("notes", "/notes");
        entry.section = Some("Work".to_string());
        let out = repos_to_toml("", &[entry]).unwrap();
        assert!(out.contains("section = \"Work\""));
    }

    #[test]
    fn sections_array_is_written_and_cleared() {
        let with = sections_to_toml(
            EXISTING,
            &["Work".to_string(), "Personal".to_string()],
        )
        .unwrap();
        assert!(with.contains("sections = [\"Work\", \"Personal\"]"));
        // Settings survive the rewrite.
        assert!(with.contains("git_program = \"lazygit\"  # the tool"));
        // An empty list removes the key.
        let cleared = sections_to_toml(&with, &[]).unwrap();
        assert!(!cleared.contains("sections ="));
    }
}
