//! The note body as blocks, leaves, styled spans, hidden syntax, links and headings.

pub(crate) mod link;

use std::ops::Range;

use pulldown_cmark::{
    Alignment as CmarkAlignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Options,
    Parser, Tag, TagEnd,
};

pub use link::Target;

use crate::inline::{Defs, Destination, Kind, Leaf, Node, Scratch};

/// A parsed note body, for live preview and for the index.
///
/// Every range is a byte range in the parsed text, and lies on char boundaries.
///
/// ```
/// use holm_markdown::{Body, Style};
///
/// let body = Body::parse("# Plans\n\nSome ==big== ideas.\n");
/// assert_eq!(body.headings()[0].text, "Plans");
/// assert_eq!(body.spans()[0].style, Style::Highlight(None));
/// assert_eq!(body.hidden(), &[0..2, 14..16, 19..21]);
/// ```
#[derive(Clone, Debug, Default)]
pub struct Body {
    blocks: Vec<Block>,
    spans: Vec<Span>,
    hidden: Vec<Range<usize>>,
    links: Vec<Link>,
    headings: Vec<Heading>,
    leaves: Vec<Leaf>,
    defs: Defs,
}

/// One block of the body, with the blocks inside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// The source of the block, without the line break at its end.
    pub range: Range<usize>,
    /// What the block is.
    pub kind: BlockKind,
    /// The blocks inside this one, like the items of a list.
    pub children: Vec<Block>,
}

/// The kind of a [`Block`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockKind {
    /// A paragraph.
    Paragraph,
    /// A heading of level 1 to 6.
    Heading {
        /// The level: 1 for `#`, 6 for `######`.
        level: u8,
    },
    /// A block quote that is not a callout.
    Quote,
    /// A GitHub callout like `> [!note]`.
    Callout(CalloutKind),
    /// A list. Its children are [`BlockKind::Item`] blocks.
    List {
        /// The first number of an ordered list. `None` for a bullet list.
        start: Option<u64>,
    },
    /// One list item.
    Item {
        /// The `[ ]` or `[x]` of a task list item.
        task: Option<Task>,
    },
    /// A fenced or indented code block.
    Code {
        /// The first word after the opening fence.
        language: Option<String>,
        /// Whether it has fences. An indented code block has none.
        fenced: bool,
    },
    /// A table. Its children are [`BlockKind::TableRow`] blocks.
    Table {
        /// The alignment of each column.
        alignments: Vec<Alignment>,
    },
    /// A table row. Its children are [`BlockKind::TableCell`] blocks.
    TableRow {
        /// Whether it is the head row.
        head: bool,
    },
    /// A table cell. Its range has the spaces around the cell text.
    TableCell,
    /// A block of raw HTML.
    Html,
    /// A thematic break like `---`.
    Rule,
}

/// The kind of a GitHub callout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalloutKind {
    /// `[!note]`
    Note,
    /// `[!tip]`
    Tip,
    /// `[!important]`
    Important,
    /// `[!warning]`
    Warning,
    /// `[!caution]`
    Caution,
}

/// The task marker of a list item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    /// Whether it is `[x]`.
    pub checked: bool,
    /// The source of the marker, like `[ ]`.
    pub range: Range<usize>,
}

/// The alignment of a table column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    /// No colon in the delimiter row.
    None,
    /// `:--`
    Left,
    /// `:-:`
    Center,
    /// `--:`
    Right,
}

/// A range of text with one style. Spans can overlap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    /// The styled text, without its syntax.
    pub range: Range<usize>,
    /// The style.
    pub style: Style,
}

/// The style of a [`Span`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// `*text*`
    Emphasis,
    /// `**text**`
    Strong,
    /// `~~text~~`
    Strikethrough,
    /// `` `text` ``
    Code,
    /// `<u>text</u>`
    Underline,
    /// `==text==` or `<mark>` with `None`, `<mark style="background: ...">` with its color.
    Highlight(Option<Color>),
    /// `<span style="color: ...">`
    Color(Color),
    /// The text of a link.
    Link,
}

/// A color from a `#rgb` or `#rrggbb` hex value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
}

