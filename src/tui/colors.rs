//! Named colour constants and styles, kept central and subdued (one soft accent
//! plus dim greys), so colour carries meaning rather than shouting.

use ratatui::style::{Color, Modifier, Style};

/// Accent for the header, the active tab and highlighted keys (muted magenta:
/// distinct from the warm semantic colours, like mdtask's purple accent).
pub const ACCENT: Color = Color::Rgb(247, 163, 189);
/// Dim colour for secondary text (footer hints, separators).
pub const DIM: Color = Color::DarkGray;
/// Slightly brighter grey for the header info line.
pub const MUTED: Color = Color::Rgb(170, 170, 178);
/// Background tint for the cursor row.
pub const SELECTION_BG: Color = Color::Rgb(40, 40, 60);
/// Background tint for multi-selected (checked) rows; fainter than the cursor.
pub const MULTI_SELECT_BG: Color = Color::Rgb(58, 44, 64);
/// Colour for the missing-path marker and destructive prompts.
pub const DANGER: Color = Color::Rgb(208, 80, 80);
/// Colour for a clean git status.
pub const POSITIVE: Color = Color::Green;
/// Colour for a git status with uncommitted changes (soft amber/yellow).
pub const CHANGES: Color = Color::Rgb(220, 190, 90);
/// Colour for the favourite star.
pub const FAVOURITE: Color = Color::Rgb(229, 192, 123);
/// Colour of the text-input block cursor.
pub const CURSOR: Color = Color::Rgb(208, 80, 80);

/// Colour for the active tab in the header: the hue complement of [`ACCENT`]
/// (opposite on the colour wheel, same lightness/saturation), so it always
/// pairs with the accent even if the accent is changed.
pub fn tab_active() -> Color {
    complement(ACCENT)
}

/// The hue complement (rotated 180°) of an RGB colour; non-RGB colours pass
/// through unchanged.
fn complement(color: Color) -> Color {
    let Color::Rgb(r, g, b) = color else {
        return color;
    };
    let (h, s, l) = rgb_to_hsl(r, g, b);
    let (r, g, b) = hsl_to_rgb((h + 180.0).rem_euclid(360.0), s, l);
    Color::Rgb(r, g, b)
}

/// Converts RGB to HSL (`h` in degrees `[0,360)`, `s`/`l` in `[0,1]`).
fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let (rf, gf, bf) = (
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    );
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == rf {
        60.0 * ((gf - bf) / d).rem_euclid(6.0)
    } else if max == gf {
        60.0 * ((bf - rf) / d + 2.0)
    } else {
        60.0 * ((rf - gf) / d + 4.0)
    };
    (h.rem_euclid(360.0), s, l)
}

/// Converts HSL back to RGB (inputs as produced by [`rgb_to_hsl`]).
fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r1, g1, b1) = match hp.floor() as i64 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to_u8 = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (to_u8(r1), to_u8(g1), to_u8(b1))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complement_rotates_hue_and_round_trips() {
        // Applying the complement twice returns (close to) the original hue.
        let once = complement(ACCENT);
        let twice = complement(once);
        let (Color::Rgb(r0, g0, b0), Color::Rgb(r2, g2, b2)) = (ACCENT, twice)
        else {
            panic!("expected rgb colours");
        };
        let close = |a: u8, b: u8| (i32::from(a) - i32::from(b)).abs() <= 2;
        assert!(close(r0, r2) && close(g0, g2) && close(b0, b2));
        // The complement of the rose accent is a green/teal (g is the max).
        let Color::Rgb(r, g, b) = once else {
            panic!("rgb");
        };
        assert!(g > r && g > b);
    }
}
