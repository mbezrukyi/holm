//! The width of the text column as the settings keep it: "Full" or "Limited", and the width in chars apart from the
//! mode. So "Full" keeps the width for the next "Limited". The range and the default live here, and nothing else knows
//! them.

use holm_editor::TextWidth;

/// The narrowest and the widest column, in chars.
const CHARS: std::ops::RangeInclusive<u16> = 40..=200;

const DEFAULT_CHARS: u16 = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextWidthSetting {
    pub(crate) limited: bool,
    chars: u16,
}

impl Default for TextWidthSetting {
    fn default() -> Self {
        Self {
            limited: false,
            chars: DEFAULT_CHARS,
        }
    }
}

impl TextWidthSetting {
    /// "Full" keeps it too.
    pub(crate) fn chars(self) -> u16 {
        self.chars
    }

    /// A width out of the range changes nothing and gives false.
    pub(crate) fn set_chars(&mut self, chars: i64) -> bool {
        match u16::try_from(chars) {
            Ok(chars) if CHARS.contains(&chars) => {
                self.chars = chars;
                true
            }
            _ => false,
        }
    }

    pub(crate) fn text_width(self) -> TextWidth {
        if self.limited {
            TextWidth::Limited(self.chars)
        } else {
            TextWidth::Full
        }
    }
}

#[cfg(test)]
mod tests;
