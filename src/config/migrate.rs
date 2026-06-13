//! One-time import of a git-repo-jumper `config.yaml` into hop's `config.toml`.
//!
//! The settings map across directly; the only semantic change is the `show`
//! flag, which becomes `archived` inverted (a hidden `show: false` entry
//! becomes `archived: true`). The produced TOML is what hop reads from then on.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use toml_edit::{ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};

use crate::domain::error::{Error, Result};

/// The git-repo-jumper YAML configuration.
#[derive(Debug, Deserialize)]
struct YamlConfig {
    #[serde(rename = "git-program")]
    git_program: Option<String>,
    #[serde(rename = "github-username")]
    github_username: Option<String>,
    #[serde(rename = "example-mode", default)]
    example_mode: bool,
    #[serde(rename = "repo-selector-column-widths", default)]
    column_widths: HashMap<String, YamlWidth>,
    #[serde(default)]
    repos: Vec<YamlRepo>,
}

/// A column width as a bare integer or a `{ min, max }` mapping.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum YamlWidth {
    Fixed(usize),
    Range {
        min: Option<usize>,
        max: Option<usize>,
    },
}

/// One repository entry in the YAML.
#[derive(Debug, Deserialize)]
struct YamlRepo {
    name: Option<String>,
    path: String,
    #[serde(default)]
    fav: bool,
    show: Option<bool>,
    #[serde(rename = "example-git-info")]
    example_git_info: Option<YamlGitInfo>,
}

/// Demo git info attached to an entry.
#[derive(Debug, Deserialize)]
struct YamlGitInfo {
    #[serde(rename = "current-branch-name")]
    current_branch_name: Option<String>,
    status: Option<String>,
    #[serde(rename = "github-repo-name")]
    github_repo_name: Option<String>,
}

/// Reads the YAML at `from` and writes the converted TOML to `to`, creating
/// parent directories as needed.
///
/// # Errors
/// Returns an error if the source cannot be read, is not valid YAML, or the
/// destination cannot be written.
pub fn migrate_file(from: &Path, to: &Path) -> Result<()> {
    let yaml = fs::read_to_string(from).map_err(|e| {
        Error::config(from.display().to_string(), e.to_string())
    })?;
    let toml = yaml_to_toml(&yaml)?;
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| Error::io("create config directory", e))?;
    }
    fs::write(to, toml).map_err(|e| Error::io("write config.toml", e))
}

/// Converts a git-repo-jumper `config.yaml` body into a hop `config.toml` body.
///
/// # Errors
/// Returns [`Error::Config`] when the input is not valid YAML.
pub fn yaml_to_toml(yaml: &str) -> Result<String> {
    let config: YamlConfig = serde_yaml::from_str(yaml)
        .map_err(|e| Error::config("config.yaml", e.to_string()))?;

    let mut doc = DocumentMut::new();
    if let Some(program) = &config.git_program {
        doc["git_program"] = toml_edit::value(program.clone());
    }
    if let Some(username) = &config.github_username {
        doc["github_username"] = toml_edit::value(username.clone());
    }
    doc["example_mode"] = toml_edit::value(config.example_mode);

    let mut icons = Table::new();
    icons["variant"] = toml_edit::value("nerdfont");
    doc["icons"] = Item::Table(icons);

    if !config.column_widths.is_empty() {
        doc["column_widths"] = Item::Table(column_widths_table(&config));
    }

    doc["repos"] = Item::ArrayOfTables(repos_array(&config.repos));

    let header = "# hop configuration (imported from git-repo-jumper).\n\
                  # Entries hidden via `show: false` are now archived.\n\n";
    Ok(format!("{header}{doc}"))
}

