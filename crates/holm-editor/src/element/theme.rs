//! The warm Holm colors of the text column, for light and dark mode.

use gpui_kit::{Hsla, rgb, rgba};
use holm_markdown::{CalloutKind, Color};

use crate::Appearance;
use crate::highlight::Token;
use crate::palette;

/// The colors of one callout kind.
#[derive(Clone, Copy)]
pub(crate) struct CalloutColors {
    pub(crate) accent: Hsla,
    pub(crate) background: Hsla,
}

#[derive(Clone)]
pub(crate) struct Theme {
    pub(crate) dark: bool,
    pub(crate) background: Hsla,
    pub(crate) text: Hsla,
    /// Syntax, list markers, quote text and other quiet parts.
    pub(crate) muted: Hsla,
    pub(crate) link: Hsla,
    pub(crate) cursor: Hsla,
    pub(crate) selection: Hsla,
    pub(crate) highlight: Hsla,
    pub(crate) code_background: Hsla,
    /// Lines of quotes, tables and rules, and the borders of boxes.
    pub(crate) line: Hsla,
    /// The background of table heads, file cards and loading images.
    pub(crate) panel: Hsla,
    pub(crate) callouts: [CalloutColors; 5],
    tokens: [Hsla; 10],
}

fn callout(accent: u32, background: u32) -> CalloutColors {
    CalloutColors {
        accent: rgb(accent).into(),
        background: rgba(background).into(),
    }
}

fn colors<const N: usize>(values: [u32; N]) -> [Hsla; N] {
    values.map(|value| rgb(value).into())
}

impl Theme {
    pub(crate) fn new(appearance: Appearance) -> Self {
        match appearance {
            Appearance::Light => Self::light(),
            Appearance::Dark => Self::dark(),
        }
    }

    fn light() -> Self {
        Self {
            dark: false,
            background: rgb(0x00fb_f7f0).into(),
            text: rgb(0x002b_2520).into(),
            muted: rgb(0x009a_8d7e).into(),
            link: rgb(0x008a_5a2b).into(),
            cursor: rgb(0x008a_5a2b).into(),
            selection: rgba(0xd9c7_b080).into(),
            highlight: rgba(0xfde6_8aff).into(),
            code_background: rgb(0x00f2_ebe0).into(),
            line: rgb(0x00e2_d8c8).into(),
            panel: rgb(0x00f4_eee4).into(),
            callouts: [
                callout(0x003f_6fa8, 0x3f6f_a814),
                callout(0x005f_8f4e, 0x5f8f_4e14),
                callout(0x008a_5aa8, 0x8a5a_a814),
                callout(0x00c7_7c1e, 0xc77c_1e14),
                callout(0x00c4_553a, 0xc455_3a14),
            ],
            tokens: colors([
                0x009a_8d7e,
                0x005f_8f4e,
                0x00c0_6a2a,
                0x00c0_6a2a,
                0x008a_5aa8,
                0x003f_6fa8,
                0x00a2_7426,
                0x00c4_553a,
                0x00a2_7426,
                0x007d_7064,
            ]),
        }
    }

    fn dark() -> Self {
        Self {
            dark: true,
            background: rgb(0x001f_1b17).into(),
            text: rgb(0x00e8_dfd3).into(),
            muted: rgb(0x0088_7c6f).into(),
            link: rgb(0x00d9_a66b).into(),
            cursor: rgb(0x00d9_a66b).into(),
            selection: rgba(0x6a58_4480).into(),
            highlight: rgba(0x6b5a_1eff).into(),
            code_background: rgb(0x0029_241f).into(),
            line: rgb(0x003d_352c).into(),
            panel: rgb(0x002a_251f).into(),
            callouts: [
                callout(0x007f_a8dc, 0x7fa8_dc1a),
                callout(0x008f_c07c, 0x8fc0_7c1a),
                callout(0x00b8_92d4, 0xb892_d41a),
                callout(0x00e6_a85a, 0xe6a8_5a1a),
                callout(0x00e8_8a70, 0xe88a_701a),
            ],
            tokens: colors([
                0x0088_7c6f,
                0x008f_c07c,
                0x00e6_a05e,
                0x00e6_a05e,
                0x00b8_92d4,
                0x007f_a8dc,
                0x00d8_b26a,
                0x00e8_8a70,
                0x00d8_b26a,
                0x00a8_9a8a,
            ]),
        }
    }

    pub(crate) fn callout(&self, kind: CalloutKind) -> CalloutColors {
        let ix = match kind {
            CalloutKind::Note => 0,
            CalloutKind::Tip => 1,
            CalloutKind::Important => 2,
            CalloutKind::Warning => 3,
            CalloutKind::Caution => 4,
        };
        self.callouts[ix]
    }

    pub(crate) fn token(&self, token: Token) -> Hsla {
        let ix = match token {
            Token::Comment => 0,
            Token::String => 1,
            Token::Number => 2,
            Token::Constant => 3,
            Token::Keyword => 4,
            Token::Function => 5,
            Token::Type => 6,
            Token::Tag => 7,
            Token::Attribute => 8,
            Token::Punctuation => 9,
        };
        self.tokens[ix]
    }

    /// A color from the note. Palette colors get their dark tone in dark mode.
    pub(crate) fn note_color(&self, color: Color) -> Hsla {
        let color = if self.dark {
            palette::dark_tone(color)
        } else {
            color
        };
        rgb(u32::from(color.r) << 16 | u32::from(color.g) << 8 | u32::from(color.b)).into()
    }
}
