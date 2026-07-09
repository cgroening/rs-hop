//! Loads [`Config`] settings from the TOML file (entries are read separately).
//!
//! Precedence: built-in defaults < `config.toml` < `HOP_`-prefixed env vars.
//! The `[[repos]]` array in the same file is ignored here; the repository owns
//! it.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::{env, fs};

use serde::Deserialize;

use crate::config::{Appearance, ColumnWidth, ColumnWidths, Config};
use crate::domain::error::{Error, Result};
use crate::theme::{GlyphVariant, ThemeColors, parse_color};

/// Environment override for the git tool.
const GIT_PROGRAM_ENV: &str = "HOP_GIT_PROGRAM";

/// Environment override for the editor.
const EDITOR_ENV: &str = "HOP_EDITOR";

/// Environment override for the active theme.
const THEME_ENV: &str = "HOP_THEME";

/// Environment override for the glyph variant (`unicode`/`ascii`).
const GLYPHS_ENV: &str = "HOP_GLYPHS";

/// Raw settings as read from TOML; the `repos` array is intentionally ignored.
#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    git_program: Option<String>,
    github_username: Option<String>,
    example_mode: Option<bool>,
    fetch_on_start: Option<bool>,
    editor: Option<String>,
    editor_extensions: Option<Vec<String>>,
    appearance: Option<RawAppearance>,
    /// Legacy `[icons]` table (pre-migration), folded into `appearance.glyphs`.
    icons: Option<RawIcons>,
    themes: Option<BTreeMap<String, HashMap<String, String>>>,
    keys: Option<BTreeMap<String, KeyBinding>>,
    column_widths: Option<HashMap<String, RawColumnWidth>>,
    zip_backup_folder: Option<String>,
    zip_exclude_dirs: Option<Vec<String>>,
}

/// The `[appearance]` table.
#[derive(Debug, Default, Deserialize)]
struct RawAppearance {
    theme: Option<String>,
    colors: Option<BTreeMap<String, String>>,
    glyphs: Option<String>,
}

/// The legacy `[icons]` table.
#[derive(Debug, Default, Deserialize)]
struct RawIcons {
    variant: Option<String>,
}

/// A `[keys]` binding: one key string or a list of them.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum KeyBinding {
    One(String),
    Many(Vec<String>),
}

impl KeyBinding {
    /// The bound keys as a list.
    fn into_keys(self) -> Vec<String> {
        match self {
            KeyBinding::One(key) => vec![key],
            KeyBinding::Many(keys) => keys,
        }
    }
}

/// A column width as either a bare integer (minimum) or a `{ min, max }` table.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawColumnWidth {
    Fixed(usize),
    Range {
        min: Option<usize>,
        max: Option<usize>,
    },
}

impl RawColumnWidth {
    /// Converts to a [`ColumnWidth`], falling back to `default` for any missing
    /// bound.
    fn resolve(&self, default: ColumnWidth) -> ColumnWidth {
        match self {
            RawColumnWidth::Fixed(min) => ColumnWidth {
                min: *min,
                max: None,
            },
            RawColumnWidth::Range { min, max } => ColumnWidth {
                min: min.unwrap_or(default.min),
                max: max.or(default.max),
            },
        }
    }
}

/// Loads the configuration settings from `path`, returning defaults when the
/// file does not exist.
///
/// # Errors
/// Returns [`Error::Config`] if the file exists but cannot be read or parsed.
pub fn load_config(path: &Path) -> Result<Config> {
    let raw = read_raw(path)?;
    let mut config = build(raw);
    apply_env(&mut config);
    Ok(config)
}

/// Reads and parses the raw settings, or returns defaults when `path` is absent.
fn read_raw(path: &Path) -> Result<RawConfig> {
    if !path.exists() {
        return Ok(RawConfig::default());
    }
    let text = fs::read_to_string(path).map_err(|e| {
        Error::config(path.display().to_string(), e.to_string())
    })?;
    toml::from_str(&text)
        .map_err(|e| Error::config(path.display().to_string(), e.to_string()))
}

