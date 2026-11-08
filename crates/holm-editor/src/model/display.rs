//! Source lines as the screen shows them.

use std::ops::Range;

use holm_markdown::{Alignment, Block, BlockKind, CalloutKind, Color, Style, Target};

use super::Model;

/// How a piece of screen text looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct Look {
    pub(crate) strong: bool,
    pub(crate) emphasis: bool,
    pub(crate) strikethrough: bool,
    pub(crate) underline: bool,
    pub(crate) code: bool,
    pub(crate) link: bool,
    /// Markdown syntax on a line that shows it.
    pub(crate) syntax: bool,
    pub(crate) color: Option<Color>,
    pub(crate) highlight: Highlight,
}

/// The background of a highlight.
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

/// Screen text with its looks, and the map back to its source.
#[derive(Clone, Debug, Default)]
pub(crate) struct Shown {
    pub(crate) text: String,
    /// The looks of `text`, in order. Their lengths add up to the length of `text`.
    pub(crate) runs: Vec<Run>,
    /// Where an empty text maps to.
    start: usize,
    segments: Vec<Segment>,
}

/// A part of the source that the screen shows.
#[derive(Clone, Copy, Debug)]
struct Segment {
    display: usize,
    source: usize,
    len: usize,
}

/// One source line as the screen shows it.
#[derive(Clone, Debug)]
pub(crate) struct DisplayLine {
    /// The source line without its hidden syntax.
    pub(crate) shown: Shown,
    /// The heading level, when the line is part of a heading.
    pub(crate) heading: Option<u8>,
    /// The targets of the embedded files on the line, in order.
    pub(crate) files: Vec<String>,
    /// Whether the line shows its syntax.
    pub(crate) raw: bool,
    pub(crate) kind: LineKind,
    /// The quotes and callouts around the line, from the outside in.
    pub(crate) quotes: Vec<Quote>,
    /// How many list items hold the line.
    pub(crate) depth: usize,
    /// The marker of a list item that starts on this line. Only a line that hides its syntax has it.
    pub(crate) mark: Option<Mark>,
    /// The source before the text of a list line: its indent and its marker. A line that shows
    /// its syntax hangs it to the left, so the text stays in place.
    pub(crate) prefix: Option<Range<usize>>,
    source: Range<usize>,
}

/// What a line is part of, when the screen draws it in its own way.
#[derive(Clone, Debug)]
pub(crate) enum LineKind {
    Text,
    /// A line of a code block.
    Code {
        language: String,
        /// The source of all code lines of the block, without the fences.
        code: Range<usize>,
        /// Whether the line is a fence.
        fence: bool,
    },
    /// The `> [!note]` line of a callout, which shows the callout name instead.
    CalloutTitle(CalloutKind),
    /// A table row in the grid.
    Row {
        head: bool,
        /// The source of the whole table.
        table: Range<usize>,
        cells: Vec<Cell>,
    },
    /// The `|---|` line of a table in the grid.
    TableRule,
    /// A thematic break like `---`.
    Rule,
}

/// One table cell in the grid.
#[derive(Clone, Debug)]
pub(crate) struct Cell {
    pub(crate) shown: Shown,
    pub(crate) alignment: Alignment,
}

/// A quote around a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Quote {
    Plain,
    Callout(CalloutKind),
}

/// The marker of a list item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Mark {
    Bullet,
    /// The number with its `.` or `)`.
    Number(String),
    /// A task box, with the source of its `[ ]`.
    Task {
        checked: bool,
        range: Range<usize>,
    },
}

