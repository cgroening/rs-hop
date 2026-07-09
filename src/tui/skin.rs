//! hop's color roles, resolved from the config-driven [`Palette`].
//!
//! The pre-migration `colors` module held compile-time constants; this replaces
//! them with a [`Colors`] set built from the active theme's [`Palette`], so the
//! same rose accent (the built-in `default` theme) can be re-themed from config.
//! Each field is a ready ratatui [`Color`]; the panel bands (header/footer/
//! surface) come straight from the palette for the app-frame layout.

use ratatui::style::{Color, Modifier, Style};

use crate::theme::Palette;

/// The resolved color roles used across the TUI, built from a [`Palette`].
#[derive(Debug, Clone, Copy)]
pub struct Colors {
    /// Accent for the brand, active tab and highlighted keys.
    pub accent: Color,
    /// The active tab colour (the accent's complement).
    pub tab_active: Color,
    /// Secondary text (footer hints, separators).
    pub dim: Color,
    /// Slightly brighter grey for the header info line.
    pub muted: Color,
    /// Primary text colour.
    pub foreground: Color,
    /// Background tint for the cursor row.
    pub selection_bg: Color,
    /// Background tint for multi-selected (checked) rows; fainter than the
    /// cursor.
    pub multi_select_bg: Color,
    /// Missing-path marker and destructive prompts.
    pub danger: Color,
    /// Clean git status.
    pub positive: Color,
    /// Git status with uncommitted changes.
    pub changes: Color,
    /// Favourite star.
    pub favourite: Color,
    /// Text-input block caret.
    pub cursor: Color,
    /// Header band background.
    pub header_bg: Color,
    /// Footer / status band background.
    pub footer_bg: Color,
    /// Content surface background (lighter than the bands).
    pub surface_bg: Color,
    /// Border / separator colour.
    pub border: Color,
}

impl Colors {
    /// Builds the color roles from a [`Palette`].
    pub fn from_palette(palette: &Palette) -> Self {
        let rat = ratada::style::to_ratatui;
        Colors {
            accent: rat(palette.accent),
            tab_active: rat(complement(palette.accent)),
            dim: rat(palette.foreground_dim),
            muted: rat(palette.foreground.darken(0.15)),
            foreground: rat(palette.foreground),
            selection_bg: rat(palette.selection),
            multi_select_bg: rat(palette.surface.mix(palette.accent, 0.18)),
            danger: rat(palette.error),
            positive: rat(palette.success),
            changes: rat(palette.warning),
            favourite: Color::Rgb(0xe5, 0xc0, 0x7b),
            cursor: rat(palette.cursor),
            header_bg: rat(palette.header),
            footer_bg: rat(palette.footer),
            surface_bg: rat(palette.surface),
            border: rat(palette.border),
        }
    }

    /// Bold accent style for a table header row.
    pub fn header_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Background tint marking the selected row.
    pub fn selection_style(&self) -> Style {
        Style::default()
            .bg(self.selection_bg)
            .add_modifier(Modifier::BOLD)
    }
}

/// The hue complement (rotated 180°) of a theme colour, keeping its saturation
/// and lightness; the active-tab colour, so it always pairs with the accent.
fn complement(color: crate::theme::Color) -> crate::theme::Color {
    let Some((hue, saturation, lightness)) = color.to_hsl() else {
        return color;
    };
    crate::theme::Color::from_hsl(
        (hue + 180.0).rem_euclid(360.0),
        saturation,
        lightness,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn complement_rotates_the_hue_and_round_trips() {
        let accent = Config::default().palette().accent;
        let once = complement(accent);
        let twice = complement(once);
        let (Some((hue, _, _)), Some((back, _, _))) =
            (accent.to_hsl(), twice.to_hsl())
        else {
            panic!("the default accent is an rgb colour");
        };
        assert!((hue - back).abs() < 1.0, "two rotations return the hue");
        let Some((rotated, _, _)) = once.to_hsl() else {
            panic!("rgb");
        };
        assert!((rotated - (hue + 180.0).rem_euclid(360.0)).abs() < 1.0);
    }
}