/// A link or an embedded file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// The whole link source, like `[text](target)`.
    pub range: Range<usize>,
    /// Where the link points.
    pub target: Target,
    /// The target as written, with its `< >`. It is `None` for a reference link,
    /// because the target is in its definition line.
    pub target_range: Option<Range<usize>>,
    /// Whether it is `![..](..)`: an image or a file card.
    pub embedded: bool,
}

/// A heading, for heading links.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    /// The level: 1 for `#`, 6 for `######`.
    pub level: u8,
    /// The plain text. A heading link must match it exactly.
    pub text: String,
    /// The source of the heading, without the line break at its end.
    pub range: Range<usize>,
}

impl Body {
    /// Parses a note body: the text after the frontmatter. It never fails.
    pub fn parse(text: &str) -> Self {
        let options = Options::ENABLE_TABLES
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_GFM;
        let mut walker = Walker::new(text);
        // The parser reads the link definitions before the first event.
        let events = Parser::new_ext(text, options).into_offset_iter();
        for (label, def) in events.reference_definitions().iter() {
            walker.body.defs.insert(label, def.dest.to_string());
        }
        for (event, range) in events {
            walker.event(event, range);
        }
        walker.finish()
    }

    /// The top level blocks, in order.
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// The leaf blocks, in order.
    pub fn leaves(&self) -> &[Leaf] {
        &self.leaves
    }

    /// The inline tree of `leaf`, in source offsets. `text` is the parsed text.
    ///
    /// The body keeps no trees, because a full parse runs after each edit. Many small trees are
    /// slow to free. This call parses the leaf again.
    pub fn inline(&self, text: &str, leaf: &Leaf) -> Vec<Node> {
        leaf.tree(text, &self.defs)
    }

    /// The styled spans, sorted by start.
    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    /// The syntax that live preview hides away from the cursor line, sorted, with no overlaps.
    ///
    /// Tables, list markers and task markers are not in it. The editor draws them from the blocks.
    pub fn hidden(&self) -> &[Range<usize>] {
        &self.hidden
    }

    /// The links and embedded files, sorted by start. A link with an empty target is not in it.
    pub fn links(&self) -> &[Link] {
        &self.links
    }

    /// The headings, in order.
    pub fn headings(&self) -> &[Heading] {
        &self.headings
    }

    pub(crate) fn defs(&self) -> &Defs {
        &self.defs
    }
}

/// An open block tag while the walker reads its children.
struct Frame {
    kind: Option<BlockKind>,
    range: Range<usize>,
    /// Start of the first non-empty child event.
    first: Option<usize>,
    /// End of the last non-empty child event.
    last: Option<usize>,
    children: Vec<Block>,
}

/// The inline events of a leaf block, while the walker reads them.
struct OpenLeaf {
    /// How many frames are open, with the leaf's own frame. For the text of a tight list item,
    /// the item's frame.
    depth: usize,
    /// Whether it is the text of a tight list item. Such text has no frame of its own.
    item: bool,
    table: bool,
    heading: Option<usize>,
    first: usize,
    last: usize,
    /// Where the inline events start, except line breaks.
    starts: Vec<usize>,
    /// Code spans and HTML tags that hold a line break.
    long: Vec<Range<usize>>,
}

struct Walker<'a> {
    text: &'a str,
    frames: Vec<Frame>,
    leaf: Option<OpenLeaf>,
    /// The `starts` buffer of the last leaf, kept for the next one.
    starts: Vec<usize>,
    /// The lines of the last leaf, kept for the next one.
    segments: Vec<Range<usize>>,
    scratch: Scratch,
    /// The inline syntax to hide, in order. The block syntax goes in the body's list.
    inline_hidden: Vec<Range<usize>>,
    body: Body,
}

