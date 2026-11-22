//! A leaf block is a paragraph, a heading or a table cell. The text of a tight list item is one too. Its text can run
//! over several lines. Quote markers or an indent can stand between them. The tree skips them, but a node that crosses
//! a line holds them in its range.

mod html_tag;
mod parse;
mod restyle;
mod scan;

use std::borrow::Cow;
use std::ops::Range;

use parse::OffsetMap;
pub(crate) use parse::{Defs, Scratch};
pub use restyle::{Edit, Format, restyle};

use crate::{Color, JoinedOffset, OffsetRange, SourceOffset, Target};

/// `O` is the kind of offset. The parser gives offsets into the joined lines of a leaf, and a [`Leaf`] maps them to
/// [`SourceOffset`]s.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node<O = SourceOffset> {
    pub kind: Kind<O>,
    /// The whole source of the node, with its markers.
    pub source: Range<O>,
    /// The source between the markers. For a node with no markers it is the same as `source`.
    pub content: Range<O>,
    /// The nodes inside a style, a link or an image, in order.
    pub children: Vec<Node<O>>,
}

/// The marker spelling is the source outside the content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind<O = SourceOffset> {
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
    Link(Box<Destination<O>>),
    /// An image or an embedded file. Its content is the alt text.
    Image(Box<Destination<O>>),
    /// A line break: a backslash or two spaces at the end of a line.
    HardBreak,
    /// Raw HTML that is not a Holm style HTML tag, or one with no partner.
    Html,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Destination<O = SourceOffset> {
    /// `None` when the destination is empty.
    pub target: Option<Target>,
    /// The destination as written, with its `< >`. It is `None` for a reference link.
    pub range: Option<Range<O>>,
}

/// Where the text of a leaf block lies in the source. [`crate::Body::inline`] gives its tree.
#[derive(Clone, Debug)]
pub struct Leaf {
    /// The source from the start of the first line to the end of the last one.
    range: Range<SourceOffset>,
    /// All lines, when quote markers, an indent or `\r` stand between them. Without a list, the source of the leaf is
    /// its joined text.
    lines: Box<[Line]>,
    container: Container,
}

/// What holds a leaf. In a table cell, `\|` is a pipe in the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Container {
    Block,
    TableCell,
}

/// One line of a leaf with a gap between its lines.
#[derive(Clone, Debug)]
struct Line {
    source: Range<SourceOffset>,
    /// Where the line starts.
    joined: JoinedOffset,
}

impl Leaf {
    /// The leaf of the text in `segments`. There is at least one segment, and they are in order.
    pub(crate) fn new(text: &str, segments: &[Range<SourceOffset>], container: Container) -> Self {
        let start = segments.first().map_or(SourceOffset::default(), |s| s.start);
        let end = segments.last().map_or(start, |s| s.end);
        let one_piece = segments
            .windows(2)
            .all(|w| w[1].start == w[0].end + 1 && text.as_bytes()[w[0].end.get()] == b'\n');
        let mut lines = Vec::new();
        if !one_piece {
            lines.reserve_exact(segments.len());
            let mut at = JoinedOffset::default();
            for segment in segments {
                lines.push(Line {
                    source: segment.clone(),
                    joined: at,
                });
                at += segment.end - segment.start + 1;
            }
        }
        Self {
            range: start..end,
            lines: lines.into_boxed_slice(),
            container,
        }
    }

    /// No inline syntax, so the tree is one text node.
    pub(crate) fn is_plain(&self, text: &str) -> bool {
        parse::is_plain(&self.joined(text))
    }

    /// The inline tree, with the buffers of earlier parses. With `plain`, it also writes the plain text for a heading
    /// link.
    pub(crate) fn parse(
        &self,
        text: &str,
        defs: &Defs,
        scratch: &mut Scratch,
        plain: Option<&mut String>,
    ) -> Vec<Node> {
        let joined = self.joined(text);
        let nodes = parse::parse_with(&joined, defs, self, scratch);
        if let Some(out) = plain {
            plain_text(&joined, self, &nodes, out);
            let trimmed = out.trim();
            if trimmed.len() != out.len() {
                *out = trimmed.to_owned();
            }
        }
        nodes
    }

