//! Persists small UI state across runs (currently the chosen sort mode), as a
//! TOML file in the state directory. Loading is best-effort: a missing or
//! corrupt file falls back to the default.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::domain::error::{Error, Result};
use crate::domain::sort::SortMode;

/// The on-disk UI state document.
#[derive(Debug, Default, Serialize, Deserialize)]
struct UiStateDoc {
    sort: Option<String>,
}

/// Loads the persisted sort mode, defaulting when the file is missing/corrupt.
pub fn load_sort(path: &Path) -> SortMode {
    let Ok(text) = fs::read_to_string(path) else {
        return SortMode::default();
    };
    let Ok(doc) = toml::from_str::<UiStateDoc>(&text) else {
        return SortMode::default();
    };
    doc.sort
        .as_deref()
        .map_or_else(SortMode::default, SortMode::from_config_value)
}

/// Saves the sort mode to `path`.
///
/// # Errors
/// Returns an error if the directory or file cannot be written.
pub fn save_sort(path: &Path, sort: SortMode) -> Result<()> {
    let doc = UiStateDoc {
        sort: Some(sort.label().to_string()),
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
        save_sort(&file, SortMode::Custom).unwrap();
        assert_eq!(load_sort(&file), SortMode::Custom);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_defaults() {
        assert_eq!(
            load_sort(Path::new("/nonexistent/hop-ui-state.toml")),
            SortMode::default()
        );
    }
}
