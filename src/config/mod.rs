//! Application configuration (settings only; entries live in the repository).
//!
//! [`Config`] holds the user-facing settings: the git tool to launch, the
//! GitHub username to strip from remote names, the icon variant, the example
//! mode and the table column widths. It is loaded from the same `config.toml`
//! that stores the `[[repos]]` entries, but the two concerns are read by
//! different layers ([`loader`] here, the repository in `storage`).

pub mod loader;
pub mod migrate;
pub mod writer;

/// Which glyph set the TUI renders, per the user's terminal/font support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IconVariant {
    /// Nerd Font icons (the polished default).
    #[default]
    NerdFont,
    /// Plain Unicode symbols.
    Unicode,
    /// ASCII-only fallback.
    Ascii,
}

impl IconVariant {
    /// Parses the config string value, defaulting to [`IconVariant::NerdFont`].
    pub fn from_config_value(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "unicode" => IconVariant::Unicode,
            "ascii" => IconVariant::Ascii,
            _ => IconVariant::NerdFont,
        }
    }
}

/// A column's width budget: a lower bound and an optional upper bound it may
/// grow to when content needs it and space is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnWidth {
    /// Minimum number of display columns.
    pub min: usize,
    /// Maximum number of display columns, when capped.
    pub max: Option<usize>,
}

impl ColumnWidth {
    /// A fixed lower bound with no cap.
    pub const fn min(min: usize) -> Self {
        ColumnWidth { min, max: None }
    }

    /// A bounded range.
    pub const fn range(min: usize, max: usize) -> Self {
        ColumnWidth {
            min,
            max: Some(max),
        }
    }
}

/// Width budgets for the repository table columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnWidths {
    /// The name column.
    pub name: ColumnWidth,
    /// The current-branch column.
    pub current_branch_name: ColumnWidth,
    /// The status column.
    pub status: ColumnWidth,
    /// The GitHub-repo-name column.
    pub github_repo_name: ColumnWidth,
}

impl Default for ColumnWidths {
    fn default() -> Self {
        ColumnWidths {
            name: ColumnWidth::min(30),
            current_branch_name: ColumnWidth::range(10, 14),
            status: ColumnWidth::min(6),
            github_repo_name: ColumnWidth::min(20),
        }
    }
}

/// Resolved settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Git tool launched on selecting a repo (e.g. `lazygit`); `None` disables.
    pub git_program: Option<String>,
    /// GitHub username stripped from displayed remote names.
    pub github_username: Option<String>,
    /// When true, show each entry's `example_git_info` instead of real status.
    pub example_mode: bool,
    /// Preferred editor for opening files (falls back to `$VISUAL`/`$EDITOR`).
    pub editor: Option<String>,
    /// Which glyph set to render.
    pub icons: IconVariant,
    /// Table column width budgets.
    pub column_widths: ColumnWidths,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            git_program: Some("lazygit".to_string()),
            github_username: None,
            example_mode: false,
            editor: None,
            icons: IconVariant::default(),
            column_widths: ColumnWidths::default(),
        }
    }
}
