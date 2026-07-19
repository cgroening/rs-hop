//! The TUI's own view state (sort, tab, panel, columns, hint footer) across
//! runs.
//!
//! A thin facade over the storage adapter, so the TUI reaches its persisted
//! state through the service layer like every other read and write, and never
//! names a storage module itself.

use std::path::Path;

use crate::domain::error::Result;
use crate::storage::ui_state;

pub use crate::storage::ui_state::{TabView, UiState};

/// Loads the stored view state from `path`, falling back to the defaults when
/// the file is missing or unreadable.
pub fn load(path: &Path) -> UiState {
    ui_state::load(path)
}

/// Writes the view state to `path`.
///
/// # Errors
///
/// Returns an error if the state file cannot be written.
pub fn save(path: &Path, state: &UiState) -> Result<()> {
    ui_state::save(path, state)
}
