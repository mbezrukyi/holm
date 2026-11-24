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
