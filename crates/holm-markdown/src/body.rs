//! The note body as blocks, styled spans, hidden syntax, links and headings.

mod html;
mod link;

use std::ops::Range;

use pulldown_cmark::{
    Alignment as CmarkAlignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, LinkType,
    Options, Parser, Tag, TagEnd,
};

pub use link::Target;

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
        for (event, range) in Parser::new_ext(text, options).into_offset_iter() {
            walker.event(event, range);
        }
        walker.finish()
    }

    /// The top level blocks, in order.
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
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
}

enum FrameKind {
    Block(BlockKind),
    Inline(Inline),
    Other,
}

#[derive(Clone, Copy)]
enum Inline {
    Emphasis,
    Strong,
    Strikethrough,
    Link,
    Image,
}

/// An open tag while the walker reads its children.
struct Frame {
    kind: FrameKind,
    range: Range<usize>,
    /// Start of the first non-empty child event.
    first: Option<usize>,
    /// End of the last non-empty child event.
    last: Option<usize>,
    children: Vec<Block>,
    /// Where this frame's entries start in [`Walker::texts`] and [`Walker::tags`].
    texts_from: usize,
    tags_from: usize,
    /// The target of a link or image. It is in a box, so other frames stay small.
    link: Option<Box<(Option<Target>, LinkType)>>,
}

struct Walker<'a> {
    text: &'a str,
    frames: Vec<Frame>,
    /// How many images are open. Events inside an image are its alt text.
    image_depth: usize,
    in_heading: bool,
    /// Text events with the same bytes as the source, for `==` pairs.
    texts: Vec<Range<usize>>,
    /// Inline HTML tags, for `<u>`, `<span>` and `<mark>` pairs.
    tags: Vec<Range<usize>>,
    body: Body,
}

