//! Holm's palette colors and their tones for dark mode.
//!
//! A note stores each color as hex. Holm finds its own palette hexes in it and shows their dark
//! tone in dark mode. Any other color shows as it is.

use holm_markdown::Color;

const fn hex(value: u32) -> Color {
    #[allow(clippy::cast_possible_truncation, reason = "each part is one byte")]
    Color {
        r: (value >> 16) as u8,
        g: (value >> 8) as u8,
        b: value as u8,
    }
}

/// Each palette color, with its dark tone. Colors of old palette versions stay in this list.
const DARK_TONES: [(Color, Color); 6] = [
    (hex(0x00c4_553a), hex(0x00e8_8a70)),
    (hex(0x00c7_7c1e), hex(0x00e6_a85a)),
    (hex(0x00fd_e68a), hex(0x006b_5a1e)),
    (hex(0x005f_8f4e), hex(0x008f_c07c)),
    (hex(0x003f_6fa8), hex(0x007f_a8dc)),
    (hex(0x008a_5aa8), hex(0x00b8_92d4)),
];

/// The color to show for `color` in dark mode.
pub(crate) fn dark_tone(color: Color) -> Color {
    DARK_TONES
        .iter()
        .find(|(light, _)| *light == color)
        .map_or(color, |(_, dark)| *dark)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_colors_get_their_dark_tone_and_others_stay() {
        assert_eq!(dark_tone(hex(0x00c4_553a)), hex(0x00e8_8a70));
        assert_eq!(dark_tone(hex(0x0012_3456)), hex(0x0012_3456));
    }
}
