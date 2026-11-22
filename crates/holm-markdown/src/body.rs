pub(crate) mod link;

use std::ops::Range;

use pulldown_cmark::{
    Alignment as CmarkAlignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};

pub use link::{Target, escape_text, unescape_text};

use crate::inline::{Container, Defs, Destination, Kind, Leaf, Node, Scratch};

const OPTIONS: Options = Options::ENABLE_TABLES
    .union(Options::ENABLE_TASKLISTS)
    .union(Options::ENABLE_STRIKETHROUGH)
    .union(Options::ENABLE_GFM);

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
    /// The source of each link reference definition, sorted.
    def_ranges: Vec<Range<usize>>,
    text_len: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// The source of the block, without the line break at its end.
    pub range: Range<usize>,
    pub kind: BlockKind,
    pub children: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Paragraph,
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
    Item {
        /// The `[ ]` or `[x]` of a task list item.
        task: Option<Task>,
    },
    /// A fenced or indented code block.
    Code {
        /// The first word after the opening fence.
        language: Option<String>,
        /// An indented code block has no fences.
        fenced: bool,
    },
    /// A table. Its children are [`BlockKind::TableRow`] blocks.
    Table {
        alignments: Vec<Alignment>,
    },
    /// A table row. Its children are [`BlockKind::TableCell`] blocks.
    TableRow {
        head: bool,
    },
    /// A table cell. Its range has the spaces around the cell text.
    TableCell,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    pub state: TaskState,
    /// The source of the marker, like `[ ]`.
    pub range: Range<usize>,
}

/// `[ ]` or `[x]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    Unchecked,
    Checked,
}

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

/// Spans can overlap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    /// The styled text, without its syntax.
    pub range: Range<usize>,
    pub style: Style,
}

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
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    /// ```
    /// use holm_markdown::Color;
    ///
    /// assert_eq!(Color::from_hex("#c4553a"), Some(Color { r: 0xc4, g: 0x55, b: 0x3a }));
    /// assert_eq!(Color::from_hex("#fff"), Some(Color { r: 255, g: 255, b: 255 }));
    /// assert_eq!(Color::from_hex("c4553a"), None);
    /// ```
    pub fn from_hex(value: &str) -> Option<Self> {
        let hex = value.strip_prefix('#')?;
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let digit = |i: usize| u8::from_str_radix(&hex[i..=i], 16).ok();
        match hex.len() {
            3 => Some(Color {
                r: digit(0)? * 17,
                g: digit(1)? * 17,
                b: digit(2)? * 17,
            }),
            6 => Some(Color {
                r: digit(0)? * 16 + digit(1)?,
                g: digit(2)? * 16 + digit(3)?,
                b: digit(4)? * 16 + digit(5)?,
            }),
            _ => None,
        }
    }
}

/// Writes `#rrggbb`, the form that Holm stores.
impl std::fmt::Display for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