impl<'a> Walker<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            frames: Vec::new(),
            image_depth: 0,
            in_heading: false,
            texts: Vec::new(),
            tags: Vec::new(),
            body: Body::default(),
        }
    }

    fn event(&mut self, event: Event<'_>, range: Range<usize>) {
        if let (true, Event::Text(t) | Event::Code(t)) = (self.in_heading, &event)
            && let Some(heading) = self.body.headings.last_mut()
        {
            heading.text.push_str(t);
        }
        if self.image_depth > 0 {
            match event {
                Event::Start(Tag::Image { .. }) => self.image_depth += 1,
                Event::End(TagEnd::Image) => {
                    self.image_depth -= 1;
                    if self.image_depth == 0 {
                        self.end();
                        return;
                    }
                }
                _ => {}
            }
            self.touch(&range);
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag, range),
            Event::End(_) => self.end(),
            Event::Text(t) => self.text_event(&t, range),
            Event::Code(_) => self.code(range),
            Event::InlineHtml(_) => {
                self.tags.push(range.clone());
                self.touch(&range);
            }
            Event::HardBreak => {
                if self.text.as_bytes()[range.start] == b'\\' {
                    self.hide(range.start..range.start + 1);
                }
                self.touch(&range);
            }
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
            _ => self.touch(&range),
        }
    }

    fn start(&mut self, tag: Tag<'_>, range: Range<usize>) {
        self.touch(&range);
        let mut link = None;
        let kind = match tag {
            Tag::Paragraph => FrameKind::Block(BlockKind::Paragraph),
            Tag::Heading { level, .. } => {
                self.in_heading = true;
                self.body.headings.push(Heading {
                    level: heading_level(level),
                    text: String::new(),
                    range: self.trim(&range),
                });
                FrameKind::Block(BlockKind::Heading {
                    level: heading_level(level),
                })
            }
            Tag::BlockQuote(kind) => FrameKind::Block(kind.map_or(BlockKind::Quote, |kind| {
                BlockKind::Callout(callout_kind(kind))
            })),
            Tag::CodeBlock(kind) => FrameKind::Block(match kind {
                CodeBlockKind::Fenced(info) => BlockKind::Code {
                    language: info.split_whitespace().next().map(str::to_owned),
                    fenced: true,
                },
                CodeBlockKind::Indented => BlockKind::Code {
                    language: None,
                    fenced: false,
                },
            }),
            Tag::HtmlBlock => FrameKind::Block(BlockKind::Html),
            Tag::List(start) => FrameKind::Block(BlockKind::List { start }),
            Tag::Item => FrameKind::Block(BlockKind::Item { task: None }),
            Tag::Table(alignments) => FrameKind::Block(BlockKind::Table {
                alignments: alignments.into_iter().map(alignment).collect(),
            }),
            Tag::TableHead => FrameKind::Block(BlockKind::TableRow { head: true }),
            Tag::TableRow => FrameKind::Block(BlockKind::TableRow { head: false }),
            Tag::TableCell => FrameKind::Block(BlockKind::TableCell),
            Tag::Emphasis => FrameKind::Inline(Inline::Emphasis),
            Tag::Strong => FrameKind::Inline(Inline::Strong),
            Tag::Strikethrough => FrameKind::Inline(Inline::Strikethrough),
            Tag::Link {
                link_type,
                dest_url,
                ..
            } => {
                link = Some(Box::new((link::target(&dest_url, link_type), link_type)));
                FrameKind::Inline(Inline::Link)
            }
            Tag::Image {
                link_type,
                dest_url,
                ..
            } => {
                self.image_depth = 1;
                link = Some(Box::new((link::target(&dest_url, link_type), link_type)));
                FrameKind::Inline(Inline::Image)
            }
            _ => FrameKind::Other,
        };
        self.frames.push(Frame {
            kind,
            range,
            first: None,
            last: None,
            children: Vec::new(),
            texts_from: self.texts.len(),
            tags_from: self.tags.len(),
            link,
        });
    }

    fn end(&mut self) {
        let Some(mut frame) = self.frames.pop() else {
            return;
        };
        match std::mem::replace(&mut frame.kind, FrameKind::Other) {
            FrameKind::Block(kind) => self.end_block(kind, frame),
            FrameKind::Inline(inline) => self.end_inline(inline, frame),
            FrameKind::Other => {}
        }
    }

    fn end_block(&mut self, kind: BlockKind, frame: Frame) {
        let range = self.trim(&frame.range);
        // Inline events always belong to the innermost block, so its entries are last.
        let tags = std::mem::take(&mut self.tags);
        self.pair_tags(&tags[frame.tags_from..]);
        self.tags = tags;
        self.tags.truncate(frame.tags_from);
        let texts = std::mem::take(&mut self.texts);
        self.pair_highlights(&texts[frame.texts_from..]);
        self.texts = texts;
        self.texts.truncate(frame.texts_from);
        match &kind {
            BlockKind::Heading { .. } => {
                self.in_heading = false;
                self.hide_heading_syntax(&range, &frame);
            }
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

    fn end_inline(&mut self, inline: Inline, frame: Frame) {
        let range = frame.range;
        let style = match inline {
            Inline::Emphasis => Style::Emphasis,
            Inline::Strong => Style::Strong,
            Inline::Strikethrough => Style::Strikethrough,
            Inline::Link => Style::Link,
            Inline::Image => {
                self.hide(range.clone());
                self.push_link(&range, frame.link, frame.last, true);
                return;
            }
        };
        if let (Some(first), Some(last)) = (frame.first, frame.last) {
            self.span(first..last, style);
            self.hide(range.start..first);
            self.hide(last..range.end);
        }
        if let Inline::Link = inline {
            self.push_link(&range, frame.link, frame.last, false);
        }
    }

    fn push_link(
        &mut self,
        range: &Range<usize>,
        link: Option<Box<(Option<Target>, LinkType)>>,
        last: Option<usize>,
        embedded: bool,
    ) {
        let Some((Some(target), link_type)) = link.map(|link| *link) else {
            return;
        };
        let target_range = match link_type {
            LinkType::Inline => {
                let open = if embedded { 2 } else { 1 };
                let text_end = last.unwrap_or(range.start + open);
                link::target_range(self.text, text_end, range.end)
            }
            LinkType::Autolink | LinkType::Email => Some(range.start + 1..range.end - 1),
            _ => None,
        };
        self.body.links.push(Link {
            range: range.clone(),
            target,
            target_range,
            embedded,
        });
    }

    fn text_event(&mut self, text: &str, range: Range<usize>) {
        self.touch(&range);
        // A code block holds only text, so it is always the top frame.
        let in_code = matches!(
            self.frames.last().map(|frame| &frame.kind),
            Some(FrameKind::Block(BlockKind::Code { .. }))
        );
        if in_code || range.is_empty() {
            return;
        }
        let bytes = self.text.as_bytes();
        // An escaped char starts its own text event, right after the backslash.
        if is_escaped(bytes, range.start) && bytes[range.start].is_ascii_punctuation() {
            self.hide(range.start - 1..range.start);
        }
        if &self.text[range.clone()] == text {
            self.texts.push(range);
        }
    }

    fn code(&mut self, range: Range<usize>) {
        self.touch(&range);
        let source = &self.text.as_bytes()[range.clone()];
        let ticks = source.iter().take_while(|&&b| b == b'`').count();
        if ticks * 2 <= source.len() {
            let inner = range.start + ticks..range.end - ticks;
            self.hide(range.start..inner.start);
            self.hide(inner.end..range.end);
            self.span(inner, Style::Code);
        }
    }

    fn task(&mut self, checked: bool, range: Range<usize>) {
        self.touch(&range);
        let item = self
            .frames
            .iter_mut()
            .rev()
            .find(|frame| matches!(frame.kind, FrameKind::Block(BlockKind::Item { .. })));
        if let Some(frame) = item {
            frame.kind = FrameKind::Block(BlockKind::Item {
                task: Some(Task { checked, range }),
            });
        }
    }

    /// Hides `# ` before the text and the closing `#`s, or the `===` line of a setext heading.
    fn hide_heading_syntax(&mut self, range: &Range<usize>, frame: &Frame) {
        if let Some(heading) = self.body.headings.last_mut() {
            let trimmed = heading.text.trim();
            if trimmed.len() != heading.text.len() {
                heading.text = trimmed.to_owned();
            }
        }
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
            .filter(|frame| {
                matches!(
                    frame.kind,
                    FrameKind::Block(BlockKind::Quote | BlockKind::Callout(_))
                )
            })
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

    /// Pairs `<u>`, `<span style>` and `<mark>` tags inside one block.
    fn pair_tags(&mut self, tags: &[Range<usize>]) {
        let mut open: Vec<(html::Name, Range<usize>, Style)> = Vec::new();
        for range in tags {
            match html::parse(&self.text[range.clone()]) {
                Some(html::Tag::Open(name, style)) => open.push((name, range.clone(), style)),
                Some(html::Tag::Close(name)) => {
                    if let Some(i) = open.iter().rposition(|(n, _, _)| *n == name) {
                        let (_, start, style) = open.remove(i);
                        self.span(start.end..range.start, style);
                        self.hide(start);
                        self.hide(range.clone());
                    }
                }
                None => {}
            }
        }
    }

    /// Pairs `==` runs inside one block. `texts` are text events in order.
    fn pair_highlights(&mut self, texts: &[Range<usize>]) {
        let bytes = self.text.as_bytes();
        let mut open: Option<Range<usize>> = None;
        let mut i = 0;
        while i < texts.len() {
            // Text events next to each other form one run of text.
            let start = texts[i].start;
            let mut end = texts[i].end;
            i += 1;
            while i < texts.len() && texts[i].start == end {
                end = texts[i].end;
                i += 1;
            }
            let mut p = start;
            while let Some(found) = self.text[p..end].find('=') {
                p += found;
                let mut s = p;
                while p < end && bytes[p] == b'=' {
                    p += 1;
                }
                if is_escaped(bytes, s) {
                    s += 1;
                }
                if p - s != 2 {
                    continue;
                }
                let before = self.text[..s].chars().next_back();
                let after = self.text[p..].chars().next();
                let can_close = before.is_some_and(|c| !c.is_whitespace());
                let can_open = after.is_some_and(|c| !c.is_whitespace());
                match open.take() {
                    Some(o) if can_close => {
                        self.span(o.end..s, Style::Highlight(None));
                        self.hide(o);
                        self.hide(s..p);
                    }
                    o => open = if can_open { Some(s..p) } else { o },
                }
            }
        }
    }

    fn finish(mut self) -> Body {
        while !self.frames.is_empty() {
            self.end();
        }
        let mut body = self.body;
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
        let mut merged: Vec<Range<usize>> = Vec::with_capacity(body.hidden.len());
        for range in body.hidden {
            match merged.last_mut() {
                Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
                _ => merged.push(range),
            }
        }
        body.hidden = merged;
        body
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

    fn block_frame(&mut self) -> Option<&mut Frame> {
        self.frames
            .iter_mut()
            .rev()
            .find(|frame| matches!(frame.kind, FrameKind::Block(_)))
    }

    fn push_block(&mut self, block: Block) {
        match self.block_frame() {
            Some(frame) => frame.children.push(block),
            None => self.body.blocks.push(block),
        }
    }

    fn span(&mut self, range: Range<usize>, style: Style) {
        if !range.is_empty() {
            self.body.spans.push(Span { range, style });
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

/// Whether an odd number of backslashes stands right before `pos`.
fn is_escaped(bytes: &[u8], pos: usize) -> bool {
    bytes[..pos]
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count()
        % 2
        == 1
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
