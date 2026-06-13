//! Persists small UI state across runs (the chosen sort mode and active tab),
//! as a TOML file in the state directory. Loading is best-effort: a missing or
//! corrupt file falls back to the defaults.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::domain::error::{Error, Result};
use crate::domain::filter::Tab;
use crate::domain::sort::SortMode;

/// The restored UI state.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct UiState {
    /// The list sort mode.
    pub sort: SortMode,
    /// The active tab.
    pub tab: Tab,
    /// Whether slugs are shown inline after the entry name.
    pub show_slugs: bool,
}

/// The on-disk UI state document.
#[derive(Debug, Default, Serialize, Deserialize)]
struct UiStateDoc {
    sort: Option<String>,
    tab: Option<String>,
    show_slugs: Option<bool>,
}

/// Loads the persisted UI state, defaulting when the file is missing/corrupt.
pub fn load(path: &Path) -> UiState {
    let Ok(text) = fs::read_to_string(path) else {
        return UiState::default();
    };
    let Ok(doc) = toml::from_str::<UiStateDoc>(&text) else {
        return UiState::default();
    };
    UiState {
        sort: doc
            .sort
            .as_deref()
            .map_or_else(SortMode::default, SortMode::from_config_value),
        tab: doc.tab.as_deref().map_or_else(Tab::default, Tab::from_key),
        show_slugs: doc.show_slugs.unwrap_or(false),
    }
}

/// Saves the UI state to `path`.
///
/// # Errors
/// Returns an error if the directory or file cannot be written.
pub fn save(path: &Path, state: UiState) -> Result<()> {
    let doc = UiStateDoc {
        sort: Some(state.sort.label().to_string()),
        tab: Some(state.tab.as_key().to_string()),
        show_slugs: Some(state.show_slugs),
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

    #[test]
    fn save_then_load_round_trips() {
        let dir = std::env::temp_dir()
            .join(format!("hop-uistate-test-{}", std::process::id()));
        let file = dir.join("ui-state.toml");
        let state = UiState {
            sort: SortMode::Custom,
            tab: Tab::Archive,
            show_slugs: true,
        };
        save(&file, state).unwrap();
        assert_eq!(load(&file), state);
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