impl<'a> Walker<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            frames: Vec::new(),
            leaf: None,
            starts: Vec::new(),
            segments: Vec::new(),
            scratch: Scratch::default(),
            inline_hidden: Vec::new(),
            body: Body::default(),
        }
    }

    fn event(&mut self, event: Event<'_>, range: Range<usize>) {
        match event {
            Event::Start(tag) if is_inline_tag(&tag) => self.inline(&range, true),
            Event::End(end) if is_inline_end(end) => self.touch(&range),
            Event::Start(tag) => self.start(tag, range),
            Event::End(_) => self.end(),
            Event::TaskListMarker(checked) => self.task(checked, range),
            Event::Rule => {
                self.touch(&range);
                let block = Block {
                    range: self.trim(&range),
                    kind: BlockKind::Rule,
                    children: Vec::new(),
                };
                self.push_block(block);
            }
            Event::SoftBreak | Event::HardBreak => self.inline(&range, false),
            Event::Html(_) => self.touch(&range),
            Event::Code(_) | Event::InlineHtml(_) => {
                self.inline(&range, true);
                if let Some(leaf) = &mut self.leaf
                    && self.text[range.clone()].contains(['\n', '\r'])
                {
                    leaf.long.push(range);
                }
            }
            _ => self.inline(&range, true),
        }
    }

    /// Notes an inline event. `start` says whether its start can begin a line of the leaf.
    fn inline(&mut self, range: &Range<usize>, start: bool) {
        self.touch(range);
        if range.is_empty() {
            return;
        }
        let in_item = matches!(
            self.frames.last(),
            Some(Frame {
                kind: Some(BlockKind::Item { .. }),
                ..
            })
        );
        if self.leaf.is_none() && in_item {
            self.leaf = Some(OpenLeaf {
                depth: self.frames.len(),
                item: true,
                table: false,
                heading: None,
                first: range.start,
                last: range.end,
                starts: std::mem::take(&mut self.starts),
                long: Vec::new(),
            });
        }
        let Some(leaf) = &mut self.leaf else {
            return;
        };
        leaf.first = leaf.first.min(range.start);
        leaf.last = leaf.last.max(range.end);
        if start {
            leaf.starts.push(range.start);
        }
    }

    fn start(&mut self, tag: Tag<'_>, range: Range<usize>) {
        self.touch(&range);
        if self.leaf.as_ref().is_some_and(|leaf| leaf.item) {
            self.close_leaf();
        }
        let kind = match tag {
            Tag::Paragraph => Some(BlockKind::Paragraph),
            Tag::Heading { level, .. } => {
                self.body.headings.push(Heading {
                    level: heading_level(level),
                    text: String::new(),
                    range: self.trim(&range),
                });
                Some(BlockKind::Heading {
                    level: heading_level(level),
                })
            }
            Tag::BlockQuote(kind) => Some(kind.map_or(BlockKind::Quote, |kind| {
                BlockKind::Callout(callout_kind(kind))
            })),
            Tag::CodeBlock(kind) => Some(match kind {
                CodeBlockKind::Fenced(info) => BlockKind::Code {
                    language: info.split_whitespace().next().map(str::to_owned),
                    fenced: true,
                },
                CodeBlockKind::Indented => BlockKind::Code {
                    language: None,
                    fenced: false,
                },
            }),
            Tag::HtmlBlock => Some(BlockKind::Html),
            Tag::List(start) => Some(BlockKind::List { start }),
            Tag::Item => Some(BlockKind::Item { task: None }),
            Tag::Table(alignments) => Some(BlockKind::Table {
                alignments: alignments.into_iter().map(alignment).collect(),
            }),
            Tag::TableHead => Some(BlockKind::TableRow { head: true }),
            Tag::TableRow => Some(BlockKind::TableRow { head: false }),
            Tag::TableCell => Some(BlockKind::TableCell),
            _ => None,
        };
        let leaf = matches!(
            kind,
            Some(BlockKind::Paragraph | BlockKind::Heading { .. } | BlockKind::TableCell)
        );
        let table = matches!(kind, Some(BlockKind::TableCell));
        let heading = matches!(kind, Some(BlockKind::Heading { .. }));
        self.frames.push(Frame {
            kind,
            range,
            first: None,
            last: None,
            children: Vec::new(),
        });
        if leaf {
            self.leaf = Some(OpenLeaf {
                depth: self.frames.len(),
                item: false,
                table,
                heading: heading.then(|| self.body.headings.len() - 1),
                first: usize::MAX,
                last: 0,
                starts: std::mem::take(&mut self.starts),
                long: Vec::new(),
            });
        }
    }

    fn end(&mut self) {
        if self
            .leaf
            .as_ref()
            .is_some_and(|leaf| leaf.depth == self.frames.len())
        {
            self.close_leaf();
        }
        let Some(frame) = self.frames.pop() else {
            return;
        };
        let Some(kind) = frame.kind.clone() else {
            return;
        };
        let range = self.trim(&frame.range);
        match &kind {
            BlockKind::Heading { .. } => self.hide_heading_syntax(&range, &frame),
            BlockKind::Code { fenced: true, .. } => self.hide_fences(&range, frame.last),
            BlockKind::Quote => self.hide_quote_markers(&range),
            BlockKind::Callout(_) => {
                self.hide(range.start..self.line_end(range.start, range.end));
                self.hide_quote_markers(&range);
            }
            _ => {}
        }
        let block = Block {
            range,
            kind,
            children: frame.children,
        };
        self.push_block(block);
    }

    fn close_leaf(&mut self) {
        let Some(mut leaf) = self.leaf.take() else {
            return;
        };
        if leaf.first < leaf.last {
            self.segments.clear();
            segments(
                self.text,
                leaf.first..leaf.last,
                &mut leaf.starts,
                &leaf.long,
                &mut self.segments,
            );
            self.parse_leaf(
                Leaf::new(self.text, &self.segments, leaf.table),
                leaf.heading,
            );
        }
        leaf.starts.clear();
        self.starts = leaf.starts;
    }

    /// Adds the inline syntax of `leaf` to the body. A heading also gets its plain text.
    fn parse_leaf(&mut self, leaf: Leaf, heading: Option<usize>) {
        let text = self.text;
        if let Some(i) = heading {
            let mut plain = String::new();
            let nodes = leaf.parse(text, &self.body.defs, &mut self.scratch, Some(&mut plain));
            self.body.headings[i].text = plain;
            self.collect(nodes);
        } else if !leaf.is_plain(text) {
            let nodes = leaf.parse(text, &self.body.defs, &mut self.scratch, None);
            self.collect(nodes);
        }
        self.body.leaves.push(leaf);
    }

    fn task(&mut self, checked: bool, range: Range<usize>) {
        self.touch(&range);
        let item = self
            .frames
            .iter_mut()
            .rev()
            .find(|frame| matches!(frame.kind, Some(BlockKind::Item { .. })));
        if let Some(frame) = item {
            frame.kind = Some(BlockKind::Item {
                task: Some(Task { checked, range }),
            });
        }
    }

    /// Hides `# ` before the text and the closing `#`s, or the `===` line of a setext heading.
    fn hide_heading_syntax(&mut self, range: &Range<usize>, frame: &Frame) {
        let (Some(first), Some(last)) = (frame.first, frame.last) else {
            return;
        };
        self.hide(range.start..first);
        if last < range.end {
            let after = &self.text[last..range.end];
            let start = after.rfind('\n').map_or(last, |i| last + i + 1);
            self.hide(start..range.end);
        }
    }

    /// Hides the opening fence line and the closing fence line.
    fn hide_fences(&mut self, range: &Range<usize>, content_end: Option<usize>) {
        let open_end = self.line_end(range.start, range.end);
        self.hide(range.start..open_end);
        let close_start = content_end.unwrap_or_else(|| {
            self.text[open_end..range.end]
                .find('\n')
                .map_or(range.end, |i| open_end + i + 1)
        });
        if close_start < range.end {
            self.hide(close_start..range.end);
        }
    }

    /// Hides the `>` of this quote, and one space after it, on each line.
    fn hide_quote_markers(&mut self, range: &Range<usize>) {
        let depth = 1 + self
            .frames
            .iter()
            .filter(|frame| matches!(frame.kind, Some(BlockKind::Quote | BlockKind::Callout(_))))
            .count();
        let bytes = self.text.as_bytes();
        let mut line_start = range.start;
        // The first line starts at this quote's own `>`.
        let mut want = 1;
        loop {
            let mut seen = 0;
            for i in line_start..range.end {
                match bytes[i] {
                    b'>' => {
                        seen += 1;
                        if seen == want {
                            let end = if bytes.get(i + 1) == Some(&b' ') {
                                i + 2
                            } else {
                                i + 1
                            };
                            self.hide(i..end.min(range.end));
                            break;
                        }
                    }
                    b' ' | b'\t' => {}
                    _ => break,
                }
            }
            match self.text[line_start..range.end].find('\n') {
                Some(i) => line_start += i + 1,
                None => break,
            }
            want = depth;
        }
    }

    fn finish(mut self) -> Body {
        while !self.frames.is_empty() {
            self.end();
        }
        let mut body = self.body;
        let mut blocks_hidden = std::mem::take(&mut body.hidden);
        if !blocks_hidden.is_sorted_by_key(|range| range.start) {
            blocks_hidden.sort_unstable_by_key(|range| range.start);
        }
        body.hidden = merge(blocks_hidden, self.inline_hidden);
        // The lists are nearly sorted already. A check is cheaper than a sort.
        if !body.spans.is_sorted_by_key(|span| span.range.start) {
            body.spans.sort_by_key(|span| span.range.start);
        }
        if !body.links.is_sorted_by_key(|link| link.range.start) {
            body.links.sort_by_key(|link| link.range.start);
        }
        if !body.hidden.is_sorted_by_key(|range| range.start) {
            body.hidden.sort_unstable_by_key(|range| range.start);
        }
        body.hidden.dedup_by(|next, last| {
            let touch = next.start <= last.end;
            if touch {
                last.end = last.end.max(next.end);
            }
            touch
        });
        body
    }

    /// Adds the spans, the hidden syntax and the links of an inline tree.
    fn collect(&mut self, nodes: Vec<Node>) {
        for node in nodes {
            let style = match node.kind {
                Kind::Text => {
                    self.hide_escapes(&node.source);
                    continue;
                }
                Kind::Html => continue,
                Kind::HardBreak => {
                    if self.text.as_bytes()[node.source.start] == b'\\' {
                        self.hide_inline(node.source.start..node.source.start + 1);
                    }
                    continue;
                }
                Kind::Image(dest) => {
                    self.hide_inline(node.source.clone());
                    self.push_link(node.source, *dest, true);
                    continue;
                }
                Kind::Link(dest) => {
                    self.push_link(node.source.clone(), *dest, false);
                    if node.content.is_empty() {
                        continue;
                    }
                    Style::Link
                }
                Kind::Code => Style::Code,
                Kind::Emphasis => Style::Emphasis,
                Kind::Strong => Style::Strong,
                Kind::Strikethrough => Style::Strikethrough,
                Kind::Highlight(color) => Style::Highlight(color),
                Kind::Underline => Style::Underline,
                Kind::Color(color) => Style::Color(color),
            };
            self.span(node.content.clone(), style);
            self.hide_inline(node.source.start..node.content.start);
            self.collect(node.children);
            self.hide_inline(node.content.end..node.source.end);
        }
    }

    fn hide_escapes(&mut self, range: &Range<usize>) {
        let bytes = self.text.as_bytes();
        let mut i = range.start;
        while i + 1 < range.end {
            if bytes[i] == b'\\' && bytes[i + 1].is_ascii_punctuation() {
                self.hide_inline(i..i + 1);
                i += 2;
            } else {
                i += 1;
            }
        }
    }

    fn push_link(&mut self, range: Range<usize>, dest: Destination, embedded: bool) {
        if let Some(target) = dest.target {
            self.body.links.push(Link {
                range,
                target,
                target_range: dest.range,
                embedded,
            });
        }
    }

    /// Notes a child event in the open frame.
    fn touch(&mut self, range: &Range<usize>) {
        if range.is_empty() {
            return;
        }
        if let Some(frame) = self.frames.last_mut() {
            frame.first.get_or_insert(range.start);
            frame.last = Some(range.end);
        }
    }

    fn push_block(&mut self, block: Block) {
        let frame = self
            .frames
            .iter_mut()
            .rev()
            .find(|frame| frame.kind.is_some());
        match frame {
            Some(frame) => frame.children.push(block),
            None => self.body.blocks.push(block),
        }
    }

    fn span(&mut self, range: Range<usize>, style: Style) {
        if !range.is_empty() {
            self.body.spans.push(Span { range, style });
        }
    }

    fn hide_inline(&mut self, range: Range<usize>) {
        if !range.is_empty() {
            self.inline_hidden.push(range);
        }
    }

    fn hide(&mut self, range: Range<usize>) {
        if !range.is_empty() {
            self.body.hidden.push(range);
        }
    }

    /// The range without the line breaks at its end.
    fn trim(&self, range: &Range<usize>) -> Range<usize> {
        let source = &self.text[range.clone()];
        let kept = source.trim_end_matches(['\n', '\r']).len();
        range.start..range.start + kept
    }

    /// The end of the line at `start`, before its line break, at most `end`.
    fn line_end(&self, start: usize, end: usize) -> usize {
        let line = &self.text[start..end];
        let len = line.find('\n').unwrap_or(line.len());
        start + line[..len].trim_end_matches('\r').len()
    }
}