/// A link or an embedded file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// The whole link source, like `[text](target)`.
    pub range: Range<usize>,
    /// The link text between `[` and `]`, as written.
    pub text: Range<usize>,
    pub target: Target,
    /// The target as written, with its `< >`. It is `None` for a reference link, because the target is in its
    /// definition line.
    pub target_range: Option<Range<usize>>,
    pub kind: LinkKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    /// `[..](..)`.
    Plain,
    /// `![..](..)`: an image or a file chip.
    Embedded,
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
    /// `text` is the text after the frontmatter.
    pub fn parse(text: &str) -> Self {
        let events = Parser::new_ext(text, OPTIONS).into_offset_iter();
        // The parser reads the link definitions before the first event.
        let mut defs = Defs::default();
        let mut def_ranges = Vec::new();
        for (label, def) in events.reference_definitions().iter() {
            defs.insert(label, def.dest.to_string());
            def_ranges.push(def.span.clone());
        }
        def_ranges.sort_unstable_by_key(|range| range.start);
        let mut walker = Walker::new(text, defs);
        for (event, range) in events {
            walker.event(event, range);
        }
        let mut body = walker.finish();
        body.def_ranges = def_ranges;
        body
    }

    /// Parses `text` again after an edit, with the same result as [`Body::parse`].
    ///
    /// `text` is the text of this body with the bytes in `old` replaced by `new_len` bytes. Only the blocks near the
    /// edit are parsed again. An edit that can change blocks far away, like an open code fence or a link definition,
    /// gets a full parse. So does an edit that does not fit the old text.
    pub fn reparse(&mut self, text: &str, old: Range<usize>, new_len: usize) {
        if !self.reparse_part(text, old, new_len) {
            *self = Self::parse(text);
        }
    }

    /// Parses again only the top level blocks near the edit. Returns `false`, with the body unchanged, when that can
    /// give a result other than a full parse.
    ///
    /// The part starts after a blank line before the edit. There the parser is in the same state as at the start of a
    /// text. The part ends with the first block after the edit, and the line breaks after that block. When that block
    /// comes out the same, the parser is in the same state after it as before the edit.
    fn reparse_part(&mut self, text: &str, old: Range<usize>, new_len: usize) -> bool {
        let fits = old.start <= old.end
            && old.end <= self.text_len
            && self.text_len - old.len() + new_len == text.len()
            && text.is_char_boundary(old.start)
            && text.is_char_boundary(old.start + new_len);
        if !fits {
            return false;
        }
        let new_end = old.start + new_len;
        let moved = |at: usize| at + new_len - old.len();
        let blocks = &self.blocks;

        // Blocks before `old.start` keep the old text, so `text` has their gaps too.
        let mut first = blocks
            .partition_point(|block| block.range.end < old.start)
            .saturating_sub(1);
        while first > 0 && !is_blank_gap(&text[blocks[first - 1].range.end..blocks[first].range.start]) {
            first -= 1;
        }
        let start = if first == 0 {
            0
        } else {
            line_start(text, blocks[first].range.start)
        };

        // The first block whose whole line comes after the edit.
        let mut after = blocks.partition_point(|block| block.range.start <= old.end);
        while after < blocks.len() && line_start(text, moved(blocks[after].range.start)) <= new_end {
            after += 1;
        }
        let (old_stop, stop) = match blocks.get(after + 1) {
            Some(next) => {
                let stop = line_start(text, moved(next.range.start));
                (stop + old.len() - new_len, stop)
            }
            None => (self.text_len, text.len()),
        };
        let defs_after = self.def_ranges.partition_point(|range| range.start < old_stop);
        if self.def_ranges.partition_point(|range| range.start < start) != defs_after {
            return false;
        }

        let events = Parser::new_ext(&text[start..stop], OPTIONS).into_offset_iter();
        if events.reference_definitions().iter().next().is_some() {
            return false;
        }
        let mut walker = Walker::new(text, std::mem::take(&mut self.defs));
        for (event, range) in events {
            walker.event(event, range.start + start..range.end + start);
        }
        let mut new = walker.finish();
        self.defs = std::mem::take(&mut new.defs);

        let same_end = match self.blocks.get(after) {
            Some(block) => new.blocks.last().is_some_and(|end| {
                let mut block = block.clone();
                block.shift(&moved);
                *end == block
            }),
            None => true,
        };
        if !same_end {
            return false;
        }

        let blocks_end = (after + 1).min(self.blocks.len());
        splice(&mut self.blocks, first..blocks_end, new.blocks, &moved);
        replace(&mut self.spans, new.spans, start..old_stop, &moved);
        replace(&mut self.hidden, new.hidden, start..old_stop, &moved);
        replace(&mut self.links, new.links, start..old_stop, &moved);
        replace(&mut self.headings, new.headings, start..old_stop, &moved);
        replace(&mut self.leaves, new.leaves, start..old_stop, &moved);
        for range in &mut self.def_ranges[defs_after..] {
            range.shift(&moved);
        }
        self.text_len = text.len();
        true
    }

    /// The top level blocks, in order.
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn leaves(&self) -> &[Leaf] {
        &self.leaves
    }

    /// The inline tree of `leaf`, in source offsets. `text` is the parsed text.
    ///
    /// The body keeps no trees, because many small trees are slow to free. This call parses the leaf again.
    pub fn inline(&self, text: &str, leaf: &Leaf) -> Vec<Node> {
        leaf.tree(text, &self.defs)
    }

    /// Sorted by start.
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

    pub fn headings(&self) -> &[Heading] {
        &self.headings
    }

    pub(crate) fn defs(&self) -> &Defs {
        &self.defs
    }
}

/// A part of a body with source offsets, which an edit can move.
trait Moves {
    fn start(&self) -> usize;
    fn shift(&mut self, to: &impl Fn(usize) -> usize);
}

impl Moves for Range<usize> {
    fn start(&self) -> usize {
        self.start
    }

    fn shift(&mut self, to: &impl Fn(usize) -> usize) {
        *self = to(self.start)..to(self.end);
    }
}

impl Moves for Block {
    fn start(&self) -> usize {
        self.range.start
    }

    fn shift(&mut self, to: &impl Fn(usize) -> usize) {
        self.range.shift(to);
        if let BlockKind::Item { task: Some(task) } = &mut self.kind {
            task.range.shift(to);
        }
        for child in &mut self.children {
            child.shift(to);
        }
    }
}

