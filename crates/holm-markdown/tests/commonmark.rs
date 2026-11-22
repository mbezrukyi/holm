//! The inline examples of the `CommonMark` 0.31.2 spec: backslash escapes, entities, and the
//! sections from code spans to the end.
//!
//! The test writes HTML from the inline tree and compares it with the spec. Links and images
//! compare without their attributes, because the tree keeps no link title. Examples with blocks
//! other than paragraphs and headings are not checked here: they test the block parser.

#![cfg(test)]

use std::fmt::Write as _;
use std::fs;

use holm_markdown::inline::{Kind, Node};
use holm_markdown::{BlockKind, Body};

/// Examples that need a named entity outside the small table below.
const NAMED_ENTITIES: [u32; 3] = [25, 32, 41];

struct Example {
    number: u32,
    markdown: String,
    html: String,
}

fn examples() -> Vec<Example> {
    let data = fs::read_to_string("tests/data/commonmark-inline.txt").unwrap();
    let mut out = Vec::new();
    let mut rest = data.as_str();
    while let Some(start) = rest.find("~~~~ example ") {
        let after = &rest[start + 13..];
        let (number, after) = after.split_once('\n').unwrap();
        let (markdown, after) = after.split_once("~~~~ html\n").unwrap();
        let (html, after) = after.split_once("~~~~ end\n").unwrap();
        out.push(Example {
            number: number.parse().unwrap(),
            markdown: markdown.to_owned(),
            html: html.to_owned(),
        });
        rest = after;
    }
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn decode(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let entity = rest.find(';').and_then(|end| {
            let name = &rest[1..end];
            let c = match name {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "nbsp" => '\u{a0}',
                "copy" => '©',
                "ouml" => 'ö',
                "auml" => 'ä',
                _ => {
                    let number = name.strip_prefix('#')?;
                    let code = match number.strip_prefix(['x', 'X']) {
                        Some(hex) if (1..=6).contains(&hex.len()) => u32::from_str_radix(hex, 16).ok()?,
                        None if (1..=7).contains(&number.len()) => number.parse().ok()?,
                        _ => return None,
                    };
                    char::from_u32(code).filter(|&c| c != '\0').unwrap_or('\u{fffd}')
                }
            };
            Some((c, end + 1))
        });
        if let Some((c, len)) = entity {
            out.push(c);
            rest = &rest[len..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match chars.peek() {
            Some(&next) if c == '\\' && next.is_ascii_punctuation() => {
                // An escaped `&` must not start an entity.
                out.push(if next == '&' { '\u{1}' } else { next });
                chars.next();
            }
            _ => out.push(c),
        }
    }
    out
}

/// The text of a text node as HTML: escapes and entities as chars, no spaces around a line
/// break.
fn text_html(source: &str) -> String {
    let lines: Vec<&str> = source.split('\n').collect();
    let last = lines.len() - 1;
    let lines: Vec<&str> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let line = if i > 0 { line.trim_start() } else { line };
            if i < last { line.trim_end_matches(' ') } else { line }
        })
        .collect();
    escape(&decode(&unescape(&lines.join("\n")))).replace('\u{1}', "&amp;")
}

fn plain(text: &str, nodes: &[Node], out: &mut String) {
    for node in nodes {
        match &node.kind {
            Kind::Text => out.push_str(&text_html(&text[node.source.clone()])),
            Kind::Code => out.push_str(&escape(&text[node.content.clone()].replace('\n', " "))),
            Kind::HardBreak => out.push('\n'),
            Kind::Html => {}
            _ => plain(text, &node.children, out),
        }
    }
}

fn render(text: &str, nodes: &[Node], out: &mut String) {
    for node in nodes {
        let tag = match &node.kind {
            Kind::Text => {
                out.push_str(&text_html(&text[node.source.clone()]));
                continue;
            }
            Kind::Link(_) if text.as_bytes()[node.source.start] == b'<' => {
                let _ = write!(out, "<a>{}</a>", escape(&text[node.content.clone()]));
                continue;
            }
            Kind::Code => {
                let code = text[node.content.clone()].replace('\n', " ");
                let _ = write!(out, "<code>{}</code>", escape(&code));
                continue;
            }
            Kind::HardBreak => {
                out.push_str("<br />\n");
                continue;
            }
            Kind::Html => {
                out.push_str(&text[node.source.clone()]);
                continue;
            }
            Kind::Image(_) => {
                let mut alt = String::new();
                plain(text, &node.children, &mut alt);
                let _ = write!(out, "<img alt=\"{alt}\" />");
                continue;
            }
            Kind::Link(_) => "a",
            Kind::Emphasis => "em",
            Kind::Strong => "strong",
            Kind::Strikethrough => "del",
            Kind::Highlight(_) => "mark",
            Kind::Underline | Kind::Color(_) => "",
        };
        // A style from an HTML tag stays as its tags.
        let raw = text.as_bytes()[node.source.start] == b'<' && !matches!(node.kind, Kind::Link(_));
        if raw {
            out.push_str(&text[node.source.start..node.content.start]);
        } else {
            let _ = write!(out, "<{tag}>");
        }
        render(text, &node.children, out);
        if raw {
            out.push_str(&text[node.content.end..node.source.end]);
        } else {
            let _ = write!(out, "</{tag}>");
        }
    }
}

/// `None` for a block that is not a paragraph or a heading.
fn html(text: &str) -> Option<String> {
    let body = Body::parse(text);
    let mut out = String::new();
    for block in body.blocks() {
        let tag = match block.kind {
            BlockKind::Paragraph => "p".to_owned(),
            BlockKind::Heading { level } => format!("h{level}"),
            _ => return None,
        };
        let leaf = body
            .leaves()
            .iter()
            .find(|leaf| block.range.start <= leaf.range().start && leaf.range().end <= block.range.end);
        let _ = write!(out, "<{tag}>");
        if let Some(leaf) = leaf {
            render(text, &body.inline(text, leaf), &mut out);
        }
        let _ = writeln!(out, "</{tag}>");
    }
    Some(out)
}

/// The HTML with each `<a …>` as `<a>`, and each `<img …>` with only its alt text.
fn without_link_attributes(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    loop {
        let a = rest.find("<a ");
        let img = rest.find("<img ");
        let Some(at) = [a, img].into_iter().flatten().min() else {
            break;
        };
        out.push_str(&rest[..at]);
        let end = at + rest[at..].find('>').unwrap() + 1;
        let tag = &rest[at..end];
        if tag.starts_with("<a ") {
            out.push_str("<a>");
        } else {
            let alt = tag
                .split_once("alt=\"")
                .map_or("", |(_, a)| a.split('"').next().unwrap());
            let _ = write!(out, "<img alt=\"{alt}\" />");
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

#[test]
fn inline_examples_of_the_spec() {
    let mut checked = 0;
    let mut failed = Vec::new();
    for example in examples() {
        if NAMED_ENTITIES.contains(&example.number) {
            continue;
        }
        let Some(got) = html(&example.markdown) else {
            continue;
        };
        let want = without_link_attributes(&example.html);
        if without_link_attributes(&got) == want {
            checked += 1;
        } else {
            failed.push(format!(
                "example {}\n{}--- want\n{want}--- got\n{got}",
                example.number, example.markdown
            ));
        }
    }
    assert!(failed.is_empty(), "{} failed:\n\n{}", failed.len(), failed.join("\n"));
    assert!(checked > 280, "only {checked} examples checked");
}
