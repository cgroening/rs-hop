//! The `[appearance]` config section: theme name, per-color overrides and the
//! glyph variant.
//!
//! These feed the resolved [`Skin`](crate::theme::Skin) the TUI renders with and
//! the CLI derives its sparcli theme from, so the look flows from one config.

use std::collections::BTreeMap;

use crate::theme::{DEFAULT_THEME, GlyphVariant};

/// The appearance settings: which theme, any per-color overrides, and whether to
/// render Unicode or ASCII glyphs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Appearance {
    /// The active theme name (a built-in or a `[themes.<name>]` entry).
    pub theme: String,
    /// Per-palette-color overrides (`name -> value`); an empty map keeps the
    /// theme colors.
    pub colors: BTreeMap<String, String>,
    /// The glyph variant (Unicode or ASCII).
    pub glyphs: GlyphVariant,
    /// How far dimmed colored text is darkened while the palette color `dim`
    /// is set, in percent (0 = unchanged, 100 = black).
    pub dim_amount: u8,
}

/// The default of [`Appearance::dim_amount`].
pub const DEFAULT_DIM_AMOUNT: u8 = 40;

/// The largest [`Appearance::dim_amount`].
pub const MAX_DIM_AMOUNT: u8 = 100;

impl Appearance {
    /// [`Appearance::dim_amount`] as the `0.0`-`1.0` fraction the toolkit
    /// takes.
    #[must_use]
    pub fn dim_fraction(&self) -> f32 {
        f32::from(self.dim_amount.min(MAX_DIM_AMOUNT)) / 100.0
    }
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance {
            theme: DEFAULT_THEME.to_string(),
            colors: BTreeMap::new(),
            glyphs: GlyphVariant::default(),
            dim_amount: DEFAULT_DIM_AMOUNT,
        }
    }
}
