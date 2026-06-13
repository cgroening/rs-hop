//! Named colour constants and styles, kept central and subdued (one soft accent
//! plus dim greys), so colour carries meaning rather than shouting.

use ratatui::style::{Color, Modifier, Style};

/// Accent for the header, the active tab and highlighted keys.
pub const ACCENT: Color = Color::Rgb(109, 208, 255);
/// Dim colour for secondary text.
pub const DIM: Color = Color::DarkGray;
/// Background tint for the selected row.
pub const SELECTION_BG: Color = Color::Rgb(40, 40, 60);
/// Colour for the missing-path marker and destructive prompts.
pub const DANGER: Color = Color::Rgb(208, 80, 80);
/// Colour for a clean git status.
pub const POSITIVE: Color = Color::Green;
/// Colour for the favourite star.
pub const FAVOURITE: Color = Color::Rgb(229, 192, 123);
/// Colour of the text-input block cursor.
pub const CURSOR: Color = Color::Rgb(208, 80, 80);

/// Bold accent style for the table header row.
pub fn header_style() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

/// Background tint marking the selected row.
pub fn selection_style() -> Style {
    Style::default()
        .bg(SELECTION_BG)
        .add_modifier(Modifier::BOLD)
}
