use std::ops::Range;

use holm_markdown::{
    Alignment, Block, BlockKind, CalloutKind, CodeStyle, Color, LinkKind, OffsetRange, SourceOffset, Style, TablePart,
    Target, TaskState,
};

use super::Model;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Look {
    pub(crate) styles: Styles,
    pub(crate) link: Option<LinkTarget>,
    pub(crate) kind: TextKind,
    pub(crate) color: Option<Color>,
    pub(crate) highlight: Highlight,
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub(crate) struct Styles: u8 {
        const STRONG = 1;
        const EMPHASIS = 1 << 1;
        const STRIKETHROUGH = 1 << 2;
        const UNDERLINE = 1 << 3;
        const CODE = 1 << 4;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinkTarget {
    Found,
    Missing,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TextKind {
    #[default]
    Content,
    /// Markdown syntax on a line that shows it.
    Syntax,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Highlight {
    #[default]
    None,
    Default,
    Color(Color),
}

/// A run of screen text with one look.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Run {
    pub(crate) len: usize,
    pub(crate) look: Look,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Shown {
    pub(crate) text: String,
    /// In order. Their lengths add up to the length of `text`.
    pub(crate) runs: Vec<Run>,
    /// In order.
    pub(crate) chips: Vec<Chip>,
    /// Where an empty text maps to.
    start: SourceOffset,
    segments: Vec<Segment>,
}

/// An embedded file that is not an image. It shows in the line as a chip. Its screen text is one [`CHIP_ATOM`], which
/// is not in the source and maps to the start of the link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Chip {
    /// In screen text.
    pub(crate) range: Range<usize>,
    pub(crate) target: String,
    /// The link text as plain text, with no Markdown escapes.
    pub(crate) name: String,
}

/// The layout puts a box in its place.
const CHIP_ATOM: &str = "\u{fffc}";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Embed {
    /// An image under the line.
    Image,
    /// A chip in the line.
    Chip,
}

#[cfg(test)]
pub(crate) fn plain_chip(_target: &str) -> Embed {
    Embed::Chip
}

struct ChipLink<'a> {
    range: Range<SourceOffset>,
    target: &'a str,
    name: &'a str,
}

/// A part of the source that the screen shows.
#[derive(Clone, Copy, Debug)]
struct Segment {
    display: usize,
    source: SourceOffset,
    len: usize,
}

/// A line shows its syntax while it has the cursor or touches the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Syntax {
    Shown,
    Hidden,
}

#[derive(Clone, Debug)]
pub(crate) struct DisplayLine {
    /// The source line without its hidden syntax.
    pub(crate) shown: Shown,
    pub(crate) heading: Option<u8>,
    /// In order. They show under the line.
    pub(crate) images: Vec<String>,
    pub(crate) syntax: Syntax,
    pub(crate) kind: LineKind,
    /// The quotes and callouts, from the outside in.
    pub(crate) quotes: Vec<Quote>,
    /// How many list items hold the line.
    pub(crate) depth: usize,
    /// The marker of a list item that starts on this line. Only a line that hides its syntax has it.
    pub(crate) mark: Option<Mark>,
    /// The source before the text of a list line: its indent and its marker. A line that shows its syntax hangs it to
    /// the left, so the text stays in place.
    pub(crate) prefix: Option<Range<SourceOffset>>,
    source: Range<SourceOffset>,
}

/// Which line of a code block a line is.
#[derive(Clone, Debug)]
pub(crate) enum CodeLine {
    /// An opening or closing fence.
    Fence,
    Code,
}

#[derive(Clone, Debug)]
pub(crate) enum LineKind {
    Text,
    Code {
        language: String,
        /// The source of all code lines of the block, without the fences.
        code: Range<SourceOffset>,
        line: CodeLine,
    },
    /// The `> [!note]` line of a callout, which shows the callout name instead.
    CalloutTitle(CalloutKind),
    Row {
        part: TablePart,
        /// The source of the whole table.
        table: Range<SourceOffset>,
        cells: Vec<Cell>,
    },
    /// The `|---|` line of a table in the grid.
    TableRule,
    /// A thematic break like `---`.
    Rule,
}

#[derive(Clone, Debug)]
pub(crate) struct Cell {
    pub(crate) shown: Shown,
    pub(crate) alignment: Alignment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Quote {
    Plain,
    Callout(CalloutKind),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Mark {
    Bullet,
    /// The number with its `.` or `)`.
    Number(String),
    /// A task box, with the source of its `[ ]`.
    Task {
        state: TaskState,
        range: Range<SourceOffset>,
    },
}

impl Model {
    /// `embed` tells how an embedded file with a target shows.
    pub(crate) fn display_line(&self, line: usize, embed: &mut dyn FnMut(&str) -> Embed) -> DisplayLine {
        let source = self.line_range(line);
        let mut images = Vec::new();
        let mut chips = Vec::new();
        for (range, target, name) in self.files_in(&source) {
            match embed(target) {
                Embed::Image => images.push(target.to_owned()),
                Embed::Chip => chips.push(ChipLink { range, target, name }),
            }
        }
        let chain = self.blocks_of_line(&source);
        let from = chain.first().map_or(source.start, |b| b.range.start);

        // Code blocks and tables show their whole source while the selection touches them.
        let whole = chain
            .iter()
            .find(|b| matches!(b.kind, BlockKind::Code { .. } | BlockKind::Table { .. }));
        let syntax = match whole {
            Some(block) => self.syntax_of(&block.range),
            None => self.syntax_of(&source),
        };

        let quotes = chain
            .iter()
            .filter_map(|b| match b.kind {
                BlockKind::Quote => Some(Quote::Plain),
                BlockKind::Callout(kind) => Some(Quote::Callout(kind)),
                _ => None,
            })
            .collect();
        let mut items: Vec<&Block> = chain
            .iter()
            .copied()
            .filter(|b| matches!(b.kind, BlockKind::Item { .. }))
            .collect();
        if items.is_empty() && source.is_empty() {
            items = self.lazy_items(line);
        }

        let kind = self.line_kind(line, &source, &chain, syntax, &chips);
        let mut mark = None;
        let mut list_prefix = None;
        if matches!(kind, LineKind::Text) {
            if let Some(item) = items.last().filter(|i| self.line_of(i.range.start) == line) {
                let (found, end) = self.mark(item);
                mark = (syntax == Syntax::Hidden).then_some(found);
                list_prefix = Some(source.start..end);
            } else if !items.is_empty() {
                let text = &self.text[source.bytes()];
                let spaces = text.len() - text.trim_start().len();
                list_prefix = Some(source.start..source.start + spaces);
            }
        }

        let shown = match kind {
            LineKind::Row { .. } | LineKind::TableRule | LineKind::Rule => Shown {
                start: source.start,
                ..Shown::default()
            },
            _ => {
                let extra = list_prefix.clone().filter(|_| syntax == Syntax::Hidden);
                self.shown(&source, from, syntax, extra, &chips)
            }
        };

        DisplayLine {
            shown,
            heading: chain.iter().find_map(|b| match b.kind {
                BlockKind::Heading { level } => Some(level),
                _ => None,
            }),
            images,
            syntax,
            kind,
            quotes,
            depth: items.len(),
            mark,
            prefix: list_prefix.filter(|prefix| !prefix.is_empty()),
            source,
        }
    }

    /// The list items of the line above the empty line `line`, when that line is item text. Text typed on `line` joins
    /// these items, by lazy continuation.
    fn lazy_items(&self, line: usize) -> Vec<&Block> {
        let Some(above) = line.checked_sub(1).map(|above| self.line_range(above)) else {
            return Vec::new();
        };
        if self.text[above.bytes()].trim().is_empty() {
            return Vec::new();
        }
        let chain = self.blocks_of_line(&above);
        if !chain
            .last()
            .is_some_and(|b| matches!(b.kind, BlockKind::Item { .. } | BlockKind::Paragraph))
        {
            return Vec::new();
        }
        chain
            .into_iter()
            .filter(|b| matches!(b.kind, BlockKind::Item { .. }))
            .collect()
    }

    /// From the top level block inward.
    pub(super) fn blocks_of_line(&self, line: &Range<SourceOffset>) -> Vec<&Block> {
        self.chain_at(self.probe(line))
    }

    /// The first offset of the line that is neither space nor hidden syntax. It finds the blocks of the line, because a
    /// nested block can start after the line start.
    fn probe(&self, line: &Range<SourceOffset>) -> SourceOffset {
        let hidden = self.body.hidden();
        let first = hidden.partition_point(|h| h.end <= line.start);
        let mut in_hidden = hidden[first..].iter().peekable();
        let mut first_text = None;
        for (at, c) in self.text[line.bytes()].char_indices() {
            if c.is_whitespace() {
                continue;
            }
            let at = line.start + at;
            first_text.get_or_insert(at);
            while in_hidden.peek().is_some_and(|h| h.end <= at) {
                in_hidden.next();
            }
            if in_hidden.peek().is_none_or(|h| h.start > at) {
                return at;
            }
        }
        first_text.unwrap_or(line.start)
    }

    /// From the top level block inward.
    pub(super) fn chain_at(&self, offset: SourceOffset) -> Vec<&Block> {
        let mut chain = Vec::new();
        let mut blocks = self.body.blocks();
        loop {
            let ix = blocks.partition_point(|b| b.range.start <= offset);
            let Some(block) = ix
                .checked_sub(1)
                .map(|ix| &blocks[ix])
                .filter(|b| offset <= b.range.end)
            else {
                return chain;
            };
            chain.push(block);
            blocks = &block.children;
        }
    }

    fn line_kind(
        &self,
        line: usize,
        source: &Range<SourceOffset>,
        chain: &[&Block],
        syntax: Syntax,
        chips: &[ChipLink<'_>],
    ) -> LineKind {
        for block in chain.iter().rev() {
            match &block.kind {
                BlockKind::Code { language, style } => {
                    return self.code_kind(line, block, language.as_deref(), *style);
                }
                BlockKind::Table { alignments } if syntax == Syntax::Hidden => {
                    return self.table_kind(source, chain[0].range.start, block, alignments, chips);
                }
                BlockKind::Rule if syntax == Syntax::Hidden => return LineKind::Rule,
                BlockKind::Callout(kind) if syntax == Syntax::Hidden && self.line_of(block.range.start) == line => {
                    return LineKind::CalloutTitle(*kind);
                }
                _ => {}
            }
        }
        LineKind::Text
    }

    fn code_kind(&self, line: usize, block: &Block, language: Option<&str>, style: CodeStyle) -> LineKind {
        let fenced = style == CodeStyle::Fenced;
        let first = self.line_of(block.range.start);
        let last = self.line_of(block.range.end);
        let last_range = self.line_range(last);
        let closing = self.text[last_range.bytes()].trim();
        let closed = fenced
            && last > first
            && closing.len() >= 3
            && (closing.bytes().all(|b| b == b'`') || closing.bytes().all(|b| b == b'~'));
        let start = if fenced {
            (self.line_range(first).end + 1).min(block.range.end)
        } else {
            self.line_range(first).start
        };
        let end = if closed {
            (last_range.start - 1).max(start)
        } else {
            block.range.end.max(start)
        };
        LineKind::Code {
            language: language.unwrap_or_default().to_owned(),
            code: start..end,
            line: if fenced && (line == first || (closed && line == last)) {
                CodeLine::Fence
            } else {
                CodeLine::Code
            },
        }
    }

    fn table_kind(
        &self,
        line: &Range<SourceOffset>,
        from: SourceOffset,
        table: &Block,
        alignments: &[Alignment],
        chips: &[ChipLink<'_>],
    ) -> LineKind {
        let row = table
            .children
            .iter()
            .find(|row| line.start <= row.range.start && row.range.start <= line.end);
        let Some(row) = row else {
            return LineKind::TableRule;
        };
        let cells = row
            .children
            .iter()
            .zip(alignments.iter().copied().chain(std::iter::repeat(Alignment::None)))
            .map(|(cell, alignment)| {
                let text = &self.text[cell.range.bytes()];
                let start = cell.range.start + text.len() - text.trim_start().len();
                let end = cell.range.start + text.trim_end().len();
                Cell {
                    shown: self.shown(&(start..end.max(start)), from, Syntax::Hidden, None, chips),
                    alignment,
                }
            })
            .collect();
        LineKind::Row {
            part: match row.kind {
                BlockKind::TableRow { part } => part,
                _ => TablePart::Body,
            },
            table: table.range.clone(),
            cells,
        }
    }

    /// The marker of a list item, and the end of the marker with its spaces.
    fn mark(&self, item: &Block) -> (Mark, SourceOffset) {
        let bytes = self.text.as_bytes();
        let start = item.range.start;
        let mut end = start;
        while end < item.range.end && bytes[end.get()].is_ascii_digit() {
            end += 1;
        }
        let mut mark = if end > start {
            end = (end + 1).min(item.range.end);
            Mark::Number(self.text[(start..end).bytes()].to_owned())
        } else {
            end = (start + 1).min(item.range.end);
            Mark::Bullet
        };
        if let BlockKind::Item { task: Some(task) } = &item.kind {
            mark = Mark::Task {
                state: task.state,
                range: task.range.clone(),
            };
            end = task.range.end;
        }
        while end < item.range.end && matches!(bytes[end.get()], b' ' | b'\t') {
            end += 1;
        }
        (mark, end)
    }

    /// The screen text of `range`. With [`Syntax::Hidden`], it hides the syntax and `extra`, and shows the `chips` that
    /// start in `range`.
    fn shown(
        &self,
        range: &Range<SourceOffset>,
        from: SourceOffset,
        syntax: Syntax,
        extra: Option<Range<SourceOffset>>,
        chips: &[ChipLink<'_>],
    ) -> Shown {
        let spans = self.body.spans();
        let spans: Vec<_> = spans
            [spans.partition_point(|s| s.range.start < from)..spans.partition_point(|s| s.range.start < range.end)]
            .iter()
            .filter(|s| s.range.end > range.start)
            .collect();
        let hidden = self.body.hidden();
        let mut hidden: Vec<_> = hidden
            [hidden.partition_point(|h| h.start < from)..hidden.partition_point(|h| h.start < range.end)]
            .iter()
            .filter(|h| h.end > range.start)
            .map(|h| h.start.max(range.start)..h.end.min(range.end))
            .collect();
        hidden.extend(extra.filter(|e| !e.is_empty()));

        let mut cuts = vec![range.start, range.end];
        for span in &spans {
            cuts.extend([span.range.start, span.range.end]);
        }
        for h in &hidden {
            cuts.extend([h.start, h.end]);
        }
        let chips: &[ChipLink<'_>] = match syntax {
            Syntax::Shown => &[],
            Syntax::Hidden => chips,
        };
        let mut chips = chips.iter().filter(|chip| range.contains(&chip.range.start)).peekable();
        cuts.extend(chips.clone().map(|chip| chip.range.start));
        cuts.retain(|cut| range.contains(cut) || *cut == range.end);
        cuts.sort_unstable();
        cuts.dedup();

        let mut shown = Shown {
            start: range.start,
            ..Shown::default()
        };
        for piece in cuts.windows(2) {
            let piece = piece[0]..piece[1];
            while let Some(chip) = chips.next_if(|chip| chip.range.start <= piece.start) {
                shown.push_chip(chip, range.end);
            }
            let is_syntax = hidden.iter().any(|h| h.start <= piece.start && piece.end <= h.end);
            if is_syntax && syntax == Syntax::Hidden {
                continue;
            }
            let mut look = Look {
                kind: if is_syntax { TextKind::Syntax } else { TextKind::Content },
                ..Look::default()
            };
            for span in spans
                .iter()
                .filter(|s| s.range.start <= piece.start && piece.end <= s.range.end)
            {
                look.add(span.style);
            }
            if look.link.is_some()
                && self
                    .link_over(&piece, range.start)
                    .is_some_and(|link| self.is_broken(&link.target))
            {
                look.link = Some(LinkTarget::Missing);
            }
            shown.push(piece, look, &self.text);
        }
        shown
    }

    /// The range, the target and the text of each embedded file link in `line`.
    fn files_in(&self, line: &Range<SourceOffset>) -> Vec<(Range<SourceOffset>, &str, &str)> {
        let links = self.body.links();
        links
            [links.partition_point(|l| l.range.start < line.start)..links.partition_point(|l| l.range.start < line.end)]
            .iter()
            .filter(|link| link.kind == LinkKind::Embedded)
            .filter_map(|link| match &link.target {
                Target::File(path) => Some((link.range.clone(), path.as_str(), &self.text[link.text.bytes()])),
                _ => None,
            })
            .collect()
    }
}

impl Look {
    fn add(&mut self, style: Style) {
        match style {
            Style::Emphasis => self.styles |= Styles::EMPHASIS,
            Style::Strong => self.styles |= Styles::STRONG,
            Style::Strikethrough => self.styles |= Styles::STRIKETHROUGH,
            Style::Code => self.styles |= Styles::CODE,
            Style::Underline => self.styles |= Styles::UNDERLINE,
            Style::Highlight(color) => {
                self.highlight = color.map_or(Highlight::Default, Highlight::Color);
            }
            Style::Color(color) => self.color = Some(color),
            Style::Link => self.link = Some(LinkTarget::Found),
        }
    }
}

impl Shown {
    fn push(&mut self, piece: Range<SourceOffset>, look: Look, source: &str) {
        let len = piece.end - piece.start;
        match self.segments.last_mut() {
            Some(last) if last.source + last.len == piece.start => last.len += len,
            _ => self.segments.push(Segment {
                display: self.text.len(),
                source: piece.start,
                len,
            }),
        }
        match self.runs.last_mut() {
            Some(last) if last.look == look => last.len += len,
            _ => self.runs.push(Run { len, look }),
        }
        self.text.push_str(&source[piece.bytes()]);
    }

    /// Adds a chip. Its left edge maps to the start of its link, and its right edge to the end of its link, cut at
    /// `end`.
    fn push_chip(&mut self, chip: &ChipLink<'_>, end: SourceOffset) {
        let start = self.text.len();
        self.segments.push(Segment {
            display: start,
            source: chip.range.start,
            len: 0,
        });
        self.text.push_str(CHIP_ATOM);
        self.segments.push(Segment {
            display: self.text.len(),
            source: chip.range.end.min(end),
            len: 0,
        });
        self.runs.push(Run {
            len: CHIP_ATOM.len(),
            look: Look {
                link: Some(LinkTarget::Found),
                ..Look::default()
            },
        });
        self.chips.push(Chip {
            range: start..self.text.len(),
            target: chip.target.to_owned(),
            name: holm_markdown::unescape_text(chip.name),
        });
    }

    /// Between two visible parts it picks the later one.
    pub(crate) fn to_source(&self, display: usize) -> SourceOffset {
        let Some(ix) = self.segments.partition_point(|s| s.display <= display).checked_sub(1) else {
            return self.start;
        };
        let segment = self.segments[ix];
        segment.source + (display - segment.display).min(segment.len)
    }

    /// Hidden source maps to the next visible text.
    pub(crate) fn to_display(&self, source: SourceOffset) -> usize {
        let Some(ix) = self.segments.partition_point(|s| s.source <= source).checked_sub(1) else {
            return 0;
        };
        let segment = self.segments[ix];
        segment.display + (source - segment.source).min(segment.len)
    }
}

impl DisplayLine {
    /// Without its line break.
    pub(crate) fn source(&self) -> Range<SourceOffset> {
        self.source.clone()
    }

    pub(crate) fn to_source(&self, display: usize) -> SourceOffset {
        self.shown.to_source(display)
    }

    pub(crate) fn to_display(&self, source: SourceOffset) -> usize {
        self.shown.to_display(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Selection;

    fn model_at(text: &str, cursor: usize) -> Model {
        let mut model = Model::new(text);
        model.set_cursor(SourceOffset::new(cursor), Selection::Collapse);
        model
    }

    #[test]
    fn list_items_hide_their_markers_and_know_their_depth() {
        let model = model_at("x\n\n- one\n  - two\n1. three\n- [x] done\n", 0);
        let one = model.display_line(2, &mut plain_chip);
        assert_eq!(one.shown.text, "one");
        assert_eq!(one.mark, Some(Mark::Bullet));
        assert_eq!(one.depth, 1);
        let two = model.display_line(3, &mut plain_chip);
        assert_eq!(two.shown.text, "two");
        assert_eq!(two.depth, 2);
        assert_eq!(
            model.display_line(4, &mut plain_chip).mark,
            Some(Mark::Number("1.".into()))
        );
        let done = model.display_line(5, &mut plain_chip);
        assert_eq!(done.shown.text, "done");
        assert!(matches!(
            done.mark,
            Some(Mark::Task {
                state: TaskState::Checked,
                ..
            })
        ));
    }

    #[test]
    fn a_chip_has_the_link_text_as_its_name() {
        let model = model_at(
            "x\n\n![photo.png](attachments/photo-3fa9c2e1.png) ![](a/b.pdf) ![a\\_b.toml](a_b.toml)\n",
            0,
        );
        let line = model.display_line(2, &mut plain_chip);
        let names: Vec<_> = line.shown.chips.iter().map(|chip| chip.name.as_str()).collect();
        assert_eq!(names, ["photo.png", "", "a_b.toml"]);
        assert_eq!(line.shown.chips[0].target, "attachments/photo-3fa9c2e1.png");
    }

    #[test]
    fn a_list_line_with_the_cursor_shows_its_marker() {
        let model = model_at("- one\n", 3);
        let line = model.display_line(0, &mut plain_chip);
        assert_eq!(line.shown.text, "- one");
        assert_eq!(line.mark, None);
        assert_eq!(line.prefix, Some(SourceOffset::range(0..2)));
    }

    #[test]
    fn a_code_block_shows_its_fences_while_the_cursor_is_inside() {
        let text = "a\n```rust\nfn x() {}\n```\nb";
        let away = model_at(text, 0);
        let fence = away.display_line(1, &mut plain_chip);
        assert_eq!(fence.syntax, Syntax::Hidden);
        assert!(matches!(
            fence.kind,
            LineKind::Code {
                line: CodeLine::Fence,
                ..
            }
        ));
        let LineKind::Code {
            language,
            code,
            line: CodeLine::Code,
        } = away.display_line(2, &mut plain_chip).kind
        else {
            panic!("line 2 is code");
        };
        assert_eq!(language, "rust");
        assert_eq!(&text[code.bytes()], "fn x() {}");
        assert!(matches!(
            away.display_line(3, &mut plain_chip).kind,
            LineKind::Code {
                line: CodeLine::Fence,
                ..
            }
        ));

        let inside = model_at(text, 12);
        assert_eq!(inside.display_line(1, &mut plain_chip).syntax, Syntax::Shown);
        assert_eq!(inside.display_line(3, &mut plain_chip).syntax, Syntax::Shown);
        assert_eq!(inside.display_line(1, &mut plain_chip).shown.text, "```rust");
    }

    #[test]
    fn a_table_is_a_grid_until_the_cursor_is_inside() {
        let text = "a\n\n| x | **y** |\n|---|--:|\n| 1 | 2 |\n";
        let away = model_at(text, 0);
        let LineKind::Row { part, cells, .. } = away.display_line(2, &mut plain_chip).kind else {
            panic!("line 2 is a row");
        };
        assert_eq!(part, TablePart::Head);
        assert_eq!(cells[0].shown.text, "x");
        assert_eq!(cells[1].shown.text, "y");
        assert_eq!(cells[1].alignment, Alignment::Right);
        assert!(matches!(
            away.display_line(3, &mut plain_chip).kind,
            LineKind::TableRule
        ));

        let inside = model_at(text, text.find('2').unwrap_or(0));
        for line in 2..=4 {
            let display = inside.display_line(line, &mut plain_chip);
            assert_eq!(display.syntax, Syntax::Shown);
            assert!(matches!(display.kind, LineKind::Text));
        }
    }

    #[test]
    fn a_callout_shows_its_name_and_its_quote() {
        let model = model_at("a\n\n> [!tip]\n> Use it.\n", 0);
        let title = model.display_line(2, &mut plain_chip);
        assert!(matches!(title.kind, LineKind::CalloutTitle(CalloutKind::Tip)));
        let body = model.display_line(3, &mut plain_chip);
        assert_eq!(body.quotes, [Quote::Callout(CalloutKind::Tip)]);
        assert_eq!(body.shown.text, "Use it.");
    }

    #[test]
    fn a_rule_away_from_the_cursor_shows_no_text() {
        let model = model_at("a\n\n---\n", 0);
        let line = model.display_line(2, &mut plain_chip);
        assert!(matches!(line.kind, LineKind::Rule));
        assert_eq!(line.shown.text, "");
    }

    #[test]
    fn task_boxes_toggle_and_the_cursor_stays() {
        let mut model = model_at("- [ ] a\n- [x] b\n- c\n", 0);
        assert!(model.toggle_task(&SourceOffset::range(2..5)));
        assert_eq!(model.text(), "- [x] a\n- [x] b\n- c\n");
        assert_eq!(model.cursor(), SourceOffset::new(0));
        model.select(SourceOffset::range(0..12));
        assert!(model.toggle_tasks());
        assert_eq!(model.text(), "- [ ] a\n- [ ] b\n- c\n");
        model.set_cursor(SourceOffset::new(17), Selection::Collapse);
        assert!(!model.toggle_tasks());
        model.undo();
        assert_eq!(model.text(), "- [x] a\n- [x] b\n- c\n");
    }
}