impl Moves for Span {
    fn start(&self) -> usize {
        self.range.start
    }

    fn shift(&mut self, to: &impl Fn(usize) -> usize) {
        self.range.shift(to);
    }
}

impl Moves for Link {
    fn start(&self) -> usize {
        self.range.start
    }

    fn shift(&mut self, to: &impl Fn(usize) -> usize) {
        self.range.shift(to);
        self.text.shift(to);
        if let Some(range) = &mut self.target_range {
            range.shift(to);
        }
    }
}

impl Moves for Heading {
    fn start(&self) -> usize {
        self.range.start
    }

    fn shift(&mut self, to: &impl Fn(usize) -> usize) {
        self.range.shift(to);
    }
}

impl Moves for Leaf {
    fn start(&self) -> usize {
        self.range().start
    }

    fn shift(&mut self, to: &impl Fn(usize) -> usize) {
        Leaf::shift(self, to);
    }
}

/// Puts `new` in place of the items in `range` of a list sorted by start, and moves the items after them by `to`.
fn replace<T: Moves>(list: &mut Vec<T>, new: Vec<T>, range: Range<usize>, to: &impl Fn(usize) -> usize) {
    let lo = list.partition_point(|item| item.start() < range.start);
    let hi = list.partition_point(|item| item.start() < range.end);
    splice(list, lo..hi, new, to);
}

/// Puts `new` in place of `list[range]`, and moves the items after them by `to`.
fn splice<T: Moves>(list: &mut Vec<T>, range: Range<usize>, new: Vec<T>, to: &impl Fn(usize) -> usize) {
    let after = range.start + new.len();
    list.splice(range, new);
    for item in &mut list[after..] {
        item.shift(to);
    }
}

/// Whether the source between two top level blocks is only white space, with a blank line.
fn is_blank_gap(gap: &str) -> bool {
    let mut breaks = 0;
    let mut bytes = gap.bytes().peekable();
    while let Some(b) = bytes.next() {
        match b {
            b'\n' => breaks += 1,
            b'\r' if bytes.peek() != Some(&b'\n') => breaks += 1,
            b' ' | b'\t' | b'\r' => {}
            _ => return false,
        }
    }
    breaks >= 2
}

fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind(['\n', '\r']).map_or(0, |i| i + 1)
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
    /// How many frames are open, with the leaf's own frame. For the text of a tight list item, the item's frame.
    depth: usize,
    /// Whether it is the text of a tight list item. Such text has no frame of its own.
    item: bool,
    container: Container,
    heading: Option<usize>,
    first: usize,
    last: usize,
}

struct Walker<'a> {
    text: &'a str,
    frames: Vec<Frame>,
    leaf: Option<OpenLeaf>,
    /// The lines of the last leaf, kept for the next one.
    segments: Vec<Range<usize>>,
    scratch: Scratch,
    /// The inline syntax to hide, in order. The block syntax goes in the body's list.
    inline_hidden: Vec<Range<usize>>,
    /// The `-` line of the open heading. The walker reads this line as an empty nested item.
    empty_item: Option<Range<usize>>,
    body: Body,
}

impl<'a> Walker<'a> {
    fn new(text: &'a str, defs: Defs) -> Self {
        Self {
            text,
            frames: Vec::new(),
            leaf: None,
            segments: Vec::new(),
            scratch: Scratch::default(),
            inline_hidden: Vec::new(),
            empty_item: None,
            body: Body {
                defs,
                text_len: text.len(),
                ..Body::default()
            },
        }
    }

    fn event(&mut self, event: Event<'_>, range: Range<usize>) {
        match event {
            Event::Start(tag) if is_inline_tag(&tag) => self.inline(&range),
            Event::End(end) if is_inline_end(end) => self.touch(&range),
            Event::Start(Tag::Heading {
                level: HeadingLevel::H2,
                ..
            }) if self.read_as_empty_item(&range) => {}
            Event::End(TagEnd::Heading(_)) if self.empty_item.is_some() => self.end_empty_item(),
            Event::Start(tag) => self.start(tag, range),
            Event::End(_) => self.end(),
            Event::TaskListMarker(checked) => {
                let state = if checked {
                    TaskState::Checked
                } else {
                    TaskState::Unchecked
                };
                self.task(state, range);
            }
            Event::Rule => {
                self.touch(&range);
                let block = Block {
                    range: self.trim_line_breaks(&range),
                    kind: BlockKind::Rule,
                    children: Vec::new(),
                };
                self.push_block(block);
            }
            Event::Html(_) => self.touch(&range),
            _ => self.inline(&range),
        }
    }

