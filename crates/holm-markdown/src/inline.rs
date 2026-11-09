//! Inline text: the tree of styled text in each leaf block, its parser, and the format keys.
//!
//! A leaf block is a paragraph, a heading or a table cell. The text of a tight list item is one
//! too. Its text can run over several lines. Quote markers or an indent can stand between
//! them. The tree skips them, but a node that crosses a line holds them in its range.

mod parse;
mod restyle;
mod scan;
mod tag;

use std::borrow::Cow;
use std::ops::Range;

pub(crate) use parse::{Defs, Scratch};
pub use restyle::{Edit, Format, restyle};

use crate::{Color, Target};

/// One node of the inline tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// What the node is.
    pub kind: Kind,
    /// The whole source of the node, with its markers.
    pub source: Range<usize>,
    /// The source between the markers. For a node with no markers it is the same as `source`.
    pub content: Range<usize>,
    /// The nodes inside a style, a link or an image, in order.
    pub children: Vec<Node>,
}

/// The kind of a [`Node`]. The marker spelling is the source outside the content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Plain text. Backslash escapes and entities stay as written.
    Text,
    /// `*text*`, `_text_`, `<i>` or `<em>`.
    Emphasis,
    /// `**text**`, `__text__`, `<b>` or `<strong>`.
    Strong,
    /// `~~text~~`, `~text~`, `<s>` or `<del>`.
    Strikethrough,
    /// `==text==` or `<mark>` with `None`, `<mark style="background: …">` with its color.
    Highlight(Option<Color>),
    /// `<u>text</u>`
    Underline,
    /// `<span style="color: …">text</span>`
    Color(Color),
    /// `` `code` ``. The content has no ticks, and no space that the code rules remove.
    Code,
    /// A link or an autolink. Its content is the link text.
    Link(Box<Destination>),
    /// An image or an embedded file. Its content is the alt text.
    Image(Box<Destination>),
    /// A line break: a backslash or two spaces at the end of a line.
    HardBreak,
    /// Raw HTML that is not a Holm style tag, or a tag with no partner.
    Html,
}

/// Where a link or an image points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Destination {
    /// The target. It is `None` when the destination is empty.
    pub target: Option<Target>,
    /// The destination as written, with its `< >`. It is `None` for a reference link.
    pub range: Option<Range<usize>>,
}

/// A leaf block: where its text lies in the source. [`crate::Body::inline`] gives its tree.
#[derive(Clone, Debug)]
pub struct Leaf {
    /// The first line of the text. It has no line break, quote markers or indent.
    first: Range<usize>,
    /// All lines, when there is more than one. A leaf of one line needs no list.
    lines: Vec<Range<usize>>,
    /// Where each of `lines` starts in the joined text.
    starts: Vec<usize>,
    table: bool,
}

impl Leaf {
    /// The leaf of the text in `segments`, and its tree. With `plain`, it also gives the plain
    /// text for a heading link. There is at least one segment, and they are in order.
    pub(crate) fn new(
        text: &str,
        mut segments: Vec<Range<usize>>,
        table: bool,
        defs: &Defs,
        plain: bool,
        scratch: &mut Scratch,
    ) -> (Self, Vec<Node>, Option<String>) {
        let first = segments.first().cloned().unwrap_or_default();
        let mut starts = Vec::new();
        if segments.len() > 1 {
            let mut at = 0;
            for segment in &segments {
                starts.push(at);
                at += segment.len() + 1;
            }
        } else {
            segments = Vec::new();
        }
        let leaf = Self {
            first,
            lines: segments,
            starts,
            table,
        };
        let joined = leaf.joined(text);
        let nodes = parse::parse_with(&joined, defs, scratch);
        let plain = plain.then(|| {
            let mut out = String::new();
            plain_text(&joined, &nodes, &mut out);
            out.trim().to_owned()
        });
        let nodes = leaf.to_source_nodes(nodes);
        (leaf, nodes, plain)
    }

    /// The inline tree in source offsets.
    pub(crate) fn tree(&self, text: &str, defs: &Defs) -> Vec<Node> {
        let nodes = parse::parse(&self.joined(text), defs);
        self.to_source_nodes(nodes)
    }

    fn to_source_nodes(&self, mut nodes: Vec<Node>) -> Vec<Node> {
        if self.lines.len() > 1 {
            for node in &mut nodes {
                self.map_node(node);
            }
        } else if self.first.start != 0 {
            for node in &mut nodes {
                shift_node(node, self.first.start);
            }
        }
        nodes
    }

