use super::*;
use crate::config::DEFAULT_GIT_TIMEOUT_SECS;

/// The `(section, name)` pairs `content` reports as unusable.
fn unknown(content: &str) -> Vec<(String, String)> {
    let raw: RawConfig = toml::from_str(content).expect("valid toml");
    unknown_color_names(&raw)
}

#[test]
fn a_theme_table_rejects_the_palettes_derived_colors() {
    // `cursor`, `selection` and the input fills are derived by the toolkit;
    // a theme cannot contribute them. Accepting them here would drop the
    // value without a word.
    let mut reported = unknown(
        "[themes.mine]\n\
             accent = \"#010203\"\n\
             border = \"#010203\"\n\
             cursor = \"#010203\"\n\
             selection = \"#010203\"\n",
    );
    reported.sort();
    assert_eq!(
        reported,
        vec![
            ("themes.mine".to_string(), "cursor".to_string()),
            ("themes.mine".to_string(), "selection".to_string()),
        ],
    );
}

#[test]
fn appearance_colors_may_name_any_palette_color() {
    let reported = unknown(
        "[appearance]\n\
             colors = { cursor = \"#010203\", selection = \"#010203\" }\n",
    );
    assert!(reported.is_empty(), "{reported:?}");
}

#[test]
fn a_theme_may_name_every_theme_color() {
    let table =
        ThemeColors::KEYS
            .iter()
            .fold(String::new(), |mut table, name| {
                table.push_str(name);
                table.push_str(" = \"#010203\"\n");
                table
            });
    assert!(unknown(&format!("[themes.mine]\n{table}")).is_empty());
}

#[test]
fn both_sections_report_a_typo() {
    assert_eq!(
        unknown("[appearance]\ncolors = { bordr = \"#010203\" }\n"),
        vec![("appearance.colors".to_string(), "bordr".to_string())],
    );
    assert_eq!(
        unknown("[themes.mine]\nbordr = \"#010203\"\n"),
        vec![("themes.mine".to_string(), "bordr".to_string())],
    );
}

#[test]
fn a_clean_file_reports_nothing() {
    assert!(unknown("[appearance]\ntheme = \"default\"\n").is_empty());
}

#[test]
fn every_theme_color_name_is_also_a_palette_color_name() {
    for name in ThemeColors::KEYS {
        assert!(Palette::KEYS.contains(name), "{name} is unreachable");
    }
}

#[test]
fn defaults_when_empty() {
    let config = build(RawConfig::default());
    assert_eq!(config.git_program.as_deref(), Some("lazygit"));
    assert!(!config.example_mode);
    assert!(!config.fetch_on_start);
    assert_eq!(config.git_timeout_secs, DEFAULT_GIT_TIMEOUT_SECS);
    assert_eq!(config.appearance.glyphs, GlyphVariant::Unicode);
    assert_eq!(config.column_widths, ColumnWidths::default());
    assert!(config.editor_extensions.iter().any(|e| e == "rs"));
}

#[test]
fn git_timeout_is_read_and_a_zero_keeps_the_default() {
    let raw: RawConfig = toml::from_str("git_timeout_secs = 40\n").unwrap();
    assert_eq!(build(raw).git_timeout_secs, 40);
    let raw: RawConfig = toml::from_str("git_timeout_secs = 0\n").unwrap();
    assert_eq!(build(raw).git_timeout_secs, DEFAULT_GIT_TIMEOUT_SECS);
}

#[test]
fn dim_amount_is_read_and_one_above_100_keeps_the_default() {
    let raw: RawConfig =
        toml::from_str("[appearance]\ndim_amount = 55\n").unwrap();
    assert_eq!(build(raw).appearance.dim_amount, 55);
    let raw: RawConfig =
        toml::from_str("[appearance]\ndim_amount = 101\n").unwrap();
    assert_eq!(
        build(raw).appearance.dim_amount,
        crate::config::appearance::DEFAULT_DIM_AMOUNT
    );
}

#[test]
fn a_timeout_from_the_environment_must_be_a_positive_number() {
    assert_eq!(parse_timeout(" 30 "), Some(30));
    assert_eq!(parse_timeout("0"), None);
    assert_eq!(parse_timeout("soon"), None);
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
    let both =
        "[appearance]\nglyphs = \"unicode\"\n\n[icons]\nvariant = \"ascii\"\n";
    let config = build(toml::from_str(both).unwrap());
    assert_eq!(config.appearance.glyphs, GlyphVariant::Unicode);

    let legacy = "[icons]\nvariant = \"ascii\"\n";
    let config = build(toml::from_str(legacy).unwrap());
    assert_eq!(config.appearance.glyphs, GlyphVariant::Ascii);
}

#[test]
fn a_bool_env_value_reads_the_usual_spellings() {
    for on in ["1", "true", "TRUE", " yes ", "on"] {
        assert_eq!(parse_bool(on), Some(true), "{on:?}");
    }
    for off in ["0", "false", "no", "OFF"] {
        assert_eq!(parse_bool(off), Some(false), "{off:?}");
    }
}

#[test]
fn a_misspelt_bool_env_value_leaves_the_setting_alone() {
    // Returning `false` here would silently disable the quit confirmation.
    assert_eq!(parse_bool("ja"), None);
    assert_eq!(parse_bool(""), None);
}