/// Builds the `[column_widths]` table, preserving integer vs `{ min, max }`.
fn column_widths_table(config: &YamlConfig) -> Table {
    let mut table = Table::new();
    for (key, width) in &config.column_widths {
        table[key] = match width {
            YamlWidth::Fixed(min) => toml_edit::value(*min as i64),
            YamlWidth::Range { min, max } => {
                let mut inline = InlineTable::new();
                if let Some(min) = min {
                    inline.insert("min", Value::from(*min as i64));
                }
                if let Some(max) = max {
                    inline.insert("max", Value::from(*max as i64));
                }
                Item::Value(Value::InlineTable(inline))
            }
        };
    }
    table
}

/// Builds the `[[repos]]` array of tables.
fn repos_array(repos: &[YamlRepo]) -> ArrayOfTables {
    let mut array = ArrayOfTables::new();
    for repo in repos {
        array.push(repo_table(repo));
    }
    array
}

/// Builds one `[[repos]]` table, inverting `show` into `archived`.
fn repo_table(repo: &YamlRepo) -> Table {
    let mut table = Table::new();
    if let Some(name) = &repo.name {
        table["name"] = toml_edit::value(name.clone());
    }
    table["path"] = toml_edit::value(repo.path.clone());
    table["kind"] = toml_edit::value("git");
    if repo.fav {
        table["fav"] = toml_edit::value(true);
    }
    // `show: false` (hidden) becomes archived; absent `show` defaults to shown.
    let archived = !repo.show.unwrap_or(true);
    if archived {
        table["archived"] = toml_edit::value(true);
    }
    if let Some(info) = &repo.example_git_info {
        table["example_git_info"] =
            Item::Value(Value::InlineTable(example_info_inline(info)));
    }
    table
}

/// Builds the inline `example_git_info` table for an entry.
fn example_info_inline(info: &YamlGitInfo) -> InlineTable {
    let mut inline = InlineTable::new();
    if let Some(branch) = &info.current_branch_name {
        inline.insert("current_branch_name", Value::from(branch.clone()));
    }
    if let Some(status) = &info.status {
        inline.insert("status", Value::from(status.clone()));
    }
    if let Some(name) = &info.github_repo_name {
        inline.insert("github_repo_name", Value::from(name.clone()));
    }
    inline
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
git-program: lazygit
github-username: cgroening
repo-selector-column-widths:
  name: 30
  current_branch_name:
    min: 10
    max: 14
repos:
  - name: Visible Fav
    path: ~/code/a
    fav: true
  - name: Hidden One
    path: ~/code/b
    show: false
  - path: ~/code/c
"#;

    #[test]
    fn converts_settings_and_inverts_show() {
        let toml = yaml_to_toml(SAMPLE).unwrap();
        assert!(toml.contains("git_program = \"lazygit\""));
        assert!(toml.contains("github_username = \"cgroening\""));
        // The hidden entry becomes archived; the others do not.
        assert_eq!(toml.matches("archived = true").count(), 1);
        assert!(toml.contains("fav = true"));
    }

    #[test]
    fn output_parses_back_as_valid_toml() {
        let toml = yaml_to_toml(SAMPLE).unwrap();
        let parsed: toml::Value = toml::from_str(&toml).unwrap();
        let repos = parsed.get("repos").and_then(|r| r.as_array()).unwrap();
        assert_eq!(repos.len(), 3);
        // The entry without a name keeps just its path (auto-naming later).
        let third = &repos[2];
        assert!(third.get("name").is_none());
        assert_eq!(
            third.get("path").and_then(|p| p.as_str()),
            Some("~/code/c")
        );
    }

    #[test]
    fn column_widths_round_trip() {
        let toml = yaml_to_toml(SAMPLE).unwrap();
        let parsed: toml::Value = toml::from_str(&toml).unwrap();
        let widths = parsed.get("column_widths").unwrap();
        assert_eq!(widths.get("name").and_then(|v| v.as_integer()), Some(30));
        let branch = widths.get("current_branch_name").unwrap();
        assert_eq!(branch.get("min").and_then(|v| v.as_integer()), Some(10));
        assert_eq!(branch.get("max").and_then(|v| v.as_integer()), Some(14));
    }
}
