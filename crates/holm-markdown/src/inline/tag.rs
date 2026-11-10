//! The HTML tags that Holm reads as styles: `<u>`, `<span style="color: …">`, `<mark>`,
//! `<b>`, `<strong>`, `<i>`, `<em>`, `<s>` and `<del>`.

use super::Kind;
use crate::Color;

/// A Holm tag that opens or closes a style.
pub(super) enum Tag {
    Open(&'static str, Kind),
    Close(&'static str),
}

const NAMES: [&str; 9] = ["u", "span", "mark", "b", "strong", "i", "em", "s", "del"];

/// Reads one raw HTML tag. Other tags, and a `<span>` with no hex color, give `None`.
pub(super) fn read(source: &str) -> Option<Tag> {
    let inner = source.strip_prefix('<')?.strip_suffix('>')?;
    if let Some(name) = inner.strip_prefix('/') {
        return known(name.trim_end()).map(Tag::Close);
    }
    if inner.ends_with('/') {
        return None;
    }
    let end = inner
        .find(|c: char| c.is_ascii_whitespace())
        .unwrap_or(inner.len());
    let name = known(&inner[..end])?;
    let style = attribute(&inner[end..], "style");
    let kind = match name {
        "u" => Kind::Underline,
        "span" => Kind::Color(css_color(style?, &["color"])?),
        "mark" => {
            Kind::Highlight(style.and_then(|s| css_color(s, &["background", "background-color"])))
        }
        "b" | "strong" => Kind::Strong,
        "i" | "em" => Kind::Emphasis,
        _ => Kind::Strikethrough,
    };
    Some(Tag::Open(name, kind))
}

fn known(name: &str) -> Option<&'static str> {
    NAMES
        .iter()
        .find(|known| known.eq_ignore_ascii_case(name))
        .copied()
}

/// The value of the attribute `name` in the attribute list `attributes`.
fn attribute<'a>(attributes: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = attributes.trim_start();
    while !rest.is_empty() {
        let name_end = rest
            .find(|c: char| c.is_ascii_whitespace() || c == '=')
            .unwrap_or(rest.len());
        let found = &rest[..name_end];
        rest = rest[name_end..].trim_start();
        let mut value = "";
        if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start();
            let (v, tail) =
                if let Some(quote) = after.chars().next().filter(|c| matches!(c, '"' | '\'')) {
                    let close = after[1..].find(quote)? + 1;
                    (&after[1..close], &after[close + 1..])
                } else {
                    let end = after
                        .find(|c: char| c.is_ascii_whitespace())
                        .unwrap_or(after.len());
                    (&after[..end], &after[end..])
                };
            value = v;
            rest = tail.trim_start();
        }
        if found.eq_ignore_ascii_case(name) {
            return Some(value);
        }
    }
    None
}

/// The color of the first of `properties` in a CSS declaration list.
fn css_color(style: &str, properties: &[&str]) -> Option<Color> {
    style.split(';').find_map(|declaration| {
        let (property, value) = declaration.split_once(':')?;
        let property = property.trim();
        properties
            .iter()
            .any(|p| p.eq_ignore_ascii_case(property))
            .then(|| Color::from_hex(value.trim()))
            .flatten()
    })
}