impl Model {
    /// Line `line` as the screen shows it.
    pub(crate) fn display_line(&self, line: usize) -> DisplayLine {
        let source = self.line_range(line);
        let chain = self.blocks_of_line(&source);
        let from = chain.first().map_or(source.start, |b| b.range.start);

        // Code blocks and tables show their whole source while the selection touches them.
        let whole = chain
            .iter()
            .find(|b| matches!(b.kind, BlockKind::Code { .. } | BlockKind::Table { .. }));
        let raw = match whole {
            Some(block) => self.is_raw(&block.range),
            None => self.is_raw(&source),
        };

        let quotes = chain
            .iter()
            .filter_map(|b| match b.kind {
                BlockKind::Quote => Some(Quote::Plain),
                BlockKind::Callout(kind) => Some(Quote::Callout(kind)),
                _ => None,
            })
            .collect();
        let items: Vec<&Block> = chain
            .iter()
            .copied()
            .filter(|b| matches!(b.kind, BlockKind::Item { .. }))
            .collect();

        let kind = self.line_kind(line, &source, &chain, raw);
        let mut mark = None;
        let mut list_prefix = None;
        if matches!(kind, LineKind::Text) {
            if let Some(item) = items.last().filter(|i| self.line_of(i.range.start) == line) {
                let (found, end) = self.mark(item);
                mark = (!raw).then_some(found);
                list_prefix = Some(source.start..end);
            } else if !items.is_empty() {
                let text = &self.text[source.clone()];
                let spaces = text.len() - text.trim_start().len();
                list_prefix = Some(source.start..source.start + spaces);
            }
        }

        let shown = match kind {
            LineKind::Row { .. } | LineKind::TableRule | LineKind::Rule => Shown {
                start: source.start,
                ..Shown::default()
            },
            _ => self.shown(&source, from, raw, list_prefix.clone().filter(|_| !raw)),
        };

        DisplayLine {
            shown,
            heading: chain.iter().find_map(|b| match b.kind {
                BlockKind::Heading { level } => Some(level),
                _ => None,
            }),
            files: self.files_in(&source),
            raw,
            kind,
            quotes,
            depth: items.len(),
            mark,
            prefix: list_prefix.filter(|prefix| !prefix.is_empty()),
            source,
        }
    }

    /// The blocks that hold the source line `line`, from the top level block inward.
    pub(super) fn blocks_of_line(&self, line: &Range<usize>) -> Vec<&Block> {
        self.chain_at(self.probe(line))
    }