    pub(crate) fn tree(&self, text: &str, defs: &Defs) -> Vec<Node> {
        self.parse(text, defs, &mut Scratch::default(), None)
    }

    /// The source from the start of the first line to the end of the last one.
    pub fn range(&self) -> Range<SourceOffset> {
        self.range.clone()
    }

    /// Moves every offset by `to`.
    pub(crate) fn shift(&mut self, to: &impl Fn(SourceOffset) -> SourceOffset) {
        self.range = to(self.range.start)..to(self.range.end);
        for line in &mut self.lines {
            line.source = to(line.source.start)..to(line.source.end);
        }
    }

    pub(crate) fn container(&self) -> Container {
        self.container
    }

    /// The text of the lines joined with `\n`. The parser and the format keys work on it.
    pub(crate) fn joined<'a>(&self, text: &'a str) -> Cow<'a, str> {
        if self.lines.is_empty() {
            return Cow::Borrowed(&text[self.range.bytes()]);
        }
        let mut joined = String::with_capacity(self.range.end - self.range.start);
        for (i, line) in self.lines.iter().enumerate() {
            if i > 0 {
                joined.push('\n');
            }
            joined.push_str(&text[line.source.bytes()]);
        }
        Cow::Owned(joined)
    }

    /// The source between line `i` and line `i + 1`, like `\n> `.
    pub(crate) fn gap<'a>(&self, text: &'a str, i: usize) -> &'a str {
        if self.lines.is_empty() {
            return "\n";
        }
        &text[(self.lines[i].source.end..self.lines[i + 1].source.start).bytes()]
    }

    pub(crate) fn to_source(&self, joined: JoinedOffset) -> SourceOffset {
        if self.lines.is_empty() {
            return self.range.start + joined.get().min(self.range.end - self.range.start);
        }
        let i = self
            .lines
            .partition_point(|line| line.joined <= joined)
            .saturating_sub(1);
        let line = &self.lines[i];
        line.source.start + (joined - line.joined).min(line.source.end - line.source.start)
    }

    /// An offset between two lines lands on the line break.
    pub(crate) fn to_joined(&self, source: SourceOffset) -> JoinedOffset {
        if self.lines.is_empty() {
            return JoinedOffset::new(source.clamp(self.range.start, self.range.end) - self.range.start);
        }
        let i = self.lines.partition_point(|line| line.source.start <= source);
        let Some(i) = i.checked_sub(1) else {
            return JoinedOffset::default();
        };
        let line = &self.lines[i];
        line.joined + (source.min(line.source.end) - line.source.start)
    }
}

impl OffsetMap for Leaf {
    type Offset = SourceOffset;

    fn to_offset(&self, index: usize) -> SourceOffset {
        self.to_source(JoinedOffset::new(index))
    }

    fn to_index(&self, offset: SourceOffset) -> usize {
        self.to_joined(offset).get()
    }
}

/// The text a reader sees: no markers, escapes, HTML or images. Line breaks become spaces.
fn plain_text<M: OffsetMap>(joined: &str, map: &M, nodes: &[Node<M::Offset>], out: &mut String) {
    for node in nodes {
        match &node.kind {
            Kind::Text => {
                let text = scan::unescape(&joined[map.to_indexes(&node.source)]);
                out.push_str(&decode_entities(&text).replace('\n', " "));
            }
            Kind::Code => out.push_str(&joined[map.to_indexes(&node.content)].replace('\n', " ")),
            Kind::HardBreak => out.push(' '),
            Kind::Html | Kind::Image(_) => {}
            _ => plain_text(joined, map, &node.children, out),
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
                    Some(char::from_u32(code).filter(|&c| c != '\0').unwrap_or('\u{fffd}'))
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
