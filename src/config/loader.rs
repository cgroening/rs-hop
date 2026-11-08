//! Loads [`Config`] settings from the TOML file (entries are read separately).
//!
//! Precedence: built-in defaults < `config.toml` < `HOP_`-prefixed env vars.
//! The `[[repos]]` array in the same file is ignored here; the repository owns
//! it.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::{env, fs};

use serde::Deserialize;

use crate::config::appearance::MAX_DIM_AMOUNT;
use crate::config::{Appearance, ColumnWidth, ColumnWidths, Config};
use crate::domain::error::{Error, Result};
use crate::theme::{GlyphVariant, Palette, ThemeColors, parse_color};

/// Environment override for the git tool.
const GIT_PROGRAM_ENV: &str = "HOP_GIT_PROGRAM";

/// Environment override for the editor.
const EDITOR_ENV: &str = "HOP_EDITOR";

/// Environment override for the active theme.
const THEME_ENV: &str = "HOP_THEME";

/// Environment override for the glyph variant (`unicode`/`ascii`).
const GLYPHS_ENV: &str = "HOP_GLYPHS";
/// Overrides `[appearance] dim_amount`.
const DIM_AMOUNT_ENV: &str = "HOP_DIM_AMOUNT";

/// Environment override for the quit confirmation.
const CONFIRM_QUIT_ENV: &str = "HOP_CONFIRM_QUIT";

/// Environment override for the pull/push stall timeout, in seconds.
const GIT_TIMEOUT_ENV: &str = "HOP_GIT_TIMEOUT_SECS";

/// Raw settings as read from TOML; the `repos` array is intentionally ignored.
#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    git_program: Option<String>,
    github_username: Option<String>,
    example_mode: Option<bool>,
    fetch_on_start: Option<bool>,
    git_timeout_secs: Option<u64>,
    confirm_quit: Option<bool>,
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
    /// Percent dimmed colored text darkens while the color `dim` is set.
    dim_amount: Option<u8>,
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
///
/// Returns [`Error::Config`] if the file exists but cannot be read or parsed.
pub fn load_config(path: &Path) -> Result<Config> {
    let raw = read_raw(path)?;
    let mut config = build(raw);
    apply_env(&mut config);
    Ok(config)
}

/// Every problem the config file at `path` has: one it cannot be read or
/// parsed for, and each value loading would only warn about and ignore (an
/// unknown colour, a `dim_amount` above 100). Empty for a clean file and for
/// a missing one. `F5` reloads only a clean configuration.
pub fn problems(path: &Path) -> Vec<String> {
    let raw = match read_raw(path) {
        Ok(raw) => raw,
        Err(error) => return vec![error.to_string()],
    };
    let mut problems: Vec<String> = unknown_color_names(&raw)
        .into_iter()
        .map(|(section, name)| format!("unknown color '{name}' in [{section}]"))
        .collect();
    if let Some(amount) = raw.appearance.as_ref().and_then(|a| a.dim_amount)
        && amount > MAX_DIM_AMOUNT
    {
        problems.push(format!(
            "appearance.dim_amount = {amount} is above {MAX_DIM_AMOUNT}"
        ));
    }
    problems
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
    report_unknown(&unknown_color_names(&raw));
    Config {
        git_program: raw.git_program.or(defaults.git_program),
        github_username: raw.github_username.or(defaults.github_username),
        example_mode: raw.example_mode.unwrap_or(defaults.example_mode),
        fetch_on_start: raw.fetch_on_start.unwrap_or(defaults.fetch_on_start),
        git_timeout_secs: raw
            .git_timeout_secs
            .filter(|secs| *secs > 0)
            .unwrap_or(defaults.git_timeout_secs),
        confirm_quit: raw.confirm_quit.unwrap_or(defaults.confirm_quit),
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
        dim_amount: dim_amount(raw.dim_amount, defaults.dim_amount),
    }
}

/// The configured `dim_amount`, or `default` when it is absent or above 100
/// (reported).
fn dim_amount(raw: Option<u8>, default: u8) -> u8 {
    match raw {
        Some(amount) if amount <= MAX_DIM_AMOUNT => amount,
        Some(amount) => {
            log::warn!(
                "appearance.dim_amount = {amount} is above {MAX_DIM_AMOUNT}, \
                 using {default}"
            );
            default
        }
        None => default,
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

/// Every color the file names in a section that cannot carry it, as
/// `(section, name)` pairs.
///
/// The two sections take different color sets, and mixing them up is silent:
/// `[appearance].colors` overrides any palette color, while a
/// `[themes.<name>]` contributes only the [`ThemeColors`] a palette is derived
/// *from*. Validating a theme against `Palette::KEYS` accepts the derived ones
/// (`selection`, `cursor`, `input_bg`, ...) and then drops the value without a
/// word.
fn unknown_color_names(raw: &RawConfig) -> Vec<(String, String)> {
    let mut unknown = Vec::new();
    if let Some(colors) =
        raw.appearance.as_ref().and_then(|it| it.colors.as_ref())
    {
        for name in unknown_colors(colors.keys(), Palette::KEYS) {
            unknown.push(("appearance.colors".to_string(), name));
        }
    }
    for (theme, colors) in raw.themes.iter().flatten() {
        for name in unknown_colors(colors.keys(), ThemeColors::KEYS) {
            unknown.push((format!("themes.{theme}"), name));
        }
    }
    unknown
}

/// The names in `names` that are not in `known`.
fn unknown_colors<'a>(
    names: impl Iterator<Item = &'a String>,
    known: &[&str],
) -> Vec<String> {
    names
        .filter(|name| !known.contains(&name.as_str()))
        .cloned()
        .collect()
}

/// Warns about each unusable color name, so a typo (or a color in the wrong
/// section) surfaces instead of being silently ignored.
fn report_unknown(unknown: &[(String, String)]) {
    for (section, name) in unknown {
        log::warn!("unknown color '{name}' in [{section}], ignoring");
    }
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

/// Applies environment overrides (git tool, editor, theme, glyphs, quit
/// confirmation).
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
    if let Ok(value) = env::var(DIM_AMOUNT_ENV)
        && let Ok(amount) = value.trim().parse::<u8>()
        && amount <= MAX_DIM_AMOUNT
    {
        config.appearance.dim_amount = amount;
    }
    if let Ok(value) = env::var(CONFIRM_QUIT_ENV)
        && let Some(flag) = parse_bool(&value)
    {
        config.confirm_quit = flag;
    }
    if let Ok(value) = env::var(GIT_TIMEOUT_ENV)
        && let Some(secs) = parse_timeout(&value)
    {
        config.git_timeout_secs = secs;
    }
}

/// Parses a positive number of seconds. Zero or anything unreadable leaves the
/// setting alone: a zero timeout would make every pull and push fail at once.
fn parse_timeout(value: &str) -> Option<u64> {
    value.trim().parse::<u64>().ok().filter(|secs| *secs > 0)
}

/// Parses a boolean environment value. Anything else leaves the setting alone,
/// so a typo cannot silently turn the quit confirmation off.
fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
