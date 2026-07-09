//! Persists small UI state across runs (sort, tab, panel, column set), as a
//! TOML file in the state directory. Loading is best-effort: a missing or
//! corrupt file falls back to the defaults.
//!
//! The panel's position and its two sizes are separate fields on purpose. One
//! `size` number would mean a percentage next to the list and a row count below
//! it - the same field carrying two units, which a reader cannot tell apart.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::domain::error::{Error, Result};
use crate::domain::filter::Tab;
use crate::domain::sort::{SortDir, SortMode};

/// Default width of the panel when it sits to the right of the list, in percent.
pub const DEFAULT_PREVIEW_WIDTH_PCT: u16 = 40;
/// Default height of the panel when it sits below the list, in rows.
pub const DEFAULT_PREVIEW_HEIGHT_ROWS: u16 = 9;

/// The restored UI state. `preview` and `columns` are raw keys: their enums
/// live in the TUI layer, which storage must not depend on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiState {
    /// The list sort mode.
    pub sort: SortMode,
    /// Which way the sort runs.
    pub sort_dir: SortDir,
    /// The active tab.
    pub tab: Tab,
    /// Whether slugs are shown inline after the entry name.
    pub show_slugs: bool,
    /// The detail-panel layout key (`"off"`, `"right"` or `"bottom"`).
    pub preview: String,
    /// Panel width when it sits to the right of the list, in percent.
    pub preview_width_pct: u16,
    /// Panel height when it sits below the list, in rows.
    pub preview_height_rows: u16,
    /// The active column-set key (`"standard"`, `"code"` or `"activity"`).
    pub columns: String,
    /// Whether the shortcut-hint footer is shown (toggled with `F1`).
    pub hints_visible: bool,
}

impl Default for UiState {
    /// Everything off except the hint footer, which starts out shown.
    fn default() -> Self {
        UiState {
            sort: SortMode::default(),
            sort_dir: SortDir::default(),
            tab: Tab::default(),
            show_slugs: false,
            preview: "off".to_string(),
            preview_width_pct: DEFAULT_PREVIEW_WIDTH_PCT,
            preview_height_rows: DEFAULT_PREVIEW_HEIGHT_ROWS,
            columns: "standard".to_string(),
            hints_visible: true,
        }
    }
}

/// The on-disk UI state document. Every field is optional, so a file written by
/// an older version still loads and only the new keys fall back to defaults.
#[derive(Debug, Default, Serialize, Deserialize)]
struct UiStateDoc {
    sort: Option<String>,
    sort_dir: Option<String>,
    tab: Option<String>,
    show_slugs: Option<bool>,
    preview: Option<String>,
    preview_width_pct: Option<u16>,
    preview_height_rows: Option<u16>,
    columns: Option<String>,
    hints_visible: Option<bool>,
}

/// Loads the persisted UI state, defaulting when the file is missing/corrupt.
pub fn load(path: &Path) -> UiState {
    let Ok(text) = fs::read_to_string(path) else {
        return UiState::default();
    };
    let Ok(doc) = toml::from_str::<UiStateDoc>(&text) else {
        return UiState::default();
    };
    let defaults = UiState::default();
    UiState {
        sort: doc
            .sort
            .as_deref()
            .map_or_else(SortMode::default, SortMode::from_config_value),
        sort_dir: doc
            .sort_dir
            .as_deref()
            .map_or_else(SortDir::default, SortDir::from_config_value),
        tab: doc.tab.as_deref().map_or_else(Tab::default, Tab::from_key),
        show_slugs: doc.show_slugs.unwrap_or(false),
        // A file written before the panel gained a size still names its
        // position here, so an existing panel keeps its side.
        preview: doc.preview.unwrap_or(defaults.preview),
        preview_width_pct: doc
            .preview_width_pct
            .unwrap_or(DEFAULT_PREVIEW_WIDTH_PCT),
        preview_height_rows: doc
            .preview_height_rows
            .unwrap_or(DEFAULT_PREVIEW_HEIGHT_ROWS),
        columns: doc.columns.unwrap_or(defaults.columns),
        hints_visible: doc.hints_visible.unwrap_or(true),
    }
}

/// Saves the UI state to `path`.
///
/// # Errors
/// Returns an error if the directory or file cannot be written.
pub fn save(path: &Path, state: &UiState) -> Result<()> {
    let doc = UiStateDoc {
        sort: Some(state.sort.label().to_string()),
        sort_dir: Some(state.sort_dir.label().to_string()),
        tab: Some(state.tab.as_key().to_string()),
        show_slugs: Some(state.show_slugs),
        preview: Some(state.preview.clone()),
        preview_width_pct: Some(state.preview_width_pct),
        preview_height_rows: Some(state.preview_height_rows),
        columns: Some(state.columns.clone()),
        hints_visible: Some(state.hints_visible),
    };
    let text = toml::to_string_pretty(&doc)
        .map_err(|e| Error::invalid(format!("serialise ui state: {e}")))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| Error::io("create state directory", e))?;
    }
    fs::write(path, text).map_err(|e| Error::io("write ui state", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!("hop-uistate-{tag}-{}", std::process::id()))
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = temp("round");
        let file = dir.join("ui-state.toml");
        let state = UiState {
            sort: SortMode::Loc,
            sort_dir: SortDir::Desc,
            tab: Tab::Archive,
            show_slugs: true,
            preview: "right".to_string(),
            preview_width_pct: 55,
            preview_height_rows: 14,
            columns: "code".to_string(),
            hints_visible: false,
        };
        save(&file, &state).unwrap();
        assert_eq!(load(&file), state);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_from_before_the_panel_had_a_size_keeps_its_position() {
        // Exactly what a pre-upgrade `ui-state.toml` looks like. Dropping the
        // old `preview` key here would silently turn every user's panel off.
        let dir = temp("legacy");
        let file = dir.join("ui-state.toml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &file,
            "sort = \"name\"\ntab = \"git\"\nshow_slugs = false\n\
             preview = \"right\"\nhints_visible = true\n",
        )
        .unwrap();

        let state = load(&file);
        assert_eq!(state.preview, "right", "the panel must stay on the right");
        assert_eq!(state.preview_width_pct, DEFAULT_PREVIEW_WIDTH_PCT);
        assert_eq!(state.preview_height_rows, DEFAULT_PREVIEW_HEIGHT_ROWS);
        assert_eq!(state.columns, "standard");
        assert_eq!(state.sort_dir, SortDir::Asc);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_defaults() {
        assert_eq!(
            load(Path::new("/nonexistent/hop-ui-state.toml")),
            UiState::default()
        );
    }
}