    /// The first offset of the line that is neither space nor hidden syntax. It finds the blocks
    /// of the line, because a nested block can start after the line start.
    fn probe(&self, line: &Range<usize>) -> usize {
        let hidden = self.body.hidden();
        let first = hidden.partition_point(|h| h.end <= line.start);
        let mut in_hidden = hidden[first..].iter().peekable();
        let mut first_text = None;
        for (at, c) in self.text[line.clone()].char_indices() {
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

    /// The blocks that hold `offset`, from the top level block inward.
    pub(super) fn chain_at(&self, offset: usize) -> Vec<&Block> {
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
        source: &Range<usize>,
        chain: &[&Block],
        raw: bool,
    ) -> LineKind {
        for block in chain.iter().rev() {
            match &block.kind {
                BlockKind::Code { language, fenced } => {
                    return self.code_kind(line, block, language.as_deref(), *fenced);
                }
                BlockKind::Table { alignments } if !raw => {
                    return self.table_kind(source, chain[0].range.start, block, alignments);
                }
                BlockKind::Rule if !raw => return LineKind::Rule,
                BlockKind::Callout(kind) if !raw && self.line_of(block.range.start) == line => {
                    return LineKind::CalloutTitle(*kind);
                }
                _ => {}
            }
        }
        LineKind::Text
    }

    fn code_kind(
        &self,
        line: usize,
        block: &Block,
        language: Option<&str>,
        fenced: bool,
    ) -> LineKind {
        let first = self.line_of(block.range.start);
        let last = self.line_of(block.range.end);
        let last_range = self.line_range(last);
        let closing = self.text[last_range.clone()].trim();
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
            last_range.start.saturating_sub(1).max(start)
        } else {
            block.range.end.max(start)
        };
        LineKind::Code {
            language: language.unwrap_or_default().to_owned(),
            code: start..end,
            fence: fenced && (line == first || (closed && line == last)),
        }
    }

    /// A table line in the grid: a row with its cells, or the `|---|` line.
    fn table_kind(
        &self,
        line: &Range<usize>,
        from: usize,
        table: &Block,
        alignments: &[Alignment],
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
            .zip(
                alignments
                    .iter()
                    .copied()
                    .chain(std::iter::repeat(Alignment::None)),
            )
            .map(|(cell, alignment)| {
                let text = &self.text[cell.range.clone()];
                let start = cell.range.start + text.len() - text.trim_start().len();
                let end = cell.range.start + text.trim_end().len();
                Cell {
                    shown: self.shown(&(start..end.max(start)), from, false, None),
                    alignment,
                }
            })
            .collect();
        LineKind::Row {
            head: matches!(row.kind, BlockKind::TableRow { head: true }),
            table: table.range.clone(),
            cells,
        }
    }

    /// The marker of a list item, and the end of the marker with its spaces.
    fn mark(&self, item: &Block) -> (Mark, usize) {
        let bytes = self.text.as_bytes();
        let start = item.range.start;
        let mut end = start;
        while end < item.range.end && bytes[end].is_ascii_digit() {
            end += 1;
        }
        let mut mark = if end > start {
            end = (end + 1).min(item.range.end);
            Mark::Number(self.text[start..end].to_owned())
        } else {
            end = (start + 1).min(item.range.end);
            Mark::Bullet
        };
        if let BlockKind::Item { task: Some(task) } = &item.kind {
            mark = Mark::Task {
                checked: task.checked,
                range: task.range.clone(),
            };
            end = task.range.end;
        }
        while end < item.range.end && matches!(bytes[end], b' ' | b'\t') {
            end += 1;
        }
        (mark, end)
    }

    /// The screen text of `range`. Without `raw`, it hides the syntax and `extra`.
    fn shown(
        &self,
        range: &Range<usize>,
        from: usize,
        raw: bool,
        extra: Option<Range<usize>>,
    ) -> Shown {
        let spans = self.body.spans();
        let spans: Vec<_> = spans[spans.partition_point(|s| s.range.start < from)
            ..spans.partition_point(|s| s.range.start < range.end)]
            .iter()
            .filter(|s| s.range.end > range.start)
            .collect();
        let hidden = self.body.hidden();
        let mut hidden: Vec<_> = hidden[hidden.partition_point(|h| h.start < from)
            ..hidden.partition_point(|h| h.start < range.end)]
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
        cuts.retain(|cut| range.contains(cut) || *cut == range.end);
        cuts.sort_unstable();
        cuts.dedup();

        let mut shown = Shown {
            start: range.start,
            ..Shown::default()
        };
        for piece in cuts.windows(2) {
            let piece = piece[0]..piece[1];
            let syntax = hidden
                .iter()
                .any(|h| h.start <= piece.start && piece.end <= h.end);
            if syntax && !raw {
                continue;
            }
            let mut look = Look {
                syntax,
                ..Look::default()
            };
            for span in spans
                .iter()
                .filter(|s| s.range.start <= piece.start && piece.end <= s.range.end)
            {
                look.add(span.style);
            }
            shown.push(piece, look, &self.text);
        }
        shown
    }

    fn files_in(&self, line: &Range<usize>) -> Vec<String> {
        let links = self.body.links();
        links[links.partition_point(|l| l.range.start < line.start)
            ..links.partition_point(|l| l.range.start < line.end)]
            .iter()
            .filter(|link| link.embedded)
            .filter_map(|link| match &link.target {
                Target::File(path) => Some(path.clone()),
                _ => None,
            })
            .collect()
    }
}

impl Look {
    fn add(&mut self, style: Style) {
        match style {
            Style::Emphasis => self.emphasis = true,
            Style::Strong => self.strong = true,
            Style::Strikethrough => self.strikethrough = true,
            Style::Code => self.code = true,
            Style::Underline => self.underline = true,
            Style::Highlight(color) => {
                self.highlight = color.map_or(Highlight::Default, Highlight::Color);
            }
            Style::Color(color) => self.color = Some(color),
            Style::Link => self.link = true,
        }
    }
}

impl Shown {
    fn push(&mut self, piece: Range<usize>, look: Look, source: &str) {
        let len = piece.len();
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
        self.text.push_str(&source[piece]);
    }

    /// The source offset for a screen offset. Between two visible parts it picks the later one.
    pub(crate) fn to_source(&self, display: usize) -> usize {
        let Some(ix) = self
            .segments
            .partition_point(|s| s.display <= display)
            .checked_sub(1)
        else {
            return self.start;
        };
        let segment = self.segments[ix];
        segment.source + (display - segment.display).min(segment.len)
    }

    /// The screen offset for a source offset. Hidden source maps to the next visible text.
    pub(crate) fn to_display(&self, source: usize) -> usize {
        let Some(ix) = self
            .segments
            .partition_point(|s| s.source <= source)
            .checked_sub(1)
        else {
            return 0;
        };
        let segment = self.segments[ix];
        segment.display + (source - segment.source).min(segment.len)
    }
}

impl DisplayLine {
    /// The source of the line, without its line break.
    pub(crate) fn source(&self) -> Range<usize> {
        self.source.clone()
    }

    /// The source offset for a screen offset.
    pub(crate) fn to_source(&self, display: usize) -> usize {
        self.shown.to_source(display)
    }

