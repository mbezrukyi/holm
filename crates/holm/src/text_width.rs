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
mod tests {
    use super::*;

    #[test]
    fn a_width_out_of_the_range_changes_nothing() {
        let mut width = TextWidthSetting::default();
        assert_eq!(width.chars(), 80);
        for bad in [39, 201, 0, -40, i64::from(u16::MAX) + 40] {
            assert!(!width.set_chars(bad), "{bad}");
        }
        assert_eq!(width.chars(), 80);
        assert!(width.set_chars(40) && width.set_chars(200));
        assert_eq!(width.chars(), 200);
    }

    #[test]
    fn full_keeps_the_width_for_the_editor_and_the_next_limited() {
        let mut width = TextWidthSetting::default();
        width.set_chars(120);
        assert_eq!(width.text_width(), TextWidth::Full);
        width.limited = true;
        assert_eq!(width.text_width(), TextWidth::Limited(120));
    }
}