/// Merges a [`RawConfig`] onto the defaults.
fn build(raw: RawConfig) -> Config {
    let defaults = Config::default();
    Config {
        git_program: raw.git_program.or(defaults.git_program),
        github_username: raw.github_username.or(defaults.github_username),
        example_mode: raw.example_mode.unwrap_or(defaults.example_mode),
        fetch_on_start: raw.fetch_on_start.unwrap_or(defaults.fetch_on_start),
        editor: raw.editor.or(defaults.editor),
        editor_extensions: raw
            .editor_extensions
            .filter(|exts| !exts.is_empty())
            .unwrap_or(defaults.editor_extensions),
        appearance: resolve_appearance(
            raw.appearance,
            raw.icons,
            defaults.appearance,
        ),
        themes: resolve_themes(raw.themes),
        keys: raw
            .keys
            .map(|keys| {
                keys.into_iter()
                    .map(|(action, binding)| (action, binding.into_keys()))
                    .collect()
            })
            .unwrap_or(defaults.keys),
        column_widths: resolve_column_widths(raw.column_widths.as_ref()),
        zip_backup_folder: raw.zip_backup_folder.or(defaults.zip_backup_folder),
        zip_exclude_dirs: raw
            .zip_exclude_dirs
            .filter(|dirs| !dirs.is_empty())
            .unwrap_or(defaults.zip_exclude_dirs),
    }
}

/// Resolves the `[appearance]` settings, folding the legacy `[icons].variant`
/// into `glyphs` for back-compat (the new `[appearance].glyphs` wins when both
/// are present).
fn resolve_appearance(
    raw: Option<RawAppearance>,
    legacy_icons: Option<RawIcons>,
    defaults: Appearance,
) -> Appearance {
    let raw = raw.unwrap_or_default();
    let legacy_glyph = legacy_icons.and_then(|icons| icons.variant);
    let glyphs = raw
        .glyphs
        .or(legacy_glyph)
        .map_or(defaults.glyphs, |value| parse_glyph_variant(&value));
    Appearance {
        theme: raw.theme.unwrap_or(defaults.theme),
        colors: raw.colors.unwrap_or(defaults.colors),
        glyphs,
    }
}

/// Parses a glyph-variant config string, defaulting to Unicode.
fn parse_glyph_variant(value: &str) -> GlyphVariant {
    match value.trim().to_lowercase().as_str() {
        "ascii" => GlyphVariant::Ascii,
        _ => GlyphVariant::Unicode,
    }
}

/// Resolves user `[themes.<name>]` tables into `(name, ThemeColors)` pairs; each
/// theme may set only some colors (the rest are derived from
/// accent/foreground/background).
fn resolve_themes(
    raw: Option<BTreeMap<String, HashMap<String, String>>>,
) -> Vec<(String, ThemeColors)> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    raw.into_iter()
        .map(|(name, colors)| {
            let theme = ThemeColors::from_lookup(|key| {
                colors.get(key).and_then(|value| parse_color(value))
            });
            (name, theme)
        })
        .collect()
}

/// Resolves the column width budgets, applying any configured override onto the
/// per-column defaults.
fn resolve_column_widths(
    raw: Option<&HashMap<String, RawColumnWidth>>,
) -> ColumnWidths {
    let defaults = ColumnWidths::default();
    let Some(raw) = raw else {
        return defaults;
    };
    let resolve = |key: &str, default: ColumnWidth| {
        raw.get(key).map_or(default, |value| value.resolve(default))
    };
    ColumnWidths {
        name: resolve("name", defaults.name),
        current_branch_name: resolve(
            "current_branch_name",
            defaults.current_branch_name,
        ),
        status: resolve("status", defaults.status),
        github_repo_name: resolve(
            "github_repo_name",
            defaults.github_repo_name,
        ),
        zip_backup: resolve("zip_backup", defaults.zip_backup),
    }
}

