use super::Kind;
use crate::Color;

pub(super) enum HtmlTag {
    Open(&'static str, Kind),
    Close(&'static str),
}

const NAMES: [&str; 9] = ["u", "span", "mark", "b", "strong", "i", "em", "s", "del"];

/// Other HTML tags, and a `<span>` with no hex color, give `None`.
pub(super) fn read(source: &str) -> Option<HtmlTag> {
    let inner = source.strip_prefix('<')?.strip_suffix('>')?;
    if let Some(name) = inner.strip_prefix('/') {
        return known(name.trim_end()).map(HtmlTag::Close);
    }
    if inner.ends_with('/') {
        return None;
    }
    let end = inner.find(|c: char| c.is_ascii_whitespace()).unwrap_or(inner.len());
    let name = known(&inner[..end])?;
    let style = attribute(&inner[end..], "style");
    let kind = match name {
        "u" => Kind::Underline,
        "span" => Kind::Color(css_color(style?, &["color"])?),
        "mark" => Kind::Highlight(style.and_then(|s| css_color(s, &["background", "background-color"]))),
        "b" | "strong" => Kind::Strong,
        "i" | "em" => Kind::Emphasis,
        _ => Kind::Strikethrough,
    };
    Some(HtmlTag::Open(name, kind))
}

fn known(name: &str) -> Option<&'static str> {
    NAMES.iter().find(|known| known.eq_ignore_ascii_case(name)).copied()
}

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
            let (v, tail) = if let Some(quote) = after.chars().next().filter(|c| matches!(c, '"' | '\'')) {
                let close = after[1..].find(quote)? + 1;
                (&after[1..close], &after[close + 1..])
            } else {
                let end = after.find(|c: char| c.is_ascii_whitespace()).unwrap_or(after.len());
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

/// The color of the first declaration that sets one of `properties`.
fn css_color(declarations: &str, properties: &[&str]) -> Option<Color> {
    declarations.split(';').find_map(|declaration| {
        let (property, value) = declaration.split_once(':')?;
        let property = property.trim();
        properties
            .iter()
            .any(|p| p.eq_ignore_ascii_case(property))
            .then(|| Color::from_hex(value.trim()))
            .flatten()
    })
}
