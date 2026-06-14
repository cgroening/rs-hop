//! Application configuration (settings only; entries live in the repository).
//!
//! [`Config`] holds the user-facing settings: the git tool to launch, the
//! GitHub username to strip from remote names, the icon variant, the example
//! mode and the table column widths. It is loaded from the same `config.toml`
//! that stores the `[[repos]]` entries, but the two concerns are read by
//! different layers ([`loader`] here, the repository in `storage`).

pub mod loader;
pub mod writer;

/// File extensions opened in the editor (everything else uses the default app).
/// Files without an extension count as text too. Overridable via the
/// `editor_extensions` config key.
pub const DEFAULT_EDITOR_EXTENSIONS: &[&str] = &[
    "txt", "text", "md", "markdown", "rst", "org", "rs", "toml", "json",
    "jsonc", "yaml", "yml", "ini", "cfg", "conf", "config", "env", "py", "js",
    "mjs", "cjs", "ts", "tsx", "jsx", "sh", "bash", "zsh", "fish", "c", "h",
    "cpp", "hpp", "cc", "hh", "go", "rb", "java", "kt", "kts", "swift", "php",
    "pl", "lua", "vim", "html", "htm", "css", "scss", "sass", "xml", "csv",
    "tsv", "sql", "tex", "log", "make", "mk",
];

/// Which glyph set the TUI renders, per the user's terminal support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IconVariant {
    /// Plain Unicode symbols (the default): width-consistent across terminals.
    #[default]
    Unicode,
    /// ASCII-only fallback.
    Ascii,
}

impl IconVariant {
    /// Parses the config string value, defaulting to [`IconVariant::Unicode`].
    pub fn from_config_value(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "ascii" => IconVariant::Ascii,
            _ => IconVariant::Unicode,
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
    /// When true, run `git fetch` before the initial status refresh on start.
    pub fetch_on_start: bool,
    /// Preferred editor for opening files (falls back to `$VISUAL`/`$EDITOR`).
    pub editor: Option<String>,
    /// Extensions opened in the editor; other files use the default app.
    pub editor_extensions: Vec<String>,
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
            fetch_on_start: false,
            editor: None,
            editor_extensions: DEFAULT_EDITOR_EXTENSIONS
                .iter()
                .map(|ext| (*ext).to_string())
                .collect(),
            icons: IconVariant::default(),
            column_widths: ColumnWidths::default(),
        }
    }
}
