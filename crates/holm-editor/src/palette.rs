//! Holm's palette colors and their tones for dark mode.
//!
//! A note stores each color as hex. Holm finds its own palette hexes in it and shows their dark
//! tone in dark mode. Any other color shows as it is.

use holm_markdown::Color;

const fn hex(value: u32) -> Color {
    let [_, r, g, b] = value.to_be_bytes();
    Color { r, g, b }
}

/// The text colors of the color window, for keys 1 to 6.
pub(crate) const TEXT: [Color; 6] = [
    hex(0x00b5_442c),
    hex(0x00a8_5d12),
    hex(0x004f_7d3c),
    hex(0x002f_7a78),
    hex(0x003a_66a0),
    hex(0x0082_50a0),
];

/// The highlight colors of the color window, for keys 1 to 6. The first one is the color of
/// `==text==`.
pub(crate) const HIGHLIGHT: [Color; 6] = [
    hex(0x00fd_e68a),
    hex(0x00f9_cfa4),
    hex(0x00f5_c6d3),
    hex(0x00cf_e6b8),
    hex(0x00c6_dbf2),
    hex(0x00dc_cff0),
];

/// Each palette color, with its dark tone. Colors of old palette versions stay in this list.
const DARK_TONES: [(Color, Color); 17] = [
    (TEXT[0], hex(0x00ec_8a70)),
    (TEXT[1], hex(0x00e6_a85a)),
    (TEXT[2], hex(0x008f_c07c)),
    (TEXT[3], hex(0x006c_c2bd)),
    (TEXT[4], hex(0x007f_a8dc)),
    (TEXT[5], hex(0x00b8_92d4)),
    (HIGHLIGHT[0], hex(0x006b_5a1e)),
    (HIGHLIGHT[1], hex(0x007a_4a22)),
    (HIGHLIGHT[2], hex(0x0074_343f)),
    (HIGHLIGHT[3], hex(0x003d_5a2c)),
    (HIGHLIGHT[4], hex(0x002c_4a6b)),
    (HIGHLIGHT[5], hex(0x0054_406e)),
    // The first palette: one list for text and highlight.
    (hex(0x00c4_553a), hex(0x00e8_8a70)),
    (hex(0x00c7_7c1e), hex(0x00e6_a85a)),
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
        assert_eq!(dark_tone(TEXT[0]), hex(0x00ec_8a70));
        assert_eq!(dark_tone(hex(0x00c4_553a)), hex(0x00e8_8a70));
        assert_eq!(dark_tone(hex(0x0012_3456)), hex(0x0012_3456));
    }

    #[test]
    fn every_palette_color_has_one_dark_tone() {
        for color in TEXT.iter().chain(&HIGHLIGHT) {
            let tones = DARK_TONES.iter().filter(|(light, _)| light == color).count();
            assert_eq!(tones, 1, "{color}");
        }
    }
}
