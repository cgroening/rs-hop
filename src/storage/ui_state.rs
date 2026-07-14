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
use crate::util::fs::write_atomic;

/// Default width of the panel when it sits to the right of the list, in percent.
pub const DEFAULT_PREVIEW_WIDTH_PCT: u16 = 40;
/// Default height of the panel when it sits below the list, in rows.
pub const DEFAULT_PREVIEW_HEIGHT_ROWS: u16 = 9;

/// The view settings a single kind (git or files) remembers, shared between its
/// active and archive views. `columns` is a raw key: its enum lives in the TUI
/// layer, which storage must not depend on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabView {
    /// The list sort mode.
    pub sort: SortMode,
    /// Which way the sort runs.
    pub sort_dir: SortDir,
    /// The active column-set key (`"standard"`, `"code"` or `"activity"`).
    pub columns: String,
    /// Whether entries are grouped into sections (off = flat global sort).
    pub grouped: bool,
    /// Whether favourites float to the top (off = pure sort).
    pub fav_float: bool,
}

impl Default for TabView {
    /// Grouping and favourite floating start on; sort and columns at their base.
    fn default() -> Self {
        TabView {
            sort: SortMode::default(),
            sort_dir: SortDir::default(),
            columns: "standard".to_string(),
            grouped: true,
            fav_float: true,
        }
    }
}

/// The restored UI state. Per-kind view settings live in `git`/`files`; the
/// rest are global. `preview` is a raw key (its enum lives in the TUI layer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiState {
    /// The git tab's view settings (active + archive share these).
    pub git: TabView,
    /// The files tab's view settings (active + archive share these).
    pub files: TabView,
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
    /// Whether the shortcut-hint footer is shown (toggled with `F1`).
    pub hints_visible: bool,
}

impl Default for UiState {
    /// Both kinds at their defaults, panel off, hint footer shown.
    fn default() -> Self {
        UiState {
            git: TabView::default(),
            files: TabView::default(),
            tab: Tab::default(),
            show_slugs: false,
            preview: "off".to_string(),
            preview_width_pct: DEFAULT_PREVIEW_WIDTH_PCT,
            preview_height_rows: DEFAULT_PREVIEW_HEIGHT_ROWS,
            hints_visible: true,
        }
    }
}

/// The per-kind view-settings block on disk. Every field optional so a partial
/// file still loads.
#[derive(Debug, Default, Serialize, Deserialize)]
struct TabViewDoc {
    sort: Option<String>,
    sort_dir: Option<String>,
    columns: Option<String>,
    grouped: Option<bool>,
    fav_float: Option<bool>,
}

/// The two per-kind blocks, serialised as `[tabs.git]` / `[tabs.files]`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct TabsDoc {
    git: Option<TabViewDoc>,
    files: Option<TabViewDoc>,
}

/// The on-disk UI state document. Every field is optional, so a file written by
/// an older version still loads and only the new keys fall back to defaults.
/// The top-level `sort`/`sort_dir`/`columns` are legacy globals: a pre-per-kind
/// file carries them, and they seed both kinds when the `[tabs.*]` blocks are
/// absent.
#[derive(Debug, Default, Serialize, Deserialize)]
struct UiStateDoc {
    tab: Option<String>,
    show_slugs: Option<bool>,
    preview: Option<String>,
    preview_width_pct: Option<u16>,
    preview_height_rows: Option<u16>,
    hints_visible: Option<bool>,
    tabs: Option<TabsDoc>,
    sort: Option<String>,
    sort_dir: Option<String>,
    columns: Option<String>,
}

