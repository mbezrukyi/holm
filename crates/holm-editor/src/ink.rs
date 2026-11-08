//! Where the ink of a letter crosses an underline, so the underline can leave a gap there, like
//! a browser does.
//!
//! GPUI does not give the outlines of letters, so this module reads the font files itself, with
//! `fontdb` and `ttf-parser`. This module has no GPUI.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use fontdb::{Database, Family, Query, Style, Weight};
use ttf_parser::{Face, OutlineBuilder};

/// The system fonts. The scan takes some milliseconds, so it happens once.
static FONTS: LazyLock<Database> = LazyLock::new(|| {
    let mut fonts = Database::new();
    fonts.load_system_fonts();
    fonts
});

/// The gaps found so far, by letter, font and band.
static GAPS: LazyLock<Mutex<HashMap<Key, Vec<Gap>>>> = LazyLock::new(Mutex::default);

/// Scans the system fonts. Call it on a background thread, so the first underline does not wait.
pub(crate) fn warm_up() {
    LazyLock::force(&FONTS);
}

/// A font face, as the text style names it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FontKey {
    pub(crate) family: String,
    pub(crate) bold: bool,
    pub(crate) italic: bool,
}

/// An underline band below the text line, in pixels, at one text size.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Band {
    pub(crate) font_size: f32,
    /// The top of the band, below the text line.
    pub(crate) top: f32,
    /// The bottom of the band, below the text line.
    pub(crate) bottom: f32,
}

/// A gap from the left to the right x, in pixels from the start of the letter.
pub(crate) type Gap = (f32, f32);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    font: FontKey,
    letter: char,
    /// The band, in hundredths of a pixel.
    band: [i32; 3],
}

/// The parts of the band that the ink of `letter` covers, sorted. A font or a letter that is not
/// found has none.
pub(crate) fn gaps(font: &FontKey, letter: char, band: Band) -> Vec<Gap> {
    #[allow(clippy::cast_possible_truncation, reason = "a small pixel value")]
    let hundredths = |value: f32| (value * 100.).round() as i32;
    let key = Key {
        font: font.clone(),
        letter,
        band: [
            hundredths(band.font_size),
            hundredths(band.top),
            hundredths(band.bottom),
        ],
    };
    if let Ok(cache) = GAPS.lock()
        && let Some(found) = cache.get(&key)
    {
        return found.clone();
    }
    let found = find(font, letter, band).unwrap_or_default();
    if let Ok(mut cache) = GAPS.lock() {
        cache.insert(key, found.clone());
    }
    found
}

fn find(font: &FontKey, letter: char, band: Band) -> Option<Vec<Gap>> {
    let query = Query {
        families: &[Family::Name(&font.family)],
        weight: if font.bold {
            Weight::BOLD
        } else {
            Weight::NORMAL
        },
        style: if font.italic {
            Style::Italic
        } else {
            Style::Normal
        },
        ..Query::default()
    };
    let id = FONTS.query(&query)?;
    FONTS
        .with_face_data(id, |data, index| {
            let face = Face::parse(data, index).ok()?;
            Some(face_gaps(&face, letter, band))
        })
        .flatten()
}

/// The parts of the band that the ink of `letter` in `face` covers.
fn face_gaps(face: &Face<'_>, letter: char, band: Band) -> Vec<Gap> {
    let Some(glyph) = face.glyph_index(letter) else {
        return Vec::new();
    };
    let scale = band.font_size / f32::from(face.units_per_em());
    let mut ink = Ink {
        at: (0., 0.),
        start: (0., 0.),
        edges: Vec::new(),
    };
    if face.outline_glyph(glyph, &mut ink).is_none() {
        return Vec::new();
    }
    // Font units grow upward from the text line. The band lies below it. Some heights in the
    // band each give the parts inside the outline, between pairs of edges.
    let mut spans: Vec<Gap> = Vec::new();
    for step in 0..=SAMPLES {
        let below = band.top + (band.bottom - band.top) * f32::from(step) / f32::from(SAMPLES);
        let y = -below / scale;
        let mut xs: Vec<f32> = ink
            .edges
            .iter()
            .filter(|(from, to)| (from.1 <= y) != (to.1 <= y))
            .map(|(from, to)| from.0 + (to.0 - from.0) * (y - from.1) / (to.1 - from.1))
            .collect();
        xs.sort_by(f32::total_cmp);
        spans.extend(
            xs.as_chunks::<2>()
                .0
                .iter()
                .map(|[left, right]| (left * scale, right * scale)),
        );
    }
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut gaps: Vec<Gap> = Vec::new();
    for (left, right) in spans {
        match gaps.last_mut() {
            Some(last) if left <= last.1 => last.1 = last.1.max(right),
            _ => gaps.push((left, right)),
        }
    }
    gaps
}

/// How many steps apart the heights in the band are.
const SAMPLES: u8 = 4;

/// Collects the outline of a letter as straight edges, in font units.
struct Ink {
    at: (f32, f32),
    start: (f32, f32),
    edges: Vec<((f32, f32), (f32, f32))>,
}

/// How many straight pieces stand for one curve.
const STEPS: u8 = 8;

impl Ink {
    fn piece(&mut self, to: (f32, f32)) {
        self.edges.push((self.at, to));
        self.at = to;
    }
}

impl OutlineBuilder for Ink {
    fn move_to(&mut self, x: f32, y: f32) {
        self.at = (x, y);
        self.start = (x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.piece((x, y));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let from = self.at;
        for step in 1..=STEPS {
            let t = f32::from(step) / f32::from(STEPS);
            let weights = [(1. - t) * (1. - t), 2. * (1. - t) * t, t * t];
            let point = |p: [f32; 3]| weights.iter().zip(p).map(|(w, p)| w * p).sum();
            self.piece((point([from.0, x1, x]), point([from.1, y1, y])));
        }
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let from = self.at;
        for step in 1..=STEPS {
            let t = f32::from(step) / f32::from(STEPS);
            let rest = 1. - t;
            let weights = [
                rest * rest * rest,
                3. * rest * rest * t,
                3. * rest * t * t,
                t * t * t,
            ];
            let point = |p: [f32; 4]| weights.iter().zip(p).map(|(w, p)| w * p).sum();
            self.piece((point([from.0, x1, x2, x]), point([from.1, y1, y2, y])));
        }
    }

    fn close(&mut self) {
        self.piece(self.start);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FONT: &[u8] = include_bytes!("../tests/fonts/FiraMono-Medium.ttf");

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
}
