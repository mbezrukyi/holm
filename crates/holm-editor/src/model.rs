//! The text, the cursor and the selection, and the map from screen text to source text.
//!
//! This module has no GPUI. Every offset is a byte offset in the source text, on a char boundary.

use std::ops::Range;

use holm_markdown::{Block, BlockKind, Body, Color, Style, Target};
use ropey::{LineType, Rope};

const LF: LineType = LineType::LF;

/// The note body with its parse, the cursor and the selection.
pub(crate) struct Model {
    rope: Rope,
    /// The same text as `rope`. The parser and the text shaper need one `&str`.
    text: String,
    body: Body,
    cursor: usize,
    anchor: usize,
}

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

/// One source line as the screen shows it.
#[derive(Clone, Debug)]
pub(crate) struct DisplayLine {
    /// The screen text: the source line without its hidden syntax.
    pub(crate) text: String,
    /// The looks of `text`, in order. Their lengths add up to the length of `text`.
    pub(crate) runs: Vec<Run>,
    /// The heading level, when the line is part of a heading.
    pub(crate) heading: Option<u8>,
    /// The targets of the embedded files on the line, in order.
    pub(crate) files: Vec<String>,
    /// Whether the line shows its syntax.
    pub(crate) raw: bool,
    source: Range<usize>,
    segments: Vec<Segment>,
}

/// A part of the source line that the screen shows.
#[derive(Clone, Copy, Debug)]
struct Segment {
    display: usize,
    source: usize,
    len: usize,
}

impl Model {
    pub(crate) fn new(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            text: text.to_owned(),
            body: Body::parse(text),
            cursor: 0,
            anchor: 0,
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }

    /// The selected source, from the lower offset to the higher one. It is empty with no selection.
    pub(crate) fn selection(&self) -> Range<usize> {
        self.cursor.min(self.anchor)..self.cursor.max(self.anchor)
    }

    /// Whether the cursor is at the start of the selection.
    pub(crate) fn is_reversed(&self) -> bool {
        self.cursor < self.anchor
    }

    /// Moves the cursor to `offset`, rounded down to a char boundary.
    /// With `extend`, the selection keeps its other end.
    pub(crate) fn set_cursor(&mut self, offset: usize, extend: bool) {
        self.cursor = self.rope.floor_char_boundary(offset.min(self.text.len()));
        if !extend {
            self.anchor = self.cursor;
        }
    }

    pub(crate) fn select(&mut self, range: Range<usize>) {
        self.set_cursor(range.start, false);
        self.set_cursor(range.end, true);
    }

    /// Replaces the selection with `text` and puts the cursor after it.
    pub(crate) fn insert(&mut self, text: &str) {
        self.replace(self.selection(), text);
    }

    /// Replaces `range` with `text` and puts the cursor after it.
    pub(crate) fn replace(&mut self, range: Range<usize>, text: &str) {
        let end = self
            .rope
            .floor_char_boundary(range.end.min(self.text.len()));
        let start = self.rope.floor_char_boundary(range.start.min(end));
        self.rope.remove(start..end);
        self.rope.insert(start, text);
        self.text.replace_range(start..end, text);
        self.body = Body::parse(&self.text);
        self.set_cursor(start + text.len(), false);
    }

    /// Removes the selection, or the char before the cursor. Returns whether the text changed.
    pub(crate) fn backspace(&mut self) -> bool {
        let mut range = self.selection();
        if range.is_empty() {
            range.start = self.prev_char(self.cursor);
        }
        self.remove(range)
    }

    /// Removes the selection, or the char after the cursor. Returns whether the text changed.
    pub(crate) fn delete(&mut self) -> bool {
        let mut range = self.selection();
        if range.is_empty() {
            range.end = self.next_char(self.cursor);
        }
        self.remove(range)
    }

    fn remove(&mut self, range: Range<usize>) -> bool {
        if range.is_empty() {
            return false;
        }
        self.replace(range, "");
        true
    }

    /// Moves the cursor one char to the left. Without `extend`, a selection collapses to its start.
    pub(crate) fn move_left(&mut self, extend: bool) {
        let selection = self.selection();
        if !extend && !selection.is_empty() {
            self.set_cursor(selection.start, false);
        } else {
            self.set_cursor(self.prev_char(self.cursor), extend);
        }
    }

    /// Moves the cursor one char to the right. Without `extend`, a selection collapses to its end.
    pub(crate) fn move_right(&mut self, extend: bool) {
        let selection = self.selection();
        if !extend && !selection.is_empty() {
            self.set_cursor(selection.end, false);
        } else {
            self.set_cursor(self.next_char(self.cursor), extend);
        }
    }

    fn prev_char(&self, offset: usize) -> usize {
        if offset == 0 {
            0
        } else {
            self.rope.floor_char_boundary(offset - 1)
        }
    }

    fn next_char(&self, offset: usize) -> usize {
        if offset >= self.text.len() {
            self.text.len()
        } else {
            self.rope.ceil_char_boundary(offset + 1)
        }
    }