/// Builds a [`TabView`] from its on-disk block, falling back to the legacy
/// top-level `sort`/`sort_dir`/`columns` so a pre-per-kind file keeps them.
fn tab_view(block: Option<TabViewDoc>, legacy: &UiStateDoc) -> TabView {
    let block = block.unwrap_or_default();
    let sort = block.sort.or_else(|| legacy.sort.clone());
    let sort_dir = block.sort_dir.or_else(|| legacy.sort_dir.clone());
    let columns = block.columns.or_else(|| legacy.columns.clone());
    TabView {
        sort: sort
            .as_deref()
            .map_or_else(SortMode::default, SortMode::from_config_value),
        sort_dir: sort_dir
            .as_deref()
            .map_or_else(SortDir::default, SortDir::from_config_value),
        columns: columns.unwrap_or_else(|| "standard".to_string()),
        grouped: block.grouped.unwrap_or(true),
        fav_float: block.fav_float.unwrap_or(true),
    }
}

/// Loads the persisted UI state, defaulting when the file is missing/corrupt.
pub fn load(path: &Path) -> UiState {
    let Ok(text) = fs::read_to_string(path) else {
        return UiState::default();
    };
    let Ok(mut doc) = toml::from_str::<UiStateDoc>(&text) else {
        return UiState::default();
    };
    let defaults = UiState::default();
    let tabs = doc.tabs.take().unwrap_or_default();
    UiState {
        git: tab_view(tabs.git, &doc),
        files: tab_view(tabs.files, &doc),
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
        hints_visible: doc.hints_visible.unwrap_or(true),
    }
}

/// The on-disk block for `view`.
fn view_doc(view: &TabView) -> TabViewDoc {
    TabViewDoc {
        sort: Some(view.sort.label().to_string()),
        sort_dir: Some(view.sort_dir.label().to_string()),
        columns: Some(view.columns.clone()),
        grouped: Some(view.grouped),
        fav_float: Some(view.fav_float),
    }
}

/// Saves the UI state to `path`.
///
/// # Errors
/// Returns an error if the directory or file cannot be written.
pub fn save(path: &Path, state: &UiState) -> Result<()> {
    let doc = UiStateDoc {
        tab: Some(state.tab.as_key().to_string()),
        show_slugs: Some(state.show_slugs),
        preview: Some(state.preview.clone()),
        preview_width_pct: Some(state.preview_width_pct),
        preview_height_rows: Some(state.preview_height_rows),
        hints_visible: Some(state.hints_visible),
        tabs: Some(TabsDoc {
            git: Some(view_doc(&state.git)),
            files: Some(view_doc(&state.files)),
        }),
        sort: None,
        sort_dir: None,
        columns: None,
    };
    let text = toml::to_string_pretty(&doc)
        .map_err(|e| Error::invalid(format!("serialise ui state: {e}")))?;
    write_atomic(path, &text, "ui state")
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
            git: TabView {
                sort: SortMode::Loc,
                sort_dir: SortDir::Desc,
                columns: "code".to_string(),
                grouped: false,
                fav_float: false,
            },
            files: TabView {
                sort: SortMode::Recent,
                sort_dir: SortDir::Asc,
                columns: "standard".to_string(),
                grouped: true,
                fav_float: false,
            },
            tab: Tab::FilesArchive,
            show_slugs: true,
            preview: "right".to_string(),
            preview_width_pct: 55,
            preview_height_rows: 14,
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
        assert_eq!(state.git.sort_dir, SortDir::Asc);
        assert_eq!(state.git.columns, "standard");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_global_sort_seeds_both_kinds() {
        // A pre-per-kind file's top-level sort/columns apply to git and files.
        let dir = temp("legacy-sort");
        let file = dir.join("ui-state.toml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&file, "sort = \"frecency\"\ncolumns = \"code\"\n").unwrap();

        let state = load(&file);
        assert_eq!(state.git.sort, SortMode::Frecency);
        assert_eq!(state.files.sort, SortMode::Frecency);
        assert_eq!(state.git.columns, "code");
        assert_eq!(state.files.columns, "code");
        // The new per-kind toggles default on.
        assert!(state.git.grouped && state.git.fav_float);
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