    /// The screen offset for a source offset.
    pub(crate) fn to_display(&self, source: usize) -> usize {
        self.shown.to_display(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_at(text: &str, cursor: usize) -> Model {
        let mut model = Model::new(text);
        model.set_cursor(cursor, false);
        model
    }

    #[test]
    fn list_items_hide_their_markers_and_know_their_depth() {
        let model = model_at("x\n\n- one\n  - two\n1. three\n- [x] done\n", 0);
        let one = model.display_line(2);
        assert_eq!(one.shown.text, "one");
        assert_eq!(one.mark, Some(Mark::Bullet));
        assert_eq!(one.depth, 1);
        let two = model.display_line(3);
        assert_eq!(two.shown.text, "two");
        assert_eq!(two.depth, 2);
        assert_eq!(model.display_line(4).mark, Some(Mark::Number("1.".into())));
        let done = model.display_line(5);
        assert_eq!(done.shown.text, "done");
        assert!(matches!(done.mark, Some(Mark::Task { checked: true, .. })));
    }

    #[test]
    fn a_list_line_with_the_cursor_shows_its_marker() {
        let model = model_at("- one\n", 3);
        let line = model.display_line(0);
        assert_eq!(line.shown.text, "- one");
        assert_eq!(line.mark, None);
        assert_eq!(line.prefix, Some(0..2));
    }

    #[test]
    fn a_code_block_shows_its_fences_while_the_cursor_is_inside() {
        let text = "a\n```rust\nfn x() {}\n```\nb";
        let away = model_at(text, 0);
        let fence = away.display_line(1);
        assert!(!fence.raw);
        assert!(matches!(fence.kind, LineKind::Code { fence: true, .. }));
        let LineKind::Code {
            language,
            code,
            fence: false,
        } = away.display_line(2).kind
        else {
            panic!("line 2 is code");
        };
        assert_eq!(language, "rust");
        assert_eq!(&text[code], "fn x() {}");
        assert!(matches!(
            away.display_line(3).kind,
            LineKind::Code { fence: true, .. }
        ));

        let inside = model_at(text, 12);
        assert!(inside.display_line(1).raw);
        assert!(inside.display_line(3).raw);
        assert_eq!(inside.display_line(1).shown.text, "```rust");
    }

    #[test]
    fn a_table_is_a_grid_until_the_cursor_is_inside() {
        let text = "a\n\n| x | **y** |\n|---|--:|\n| 1 | 2 |\n";
        let away = model_at(text, 0);
        let LineKind::Row { head, cells, .. } = away.display_line(2).kind else {
            panic!("line 2 is a row");
        };
        assert!(head);
        assert_eq!(cells[0].shown.text, "x");
        assert_eq!(cells[1].shown.text, "y");
        assert_eq!(cells[1].alignment, Alignment::Right);
        assert!(matches!(away.display_line(3).kind, LineKind::TableRule));

        let inside = model_at(text, text.find('2').unwrap_or(0));
        for line in 2..=4 {
            let display = inside.display_line(line);
            assert!(display.raw);
            assert!(matches!(display.kind, LineKind::Text));
        }
    }

    #[test]
    fn a_callout_shows_its_name_and_its_quote() {
        let model = model_at("a\n\n> [!tip]\n> Use it.\n", 0);
        let title = model.display_line(2);
        assert!(matches!(
            title.kind,
            LineKind::CalloutTitle(CalloutKind::Tip)
        ));
        let body = model.display_line(3);
        assert_eq!(body.quotes, [Quote::Callout(CalloutKind::Tip)]);
        assert_eq!(body.shown.text, "Use it.");
    }

    #[test]
    fn a_rule_away_from_the_cursor_shows_no_text() {
        let model = model_at("a\n\n---\n", 0);
        let line = model.display_line(2);
        assert!(matches!(line.kind, LineKind::Rule));
        assert_eq!(line.shown.text, "");
    }

    #[test]
    fn task_boxes_toggle_and_the_cursor_stays() {
        let mut model = model_at("- [ ] a\n- [x] b\n- c\n", 0);
        assert!(model.toggle_task(&(2..5)));
        assert_eq!(model.text(), "- [x] a\n- [x] b\n- c\n");
        assert_eq!(model.cursor(), 0);
        model.select(0..12);
        assert!(model.toggle_tasks());
        assert_eq!(model.text(), "- [ ] a\n- [ ] b\n- c\n");
        model.set_cursor(17, false);
        assert!(!model.toggle_tasks());
        model.undo();
        assert_eq!(model.text(), "- [x] a\n- [x] b\n- c\n");
    }
}
