//! Application configuration (settings only; entries live in the repository).
//!
//! [`Config`] holds the user-facing settings: the git tool to launch, the
//! GitHub username to strip from remote names, the icon variant, the example
//! mode and the table column widths. It is loaded from the same `config.toml`
//! that stores the `[[repos]]` entries, but the two concerns are read by
//! different layers ([`loader`] here, the repository in `storage`).

pub mod appearance;
pub mod loader;

use std::collections::BTreeMap;

pub use appearance::Appearance;

use crate::keymap::Keymap;
use crate::theme::{
    Color, ColorOverrides, DEFAULT_THEME, Palette, Skin, ThemeColors,
    ThemeRegistry,
};

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

/// Directory names excluded from a repo ZIP backup (`z`/`Z`): build artefacts
/// and caches that are regenerated and would only bloat the archive. Matched as
/// a name prefix at any depth (so `target` also covers `target.nosync`).
/// Overridable via the `zip_exclude_dirs` config key.
pub const DEFAULT_ZIP_EXCLUDE_DIRS: &[&str] = &[
    "target",
    "node_modules",
    "dist",
    "build",
    "out",
    "bin",
    "obj",
    ".next",
    ".nuxt",
    ".turbo",
    ".parcel-cache",
    "__pycache__",
    ".venv",
    "venv",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".tox",
    ".gradle",
    ".build",
    "DerivedData",
    "Pods",
    ".cache",
];

/// hop's built-in default theme: the rose accent kept from the pre-migration
/// look, on a dark scheme whose header/footer bands sit below a lighter content
/// surface (the panel layout). Registered as the `default` theme, replacing the
/// toolkit's teal built-in.
const HOP_DEFAULT_THEME: ThemeColors = ThemeColors {
    accent: Color::hex("#f7a3bd"),
    foreground: Color::hex("#e5e5e5"),
    background: Color::hex("#151515"),
    header: Color::hex("#0e0c12"),
    footer: Color::hex("#0e0c12"),
    panel: Color::hex("#1c1a20"),
    surface: Color::hex("#26222b"),
    border: Color::hex("#4a4652"),
    // The lifted border a focused box draws, keeping its contrast against the
    // brighter fill; `border.lighten(0.15)`, the toolkit's own derivation.
    border_focus: Color::hex("#74707d"),
    success: Color::hex("#8fbf7f"),
    warning: Color::hex("#dcbe5a"),
    error: Color::hex("#d05050"),
    info: Color::hex("#7fb3d4"),
};

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
    /// The ZIP-backup-date column.
    pub zip_backup: ColumnWidth,
}

impl Default for ColumnWidths {
    fn default() -> Self {
        ColumnWidths {
            name: ColumnWidth::min(30),
            current_branch_name: ColumnWidth::range(10, 14),
            status: ColumnWidth::min(6),
            github_repo_name: ColumnWidth::min(20),
            zip_backup: ColumnWidth::min(10),
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
    /// When true, quitting with `q` asks for confirmation first. `Ctrl+Q`
    /// always quits at once.
    pub confirm_quit: bool,
    /// Preferred editor for opening files (falls back to `$VISUAL`/`$EDITOR`).
    pub editor: Option<String>,
    /// Extensions opened in the editor; other files use the default app.
    pub editor_extensions: Vec<String>,
    /// Theme, per-color overrides and glyph variant (`[appearance]`).
    pub appearance: Appearance,
    /// User-defined themes layered over the built-ins (`[themes.<name>]`).
    pub themes: Vec<(String, ThemeColors)>,
    /// Per-action key overrides (`[keys]`), an action name to its key strings.
    pub keys: BTreeMap<String, Vec<String>>,
    /// Table column width budgets.
    pub column_widths: ColumnWidths,
    /// Destination folder for repo ZIP backups (`z`/`Z`); `None` disables them.
    /// A leading `~` is expanded when the folder is used.
    pub zip_backup_folder: Option<String>,
    /// Directory names excluded from a ZIP backup (see
    /// [`DEFAULT_ZIP_EXCLUDE_DIRS`]).
    pub zip_exclude_dirs: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            git_program: Some("lazygit".to_string()),
            github_username: None,
            example_mode: false,
            fetch_on_start: false,
            confirm_quit: false,
            editor: None,
            editor_extensions: DEFAULT_EDITOR_EXTENSIONS
                .iter()
                .map(|ext| (*ext).to_string())
                .collect(),
            appearance: Appearance::default(),
            themes: Vec::new(),
            keys: BTreeMap::new(),
            column_widths: ColumnWidths::default(),
            zip_backup_folder: None,
            zip_exclude_dirs: DEFAULT_ZIP_EXCLUDE_DIRS
                .iter()
                .map(|dir| (*dir).to_string())
                .collect(),
        }
    }
}

impl Config {
    /// The theme registry: the built-ins with hop's rose theme installed as
    /// `default`, then any user `[themes.<name>]` layered on top.
    pub fn theme_registry(&self) -> ThemeRegistry {
        ThemeRegistry::builtin()
            .with_custom([(DEFAULT_THEME.to_string(), HOP_DEFAULT_THEME)])
            .with_custom(self.themes.iter().cloned())
    }

    /// The per-color overrides from `[appearance].colors`, resolved by name.
    pub fn color_overrides(&self) -> ColorOverrides<'_> {
        ColorOverrides::from_lookup(|name| {
            self.appearance.colors.get(name).map_or("", String::as_str)
        })
    }

    /// The resolved color palette: the active theme with the color overrides
    /// layered on.
    pub fn palette(&self) -> Palette {
        let base = self.theme_registry().resolve(&self.appearance.theme);
        Palette::resolve(base, &self.color_overrides())
    }

    /// The resolved skin (palette plus glyphs) the TUI renders with.
    pub fn skin(&self) -> Skin {
        Skin::new(self.palette(), self.glyphs())
    }

    /// The resolved marker glyphs.
    pub fn glyphs(&self) -> crate::theme::Glyphs {
        crate::theme::Glyphs::new(self.appearance.glyphs)
    }

    /// The keymap: hop's default bindings with any `[keys]` overrides applied.
    pub fn keymap(&self) -> Keymap {
        Keymap::from_overrides(&self.keys)
    }
}