/// Applies environment overrides (git tool, editor, theme, glyphs).
fn apply_env(config: &mut Config) {
    if let Ok(value) = env::var(GIT_PROGRAM_ENV)
        && !value.trim().is_empty()
    {
        config.git_program = Some(value);
    }
    if let Ok(value) = env::var(EDITOR_ENV)
        && !value.trim().is_empty()
    {
        config.editor = Some(value);
    }
    if let Ok(value) = env::var(THEME_ENV)
        && !value.trim().is_empty()
    {
        config.appearance.theme = value;
    }
    if let Ok(value) = env::var(GLYPHS_ENV)
        && !value.trim().is_empty()
    {
        config.appearance.glyphs = parse_glyph_variant(&value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_empty() {
        let config = build(RawConfig::default());
        assert_eq!(config.git_program.as_deref(), Some("lazygit"));
        assert!(!config.example_mode);
        assert!(!config.fetch_on_start);
        assert_eq!(config.appearance.glyphs, GlyphVariant::Unicode);
        assert_eq!(config.column_widths, ColumnWidths::default());
        assert!(config.editor_extensions.iter().any(|e| e == "rs"));
    }

    #[test]
    fn editor_extensions_override_replaces_default() {
        let text = "editor_extensions = [\"md\", \"txt\"]\n";
        let raw: RawConfig = toml::from_str(text).unwrap();
        let config = build(raw);
        assert_eq!(config.editor_extensions, ["md", "txt"]);
    }

    #[test]
    fn parses_settings_and_ignores_repos() {
        let text = r#"
git_program = "gitui"
github_username = "cgroening"
example_mode = true
fetch_on_start = true

[icons]
variant = "ascii"

[column_widths]
name = 40
[column_widths.current_branch_name]
min = 8
max = 20

[[repos]]
name = "ignored here"
path = "/tmp/x"
"#;
        let raw: RawConfig = toml::from_str(text).unwrap();
        let config = build(raw);
        assert_eq!(config.git_program.as_deref(), Some("gitui"));
        assert_eq!(config.github_username.as_deref(), Some("cgroening"));
        assert!(config.example_mode);
        assert!(config.fetch_on_start);
        assert_eq!(config.appearance.glyphs, GlyphVariant::Ascii);
        assert_eq!(config.column_widths.name, ColumnWidth::min(40));
        assert_eq!(
            config.column_widths.current_branch_name,
            ColumnWidth::range(8, 20)
        );
        // Unspecified columns keep their defaults.
        assert_eq!(config.column_widths.status, ColumnWidth::min(6));
    }

    #[test]
    fn parses_appearance_themes_and_keys() {
        let text = r##"
[appearance]
theme = "midnight"
glyphs = "ascii"
colors = { accent = "#ff0000" }

[themes.midnight]
accent = "#8899ff"
background = "#000010"

[keys]
add = "N"
delete = ["d", "backspace"]
"##;
        let raw: RawConfig = toml::from_str(text).unwrap();
        let config = build(raw);
        assert_eq!(config.appearance.theme, "midnight");
        assert_eq!(config.appearance.glyphs, GlyphVariant::Ascii);
        assert_eq!(
            config.appearance.colors.get("accent").map(String::as_str),
            Some("#ff0000"),
        );
        assert!(config.themes.iter().any(|(name, _)| name == "midnight"));
        assert_eq!(config.keys.get("add"), Some(&vec!["N".to_string()]));
        assert_eq!(
            config.keys.get("delete"),
            Some(&vec!["d".to_string(), "backspace".to_string()]),
        );
        // The custom theme resolves through the registry.
        assert!(config.theme_registry().contains("midnight"));
    }

    #[test]
    fn new_appearance_glyphs_wins_over_legacy_icons() {
        // An old config used `[icons]`; a config carrying both prefers the new
        // `[appearance].glyphs`, and one with only `[icons]` still loads.
        let both = "[appearance]\nglyphs = \"unicode\"\n\n[icons]\nvariant = \"ascii\"\n";
        let config = build(toml::from_str(both).unwrap());
        assert_eq!(config.appearance.glyphs, GlyphVariant::Unicode);

        let legacy = "[icons]\nvariant = \"ascii\"\n";
        let config = build(toml::from_str(legacy).unwrap());
        assert_eq!(config.appearance.glyphs, GlyphVariant::Ascii);
    }
}
