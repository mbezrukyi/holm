//! Holm's inline HTML tags: `<u>`, `<span style="color: ...">` and `<mark>`.

use super::{Color, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Name {
    U,
    Span,
    Mark,
}

pub(super) enum Tag {
    Open(Name, Style),
    Close(Name),
}

/// Reads one inline HTML tag. Other tags, and a `<span>` with no hex color, give `None`.
pub(super) fn parse(source: &str) -> Option<Tag> {
    let inner = source.strip_prefix('<')?.strip_suffix('>')?;
    if let Some(name) = inner.strip_prefix('/') {
        return name_of(name.trim_end()).map(Tag::Close);
    }
    let name_len = inner
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(inner.len());
    let name = name_of(&inner[..name_len])?;
    let style = style_attribute(&inner[name_len..]);
    let css = |property: &[&str]| style.and_then(|style| css_color(style, property));
    let style = match name {
        Name::U => Style::Underline,
        Name::Span => Style::Color(css(&["color"])?),
        Name::Mark => Style::Highlight(css(&["background", "background-color"])),
    };
    Some(Tag::Open(name, style))
}

fn name_of(name: &str) -> Option<Name> {
    [("u", Name::U), ("span", Name::Span), ("mark", Name::Mark)]
        .into_iter()
        .find_map(|(text, n)| name.eq_ignore_ascii_case(text).then_some(n))
}

/// The value of the `style` attribute.
fn style_attribute(mut rest: &str) -> Option<&str> {
    loop {
        rest = rest.trim_start();
        let name_len = rest
            .find(|c: char| c == '=' || c == '/' || c.is_whitespace())
            .unwrap_or(rest.len());
        if name_len == 0 {
            return None;
        }
        let name = &rest[..name_len];
        rest = rest[name_len..].trim_start();
        let mut value = "";
        if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start();
            let (v, r) = if let Some(quote @ ('"' | '\'')) = after.chars().next() {
                let body = &after[1..];
                let end = body.find(quote).unwrap_or(body.len());
                (&body[..end], body.get(end + 1..).unwrap_or(""))
            } else {
                let end = after.find(char::is_whitespace).unwrap_or(after.len());
                after.split_at(end)
            };
            value = v;
            rest = r;
        }
        if name.eq_ignore_ascii_case("style") {
            return Some(value);
        }
    }
}

/// The color of the first of `properties` in a CSS declaration list.
fn css_color(style: &str, properties: &[&str]) -> Option<Color> {
    style.split(';').find_map(|declaration| {
        let (property, value) = declaration.split_once(':')?;
        let property = property.trim();
        properties
            .iter()
            .any(|p| property.eq_ignore_ascii_case(p))
            .then(|| hex_color(value.trim()))?
    })
}

fn hex_color(value: &str) -> Option<Color> {
    let hex = value.strip_prefix('#')?;
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |s: &str| u8::from_str_radix(s, 16).ok();
    match hex.len() {
        3 => {
            let [r, g, b] = [0, 1, 2].map(|i| channel(&hex[i..=i]).map(|v| v * 17));
            Some(Color {
                r: r?,
                g: g?,
                b: b?,
            })
        }
        6 => Some(Color {
            r: channel(&hex[0..2])?,
            g: channel(&hex[2..4])?,
            b: channel(&hex[4..6])?,
        }),
        _ => None,
    }
}