/// Two sorted lists of ranges as one sorted list.
fn merge(a: Vec<Range<usize>>, b: Vec<Range<usize>>) -> Vec<Range<usize>> {
    let mut out = Vec::with_capacity(a.len() + b.len());
    let mut b = b.into_iter().peekable();
    for range in a {
        while let Some(next) = b.next_if(|next| next.start < range.start) {
            out.push(next);
        }
        out.push(range);
    }
    out.extend(b);
    out
}

fn is_inline_tag(tag: &Tag<'_>) -> bool {
    matches!(
        tag,
        Tag::Emphasis
            | Tag::Strong
            | Tag::Strikethrough
            | Tag::Superscript
            | Tag::Subscript
            | Tag::Link { .. }
            | Tag::Image { .. }
    )
}

fn is_inline_end(end: TagEnd) -> bool {
    matches!(
        end,
        TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::Link
            | TagEnd::Image
    )
}

fn is_break(b: u8) -> bool {
    matches!(b, b'\n' | b'\r')
}

/// Adds the source of each line of the leaf in `range` to `segments`. Line breaks, quote
/// markers and indents are not in it. The first inline event on a line shows where its text
/// starts. A line can start inside a `long` event. Then the quote markers and spaces at its
/// start are skipped.
fn segments(
    text: &str,
    range: Range<usize>,
    starts: &mut [usize],
    long: &[Range<usize>],
    segments: &mut Vec<Range<usize>>,
) {
    let (first, last) = (range.start, range.end);
    starts.sort_unstable();
    let b = text.as_bytes();
    // An escaped char starts its own text event, right after the backslash.
    let mut line = if first > 0 && b[first - 1] == b'\\' {
        first - 1
    } else {
        first
    };
    loop {
        let Some(brk) = b[line..last].iter().position(|&c| is_break(c)) else {
            segments.push(line..last);
            return;
        };
        let brk = line + brk;
        segments.push(line..brk);
        let next = if b[brk] == b'\r' && b.get(brk + 1) == Some(&b'\n') {
            brk + 2
        } else {
            brk + 1
        };
        let line_end = b[next..last]
            .iter()
            .position(|&c| is_break(c))
            .map_or(last, |i| next + i);
        let k = starts.partition_point(|&s| s < next);
        let inside = long.iter().any(|r| r.start < next && next < r.end);
        let mut start = match starts.get(k) {
            Some(&s) if s <= line_end && !inside => s,
            _ => {
                let prefix = b[next..line_end]
                    .iter()
                    .take_while(|&&c| matches!(c, b' ' | b'\t' | b'>'))
                    .count();
                next + prefix
            }
        };
        if start > next && b[start - 1] == b'\\' {
            start -= 1;
        }
        line = start;
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn callout_kind(kind: BlockQuoteKind) -> CalloutKind {
    match kind {
        BlockQuoteKind::Note => CalloutKind::Note,
        BlockQuoteKind::Tip => CalloutKind::Tip,
        BlockQuoteKind::Important => CalloutKind::Important,
        BlockQuoteKind::Warning => CalloutKind::Warning,
        BlockQuoteKind::Caution => CalloutKind::Caution,
    }
}

fn alignment(alignment: CmarkAlignment) -> Alignment {
    match alignment {
        CmarkAlignment::None => Alignment::None,
        CmarkAlignment::Left => Alignment::Left,
        CmarkAlignment::Center => Alignment::Center,
        CmarkAlignment::Right => Alignment::Right,
    }
}
