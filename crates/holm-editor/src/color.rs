use gpui_kit::{
    AnyElement, Hsla, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Point, Styled, anchored,
    deferred, div, px, rems,
};
use holm_markdown::Color;

use crate::element::Theme;
use crate::model::Format;
use crate::palette;
use crate::shortcuts::{Command, Place};

pub(crate) const CUSTOM_COUNT: usize = 3;
const MAX_HEX_DIGITS: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Paint {
    Text,
    Highlight,
}

impl Paint {
    fn palette(self) -> &'static [Color; 6] {
        match self {
            Self::Text => &palette::TEXT,
            Self::Highlight => &palette::HIGHLIGHT,
        }
    }

    /// `None` removes the color.
    pub(crate) fn format(self, color: Option<Color>) -> Format {
        match self {
            Self::Text => Format::TextColor(color),
            Self::Highlight => Format::HighlightColor(color),
        }
    }

    pub(crate) fn is_custom(self, color: Color) -> bool {
        !self.palette().contains(&color)
    }
}

/// `number` counts from 1.
fn nth(number: u8) -> usize {
    usize::from(number).wrapping_sub(1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyResult {
    Stay,
    /// Closes with no change.
    Close,
    /// Sets the color, or removes it with `None`, and closes.
    Apply(Option<Color>),
}

pub(crate) struct ColorWindow {
    pub(crate) paint: Paint,
    /// `Some` while the hex field is open.
    hex: Option<String>,
}

impl ColorWindow {
    pub(crate) fn new(paint: Paint) -> Self {
        Self { paint, hex: None }
    }

    pub(crate) fn place(&self) -> Place {
        if self.hex.is_some() {
            Place::HexField
        } else {
            Place::ColorWindow
        }
    }

    /// `custom` are the last custom colors, the newest first.
    ///
    /// Keys 1 to 6 pick a palette color, 7 to 9 a custom color, and 0 removes the color. `+` opens the hex field. In
    /// the field, Enter takes a `rgb` or `rrggbb` value. Esc closes.
    pub(crate) fn command(&mut self, command: Command, custom: &[Color]) -> KeyResult {
        if command == Command::CloseColor {
            return KeyResult::Close;
        }
        if let Some(hex) = &self.hex {
            return match command {
                Command::TakeHex => {
                    Color::from_hex(&format!("#{hex}")).map_or(KeyResult::Stay, |color| KeyResult::Apply(Some(color)))
                }
                _ => KeyResult::Stay,
            };
        }
        let picked = match command {
            Command::HexField => {
                self.hex = Some(String::new());
                return KeyResult::Stay;
            }
            Command::RemoveColor => return KeyResult::Apply(None),
            Command::PaletteColor(number) => self.paint.palette().get(nth(number)),
            Command::CustomColor(number) => custom.get(nth(number)),
            _ => None,
        };
        picked.map_or(KeyResult::Stay, |color| KeyResult::Apply(Some(*color)))
    }

    pub(crate) fn pop(&mut self) {
        if let Some(hex) = &mut self.hex {
            hex.pop();
        }
    }

    pub(crate) fn push(&mut self, text: &str) {
        if let Some(hex) = &mut self.hex {
            let room = MAX_HEX_DIGITS - hex.len();
            hex.extend(text.chars().filter(char::is_ascii_hexdigit).take(room));
        }
    }

    /// `at` is a point in the window. The color window sits below it and moves to stay inside the window.
    pub(crate) fn view(&self, theme: &Theme, custom: &[Color], at: Point<Pixels>) -> AnyElement {
        let paint = self.paint;
        let palette = paint
            .palette()
            .iter()
            .enumerate()
            .map(|(ix, color)| swatch(theme, paint, Some(*color), ix + 1));
        let custom = (0..CUSTOM_COUNT).map(|ix| swatch(theme, paint, custom.get(ix).copied(), ix + 7));
        let remove = key_box(theme).text_color(theme.muted).child("0");
        let window = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .p(px(6.))
            .rounded(px(8.))
            .bg(theme.panel)
            .border_1()
            .border_color(theme.line)
            .text_color(theme.text)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children(palette)
            .child(div().w(px(1.)).h(rems(1.2)).bg(theme.line))
            .children(custom)
            .child(remove)
            .child(self.hex_field(theme));
        deferred(anchored().position(at).snap_to_window_with_margin(px(8.)).child(window)).into_any_element()
    }

    /// The `+` key, or the hex field with a sample of its color when the value is whole.
    fn hex_field(&self, theme: &Theme) -> AnyElement {
        let Some(hex) = &self.hex else {
            return key_box(theme).text_color(theme.muted).child("+").into_any_element();
        };
        let caret = div().w(px(2.)).h(rems(1.)).bg(theme.cursor);
        let sample = Color::from_hex(&format!("#{hex}")).map(|color| key_box(theme).bg(theme.note_color(color)));
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .min_w(rems(4.5))
                    .child(format!("#{hex}"))
                    .child(caret),
            )
            .children(sample)
            .into_any_element()
    }
}

fn key_box(theme: &Theme) -> gpui_kit::Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(rems(1.5))
        .rounded(px(4.))
        .border_1()
        .border_color(theme.line)
}

fn swatch(theme: &Theme, paint: Paint, color: Option<Color>, number: usize) -> gpui_kit::Div {
    let label = number.to_string();
    let base = key_box(theme);
    let Some(color) = color else {
        return base.text_color(theme.muted.opacity(0.5)).child(label);
    };
    let shown: Hsla = theme.note_color(color);
    match paint {
        Paint::Text => base.bg(theme.background).text_color(shown).child(label),
        Paint::Highlight => base.bg(shown).text_color(theme.text).child(label),
    }
}

/// Returns whether the list changed.
pub(crate) fn remember(custom: &mut Vec<Color>, color: Color) -> bool {
    let before = custom.clone();
    custom.retain(|c| *c != color);
    custom.insert(0, color);
    custom.truncate(CUSTOM_COUNT);
    *custom != before
}

#[cfg(test)]
mod tests;