    fn inline(&mut self, range: &Range<usize>) {
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
                container: Container::Block,
                heading: None,
                first: range.start,
                last: range.end,
            });
        }
        let Some(leaf) = &mut self.leaf else {
            return;
        };
        leaf.first = leaf.first.min(range.start);
        leaf.last = leaf.last.max(range.end);
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
                    range: self.trim_line_breaks(&range),
                });
                Some(BlockKind::Heading {
                    level: heading_level(level),
                })
            }
            Tag::BlockQuote(kind) => Some(kind.map_or(BlockKind::Quote, |kind| BlockKind::Callout(callout_kind(kind)))),
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
        let container = if matches!(kind, Some(BlockKind::TableCell)) {
            Container::TableCell
        } else {
            Container::Block
        };
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
                container,
                heading: heading.then(|| self.body.headings.len() - 1),
                first: usize::MAX,
                last: 0,
            });
        }
    }

    fn end(&mut self) {
        if self.leaf.as_ref().is_some_and(|leaf| leaf.depth == self.frames.len()) {
            self.close_leaf();
        }
        let Some(frame) = self.frames.pop() else {
            return;
        };
        let Some(kind) = frame.kind.clone() else {
            return;
        };
        let range = self.trim_line_breaks(&frame.range);
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
        let Some(leaf) = self.leaf.take() else {
            return;
        };
        if leaf.first < leaf.last {
            self.segments.clear();
            segments(self.text, leaf.first..leaf.last, &mut self.segments);
            self.parse_leaf(Leaf::new(self.text, &self.segments, leaf.container), leaf.heading);
        }
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

    /// Whether a heading is text with an empty nested item under it, like `- a\n  - `.
    ///
    /// An empty item cannot interrupt a paragraph. So `CommonMark` reads the `-` line as a setext underline. A new
    /// nested item has this form until its first char. The walker reads it as an item now, so the line keeps its form
    /// while the user types. The heading text becomes the text of the outer item.
    fn read_as_empty_item(&mut self, range: &Range<usize>) -> bool {
        let in_item = matches!(
            self.frames.iter().rev().find(|frame| frame.kind.is_some()),
            Some(Frame {
                kind: Some(BlockKind::Item { .. }),
                ..
            })
        );
        let range = self.trim_line_breaks(range);
        let Some(line_start) = self.text[range.clone()].rfind('\n') else {
            return false;
        };
        let line = &self.text[range.start + line_start + 1..range.end];
        let dash = line.trim_start_matches([' ', '\t', '>']);
        if !in_item || dash.trim_end_matches([' ', '\t', '\r']) != "-" {
            return false;
        }
        self.touch(&range);
        self.empty_item = Some(range.end - dash.len()..range.end);
        true
    }

    fn end_empty_item(&mut self) {
        let Some(range) = self.empty_item.take() else {
            return;
        };
        self.start(Tag::List(None), range.clone());
        self.start(Tag::Item, range);
        self.end();
        self.end();
    }

    fn task(&mut self, state: TaskState, range: Range<usize>) {
        self.touch(&range);
        let item = self
            .frames
            .iter_mut()
            .rev()
            .find(|frame| matches!(frame.kind, Some(BlockKind::Item { .. })));
        if let Some(frame) = item {
            frame.kind = Some(BlockKind::Item {
                task: Some(Task { state, range }),
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
                            let end = if bytes.get(i + 1) == Some(&b' ') { i + 2 } else { i + 1 };
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
                    self.push_link(node.source, node.content, *dest, LinkKind::Embedded);
                    continue;
                }
                Kind::Link(dest) => {
                    self.push_link(node.source.clone(), node.content.clone(), *dest, LinkKind::Plain);
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

    fn push_link(&mut self, range: Range<usize>, text: Range<usize>, dest: Destination, kind: LinkKind) {
        if let Some(target) = dest.target {
            self.body.links.push(Link {
                range,
                text,
                target,
                target_range: dest.range,
                kind,
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
        let frame = self.frames.iter_mut().rev().find(|frame| frame.kind.is_some());
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

    fn trim_line_breaks(&self, range: &Range<usize>) -> Range<usize> {
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

/// Adds the source of each line of the leaf in `range` to `segments`. Line breaks, quote markers and indents are not in
/// it. A paragraph line never starts with `>`, because that starts a quote. So the `>`, spaces and tabs at the start of
/// a line are never text.
fn segments(text: &str, range: Range<usize>, segments: &mut Vec<Range<usize>>) {
    let (first, last) = (range.start, range.end);
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
        let prefix = b[next..last]
            .iter()
            .take_while(|&&c| matches!(c, b' ' | b'\t' | b'>'))
            .count();
        line = next + prefix;
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