    fn segments(&self) -> &[Range<usize>] {
        if self.lines.is_empty() {
            std::slice::from_ref(&self.first)
        } else {
            &self.lines
        }
    }

    /// The source from the start of the first line to the end of the last one.
    pub fn range(&self) -> Range<usize> {
        let segments = self.segments();
        let first = segments.first().map_or(0, |s| s.start);
        let last = segments.last().map_or(first, |s| s.end);
        first..last
    }

    /// Whether the leaf is a table cell.
    pub(crate) fn is_table_cell(&self) -> bool {
        self.table
    }

    /// The text of the lines joined with `\n`. The parser and the format keys work on it.
    pub(crate) fn joined<'a>(&self, text: &'a str) -> Cow<'a, str> {
        if self.lines.is_empty() {
            return Cow::Borrowed(&text[self.first.clone()]);
        }
        let mut joined = String::with_capacity(self.range().len());
        for (i, segment) in self.lines.iter().enumerate() {
            if i > 0 {
                joined.push('\n');
            }
            joined.push_str(&text[segment.clone()]);
        }
        Cow::Owned(joined)
    }

    /// The source between line `i` and line `i + 1`, like `\n> `.
    pub(crate) fn gap<'a>(&self, text: &'a str, i: usize) -> &'a str {
        &text[self.lines[i].end..self.lines[i + 1].start]
    }

    /// The source offset of an offset in the joined text.
    pub(crate) fn to_source(&self, joined: usize) -> usize {
        if self.lines.is_empty() {
            return self.first.start + joined.min(self.first.len());
        }
        let i = self
            .starts
            .partition_point(|&s| s <= joined)
            .saturating_sub(1);
        self.lines[i].start + (joined - self.starts[i]).min(self.lines[i].len())
    }

    /// The offset in the joined text of a source offset. An offset between two lines lands on
    /// the line break.
    pub(crate) fn to_joined(&self, source: usize) -> usize {
        if self.lines.is_empty() {
            return source.clamp(self.first.start, self.first.end) - self.first.start;
        }
        let i = self.lines.partition_point(|s| s.start <= source);
        let Some(i) = i.checked_sub(1) else {
            return 0;
        };
        let segment = &self.lines[i];
        self.starts[i] + (source.min(segment.end) - segment.start)
    }

    fn map_node(&self, node: &mut Node) {
        node.source = self.to_source(node.source.start)..self.to_source(node.source.end);
        node.content = self.to_source(node.content.start)..self.to_source(node.content.end);
        if let Kind::Link(dest) | Kind::Image(dest) = &mut node.kind
            && let Some(range) = &mut dest.range
        {
            *range = self.to_source(range.start)..self.to_source(range.end);
        }
        for child in &mut node.children {
            self.map_node(child);
        }
    }
}

fn shift_node(node: &mut Node, by: usize) {
    let shift = |r: &mut Range<usize>| *r = r.start + by..r.end + by;
    shift(&mut node.source);
    shift(&mut node.content);
    if let Kind::Link(dest) | Kind::Image(dest) = &mut node.kind
        && let Some(range) = &mut dest.range
    {
        shift(range);
    }
    for child in &mut node.children {
        shift_node(child, by);
    }
}

/// The text a reader sees: no markers, escapes, HTML or images. Line breaks become spaces.
fn plain_text(joined: &str, nodes: &[Node], out: &mut String) {
    for node in nodes {
        match &node.kind {
            Kind::Text => {
                let text = scan::unescape(&joined[node.source.clone()]);
                out.push_str(&decode_entities(&text).replace('\n', " "));
            }
            Kind::Code => out.push_str(&joined[node.content.clone()].replace('\n', " ")),
            Kind::HardBreak => out.push(' '),
            Kind::Html | Kind::Image(_) => {}
            _ => plain_text(joined, &node.children, out),
        }
    }
}

/// The text with the common named entities and all number entities as chars.
fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let decoded = rest.find(';').and_then(|end| {
            let name = &rest[1..end];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                _ => {
                    let number = name.strip_prefix('#')?;
                    let code = match number.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => number.parse().ok()?,
                    };
                    Some(
                        char::from_u32(code)
                            .filter(|&c| c != '\0')
                            .unwrap_or('\u{fffd}'),
                    )
                }
            }?;
            Some((c, end + 1))
        });
        if let Some((c, len)) = decoded {
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