    /// The number of source lines. An empty text has one line.
    pub(crate) fn line_count(&self) -> usize {
        self.rope.len_lines(LF)
    }

    /// The line that holds `offset`.
    pub(crate) fn line_of(&self, offset: usize) -> usize {
        self.rope.byte_to_line_idx(offset.min(self.text.len()), LF)
    }

    /// The source of line `line`, without its line break.
    pub(crate) fn line_range(&self, line: usize) -> Range<usize> {
        let start = self.rope.line_to_byte_idx(line, LF);
        let end = if line + 1 < self.line_count() {
            self.rope.line_to_byte_idx(line + 1, LF) - 1
        } else {
            self.text.len()
        };
        start..end
    }

    /// Whether line `line` shows its syntax: it has the cursor or touches the selection.
    fn is_raw(&self, line: &Range<usize>) -> bool {
        let selection = self.selection();
        line.start <= selection.end && selection.start <= line.end
    }

    /// Line `line` as the screen shows it.
    pub(crate) fn display_line(&self, line: usize) -> DisplayLine {
        let source = self.line_range(line);
        let raw = self.is_raw(&source);
        let blocks = self.body.blocks();
        let block = blocks
            .partition_point(|b| b.range.start <= source.start)
            .checked_sub(1)
            .map(|ix| &blocks[ix])
            .filter(|b| b.range.end >= source.start);
        // Spans and hidden syntax never leave their top level block. So the search starts there.
        let from = block.map_or(source.start, |b| b.range.start);

        let spans = self.body.spans();
        let spans = &spans[spans.partition_point(|s| s.range.start < from)
            ..spans.partition_point(|s| s.range.start < source.end)];
        let spans: Vec<_> = spans
            .iter()
            .filter(|s| s.range.end > source.start)
            .collect();
        let hidden = self.body.hidden();
        let hidden: Vec<_> = hidden[hidden.partition_point(|h| h.start < from)
            ..hidden.partition_point(|h| h.start < source.end)]
            .iter()
            .filter(|h| h.end > source.start)
            .map(|h| h.start.max(source.start)..h.end.min(source.end))
            .collect();

        let mut cuts = vec![source.start, source.end];
        for span in &spans {
            cuts.extend([span.range.start, span.range.end]);
        }
        for range in &hidden {
            cuts.extend([range.start, range.end]);
        }
        cuts.retain(|cut| source.contains(cut) || *cut == source.end);
        cuts.sort_unstable();
        cuts.dedup();

        let mut display = DisplayLine {
            text: String::new(),
            runs: Vec::new(),
            heading: block.and_then(heading_level),
            files: self.files_in(&source),
            raw,
            source: source.clone(),
            segments: Vec::new(),
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
            display.push(piece, look, &self.text);
        }
        display
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

    /// The source offset as a UTF-16 offset, for the platform text input.
    pub(crate) fn offset_to_utf16(&self, offset: usize) -> usize {
        self.rope.byte_to_utf16_idx(offset.min(self.text.len()))
    }

    /// The UTF-16 offset as a source offset, for the platform text input.
    pub(crate) fn utf16_to_offset(&self, offset: usize) -> usize {
        self.rope
            .utf16_to_byte_idx(offset.min(self.rope.len_utf16()))
    }
}

fn heading_level(block: &Block) -> Option<u8> {
    match block.kind {
        BlockKind::Heading { level } => Some(level),
        _ => None,
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

impl DisplayLine {
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

    /// The source of the line, without its line break.
    pub(crate) fn source(&self) -> Range<usize> {
        self.source.clone()
    }

    /// The source offset for a screen offset. Between two visible parts it picks the later one.
    pub(crate) fn to_source(&self, display: usize) -> usize {
        let Some(ix) = self
            .segments
            .partition_point(|s| s.display <= display)
            .checked_sub(1)
        else {
            return self.source.start;
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn model_at(text: &str, cursor: usize) -> Model {
        let mut model = Model::new(text);
        model.set_cursor(cursor, false);
        model
    }

    #[test]
    fn typing_replaces_the_selection() {
        let mut model = Model::new("one two");
        model.select(4..7);
        model.insert("three");
        assert_eq!(model.text(), "one three");
        assert_eq!(model.cursor(), 9);
        assert!(model.selection().is_empty());
    }

    #[test]
    fn backspace_and_delete_remove_whole_chars() {
        let mut model = model_at("aé🙂b", 7);
        assert!(model.backspace());
        assert_eq!(model.text(), "aéb");
        assert_eq!(model.cursor(), 3);
        assert!(model.backspace());
        assert_eq!(model.text(), "ab");
        assert!(model.delete());
        assert_eq!(model.text(), "a");
        assert!(!model.delete());
        model.set_cursor(0, false);
        assert!(!model.backspace());
    }

    #[test]
    fn arrows_collapse_a_selection() {
        let mut model = Model::new("abcdef");
        model.select(2..4);
        model.move_left(false);
        assert_eq!(model.selection(), 2..2);
        model.move_right(true);
        model.move_right(true);
        assert_eq!(model.selection(), 2..4);
        model.move_right(false);
        assert_eq!(model.selection(), 4..4);
    }

    #[test]
    fn lines_have_no_line_break() {
        let model = Model::new("one\ntwo\n");
        assert_eq!(model.line_count(), 3);
        assert_eq!(model.line_range(0), 0..3);
        assert_eq!(model.line_range(1), 4..7);
        assert_eq!(model.line_range(2), 8..8);
        assert_eq!(model.line_of(3), 0);
        assert_eq!(model.line_of(4), 1);
    }

    #[test]
    fn syntax_hides_away_from_the_cursor_line() {
        let model = model_at("a **b** c\n\nnext", 12);
        let line = model.display_line(0);
        assert!(!line.raw);
        assert_eq!(line.text, "a b c");
        assert_eq!(line.runs.len(), 3);
        assert!(line.runs[1].look.strong);
        assert_eq!(line.to_source(2), 4);
        assert_eq!(line.to_source(3), 7);
        assert_eq!(line.to_display(4), 2);
        assert_eq!(line.to_display(2), 2);
    }

    #[test]
    fn syntax_shows_on_the_cursor_line() {
        let model = model_at("a **b** c\n\nnext", 1);
        let line = model.display_line(0);
        assert!(line.raw);
        assert_eq!(line.text, "a **b** c");
        assert!(line.runs[1].look.syntax);
        assert_eq!(line.to_source(5), 5);
    }

    #[test]
    fn a_selection_shows_syntax_on_each_line_it_touches() {
        let mut model = Model::new("**a**\n**b**\n**c**");
        model.select(2..8);
        assert!(model.display_line(0).raw);
        assert!(model.display_line(1).raw);
        assert!(!model.display_line(2).raw);
    }

    #[test]
    fn embedded_files_hide_and_are_listed() {
        let model = model_at("top\n\n![cat](cat.png)\n", 0);
        let line = model.display_line(2);
        assert_eq!(line.text, "");
        assert_eq!(line.files, ["cat.png"]);
        assert_eq!(line.to_source(0), line.source().start);
    }

    #[test]
    fn heading_lines_know_their_level() {
        let model = model_at("x\n\n## Plans\n", 0);
        let line = model.display_line(2);
        assert_eq!(line.heading, Some(2));
        assert_eq!(line.text, "Plans");
    }

    #[test]
    fn utf16_offsets_round_trip() {
        let model = Model::new("a🙂b");
        assert_eq!(model.offset_to_utf16(5), 3);
        assert_eq!(model.utf16_to_offset(3), 5);
    }

    fn text() -> impl Strategy<Value = String> {
        proptest::string::string_regex("([a-c é🙂*_=`#>\\[\\]()!\n-]){0,40}").unwrap()
    }

    #[derive(Clone, Debug)]
    enum Op {
        Insert(String),
        Backspace,
        Delete,
        Left(bool),
        Right(bool),
        Cursor(usize, bool),
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            text().prop_map(Op::Insert),
            Just(Op::Backspace),
            Just(Op::Delete),
            any::<bool>().prop_map(Op::Left),
            any::<bool>().prop_map(Op::Right),
            (0..60usize, any::<bool>()).prop_map(|(at, extend)| Op::Cursor(at, extend)),
        ]
    }

    proptest! {
        #[test]
        fn the_cursor_stays_on_a_char_boundary(start in text(), ops in prop::collection::vec(op(), 0..30)) {
            let mut model = Model::new(&start);
            for op in ops {
                match op {
                    Op::Insert(text) => model.insert(&text),
                    Op::Backspace => { model.backspace(); }
                    Op::Delete => { model.delete(); }
                    Op::Left(extend) => model.move_left(extend),
                    Op::Right(extend) => model.move_right(extend),
                    Op::Cursor(at, extend) => model.set_cursor(at, extend),
                }
                prop_assert!(model.text().is_char_boundary(model.cursor()));
                prop_assert!(model.text().is_char_boundary(model.selection().start));
                prop_assert_eq!(model.rope.to_string(), model.text());
            }
        }

        #[test]
        fn screen_lines_map_back_into_their_source(start in text(), at in 0..60usize) {
            let model = model_at(&start, at);
            for line in 0..model.line_count() {
                let display = model.display_line(line);
                let source = display.source();
                prop_assert_eq!(display.runs.iter().map(|r| r.len).sum::<usize>(), display.text.len());
                for offset in (0..=display.text.len()).filter(|o| display.text.is_char_boundary(*o)) {
                    let mapped = display.to_source(offset);
                    prop_assert!(source.start <= mapped && mapped <= source.end);
                    prop_assert!(model.text().is_char_boundary(mapped));
                    prop_assert_eq!(display.to_display(mapped), offset);
                }
            }
        }
    }
}
