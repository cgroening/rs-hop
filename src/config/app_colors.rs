//! hop's built-in colors: its default theme. This is the one place in the app
//! that holds color values (`tests/theme_only_colours.rs` keeps it so).

use crate::theme::{Color, ThemeColors};

/// hop's built-in default theme: the rose accent kept from the pre-migration
/// look, on a dark scheme whose header/footer bands sit below a lighter content
/// surface (the panel layout). Registered as the `default` theme, replacing the
/// toolkit's teal built-in.
pub const HOP_DEFAULT_THEME: ThemeColors = ThemeColors {
    accent: Color::hex("#f7a3bd"),
    foreground: Color::hex("#e5e5e5"),
    background: Color::hex("#151515"),
    header: Color::hex("#120F10"),
    footer: Color::hex("#120F10"),
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
    dim: Color::Default,
};
