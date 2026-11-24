use super::*;

const FONT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test_data/fonts/FiraMono-Medium.ttf"
));

fn band() -> Band {
    Band {
        font_size: 16.,
        top: 1.6,
        bottom: 2.6,
    }
}

fn face_gaps_of(letter: char) -> Vec<Gap> {
    let face = Face::parse(FONT, 0).unwrap_or_else(|error| panic!("{error}"));
    face_gaps(&face, letter, band())
}

#[test]
fn a_letter_without_a_tail_has_no_gap() {
    for letter in ['o', 'a', 'x', 'k', ' '] {
        assert_eq!(face_gaps_of(letter), [], "{letter}");
    }
}

#[test]
fn a_tail_gets_a_gap_narrower_than_the_letter() {
    let advance = 16. * 0.6;
    for letter in ['p', 'q', 'y'] {
        let gaps = face_gaps_of(letter);
        assert!(!gaps.is_empty(), "{letter}");
        let width: f32 = gaps.iter().map(|(left, right)| right - left).sum();
        assert!(width > 0. && width < advance * 0.6, "{letter}: {gaps:?}");
    }
}

#[test]
fn an_unknown_font_has_no_gaps() {
    let font = FontKey {
        family: "No Such Font".into(),
        bold: false,
        italic: false,
    };
    assert_eq!(gaps(&font, 'p', band()), []);
}
