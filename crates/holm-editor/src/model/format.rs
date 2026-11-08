//! The writing helps: format keys, headings, and lists.

use std::ops::Range;

use holm_markdown::{Block, BlockKind, Style};

use super::{Bias, Edit, Group, Model, map_offset};

/// An inline style that a format key adds or removes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Format {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Highlight,
    Code,
}

impl Format {
    /// The syntax forms of the style, the longest first.
    fn markers(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Bold => &[("**", "**"), ("__", "__")],
            Self::Italic => &[("*", "*"), ("_", "_")],
            Self::Underline => &[("<u>", "</u>")],
            Self::Strikethrough => &[("~~", "~~"), ("~", "~")],
            Self::Highlight => &[("==", "=="), ("<mark>", "</mark>")],
            Self::Code => &[("``", "``"), ("`", "`")],
        }
    }

    fn is(self, style: Style) -> bool {
        matches!(
            (self, style),
            (Self::Bold, Style::Strong)
                | (Self::Italic, Style::Emphasis)
                | (Self::Underline, Style::Underline)
                | (Self::Strikethrough, Style::Strikethrough)
                | (Self::Highlight, Style::Highlight(None))
                | (Self::Code, Style::Code)
        )
    }

    /// The form that Holm writes.
    fn written(self) -> (&'static str, &'static str) {
        match self {
            Self::Code => ("`", "`"),
            _ => self.markers()[0],
        }
    }

    fn open(self) -> &'static str {
        self.written().0
    }

    fn close(self) -> &'static str {
        self.written().1
    }
}

/// A styled span with the syntax around it.
struct Styled {
    text: Range<usize>,
    open: &'static str,
    close: &'static str,
}

/// The start of a list item line.
#[derive(Debug, PartialEq, Eq)]
struct ListLine {
    /// The quote markers before the indent, like `> `.
    quote: Range<usize>,
    /// The spaces before the marker.
    indent: Range<usize>,
    marker: Marker,
    /// Where the item text starts: after the marker, its spaces and the task box.
    content: usize,
    task: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Marker {
    Bullet(char),
    Number(u64, char),
}

/// The list item syntax at the start of `line`.
fn list_line(line: &str, start: usize) -> Option<ListLine> {
    let bytes = line.as_bytes();
    let mut at = 0;
    while bytes.get(at) == Some(&b'>') {
        at += 1;
        if bytes.get(at) == Some(&b' ') {
            at += 1;
        }
    }
    let quote = start..start + at;
    let indent_start = at;
    while bytes.get(at) == Some(&b' ') {
        at += 1;
    }
    let indent = start + indent_start..start + at;
    let marker = match bytes.get(at)? {
        b @ (b'-' | b'*' | b'+') => {
            at += 1;
            Marker::Bullet(char::from(*b))
        }
        b'0'..=b'9' => {
            let digits = bytes[at..]
                .iter()
                .take_while(|b| b.is_ascii_digit())
                .count();
            if digits > 9 {
                return None;
            }
            let number = line[at..at + digits].parse().ok()?;
            at += digits;
            let delimiter = match bytes.get(at)? {
                b @ (b'.' | b')') => char::from(*b),
                _ => return None,
            };
            at += 1;
            Marker::Number(number, delimiter)
        }
        _ => return None,
    };
    match bytes.get(at) {
        None => {}
        Some(b' ') => {
            while bytes.get(at) == Some(&b' ') {
                at += 1;
            }
        }
        Some(_) => return None,
    }
    let task = ["[ ] ", "[x] ", "[X] "]
        .iter()
        .any(|task| line[at..].starts_with(task));
    if task {
        at += 4;
    }
    Some(ListLine {
        quote,
        indent,
        marker,
        content: start + at,
        task,
    })
}

/// The ATX heading level of `line`, and the length of its `#`s and the spaces after them.
fn heading_prefix(line: &str) -> (u8, usize) {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    if !(1..=6).contains(&hashes) {
        return (0, 0);
    }
    let rest = &line[hashes..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return (0, 0);
    }
    let spaces = rest.bytes().take_while(|b| *b == b' ').count();
    #[allow(clippy::cast_possible_truncation, reason = "at most 6")]
    (hashes as u8, hashes + spaces)
}

impl Model {
    /// Adds or removes `format` around the selection, or at the cursor.
    /// Returns whether the text changed.
    pub(crate) fn toggle_format(&mut self, format: Format) -> bool {
        let selection = self.selection();
        if selection.is_empty() {
            return self.toggle_at_cursor(format);
        }
        let pieces = self.pieces(&selection);
        if pieces.is_empty() {
            return false;
        }
        let styled: Vec<_> = pieces.iter().map(|p| self.styled(format, p)).collect();
        let mut edits: Vec<Edit> = Vec::new();
        if styled.iter().all(Option::is_some) {
            for (piece, styled) in pieces.iter().zip(styled.into_iter().flatten()) {
                edits.push(if piece.start == styled.text.start {
                    (
                        styled.text.start - styled.open.len()..styled.text.start,
                        String::new(),
                    )
                } else {
                    (piece.start..piece.start, styled.close.to_owned())
                });
                edits.push(if piece.end == styled.text.end {
                    (
                        styled.text.end..styled.text.end + styled.close.len(),
                        String::new(),
                    )
                } else {
                    (piece.end..piece.end, styled.open.to_owned())
                });
            }
        } else {
            for (piece, styled) in pieces.iter().zip(&styled) {
                if styled.is_none() {
                    // The same style inside the piece cannot nest, so its markers go.
                    // A span at an end of the piece has its markers outside the piece.
                    let inside = if format == Format::Code {
                        Vec::new()
                    } else {
                        let wide = self.widen(piece);
                        self.styled_inside(format, &wide)
                    };
                    let start = inside.first().map_or(piece.start, |s| {
                        piece.start.min(s.text.start - s.open.len())
                    });
                    let end = inside
                        .last()
                        .map_or(piece.end, |s| piece.end.max(s.text.end + s.close.len()));
                    edits.push((start..start, format.open().to_owned()));
                    for inner in inside {
                        let open = inner.text.start - inner.open.len()..inner.text.start;
                        let close = inner.text.end..inner.text.end + inner.close.len();
                        edits.push((open, String::new()));
                        edits.push((close, String::new()));
                    }
                    edits.push((end..end, format.close().to_owned()));
                }
            }
        }
        let first = pieces[0].start;
        let last = pieces[pieces.len() - 1].end;
        let start = map_offset(&edits, first, Bias::After);
        let end = map_offset(&edits, last, Bias::Before);
        let ends = if self.is_reversed() {
            (end, start)
        } else {
            (start, end)
        };
        self.edit(edits, ends, Group::Alone, None)
    }

    /// Removes the format of the span around the cursor, or puts in an empty pair of markers.
    fn toggle_at_cursor(&mut self, format: Format) -> bool {
        let cursor = self.cursor;
        if let Some(styled) = self.styled(format, &(cursor..cursor)) {
            let edits = vec![
                (
                    styled.text.start - styled.open.len()..styled.text.start,
                    String::new(),
                ),
                (
                    styled.text.end..styled.text.end + styled.close.len(),
                    String::new(),
                ),
            ];
            let cursor = map_offset(&edits, cursor, Bias::After);
            return self.edit(edits, (cursor, cursor), Group::Alone, None);
        }
        let at = cursor + format.open().len();
        self.edit(
            vec![(
                cursor..cursor,
                format!("{}{}", format.open(), format.close()),
            )],
            (at, at),
            Group::Alone,
            None,
        )
    }

    /// The part of `selection` on each line it touches, without spaces at its ends.
    /// A list marker and a task box are never part of it. On a table row each cell is its own
    /// part, without its `|`. The `|---|` line of a table and the lines of a code block have
    /// no part.
    fn pieces(&self, selection: &Range<usize>) -> Vec<Range<usize>> {
        let first = self.line_of(selection.start);
        let last = self.line_of(selection.end);
        let mut pieces = Vec::new();
        for ix in first..=last {
            let line = self.line_range(ix);
            let chain = self.chain_at(self.probe(&line));
            if chain
                .iter()
                .any(|b| matches!(b.kind, BlockKind::Code { .. }))
            {
                continue;
            }
            if let Some(table) = chain
                .iter()
                .find(|b| matches!(b.kind, BlockKind::Table { .. }))
            {
                let row = table
                    .children
                    .iter()
                    .find(|row| line.start <= row.range.start && row.range.start <= line.end);
                for cell in row.map_or(&[][..], |row| &row.children[..]) {
                    let range =
                        selection.start.max(cell.range.start)..selection.end.min(cell.range.end);
                    pieces.extend(self.piece(range));
                }
                continue;
            }
            let content = self.list_item(ix).map_or(line.start, |item| item.content);
            pieces.extend(self.piece(selection.start.max(content)..selection.end.min(line.end)));
        }
        pieces
    }

    /// `range` without spaces and hidden syntax at its ends, or `None` when nothing is left.
    fn piece(&self, range: Range<usize>) -> Option<Range<usize>> {
        if range.start >= range.end {
            return None;
        }
        let text = &self.text[range.clone()];
        let start = range.start + (text.len() - text.trim_start().len());
        let end = range.end - (text.len() - text.trim_end().len());
        let piece = self.trim_syntax(start..end.max(start));
        (!piece.is_empty()).then_some(piece)
    }

    /// `range` without the hidden syntax at its ends, like the `**` of a bold word.
    fn trim_syntax(&self, range: Range<usize>) -> Range<usize> {
        let hidden = self.body.hidden();
        let mut start = range.start;
        let ix = hidden.partition_point(|h| h.start <= start);
        if let Some(h) = ix.checked_sub(1).map(|ix| &hidden[ix])
            && start < h.end
        {
            start = h.end;
        }
        let mut end = range.end;
        let ix = hidden.partition_point(|h| h.start < end);
        if let Some(h) = ix.checked_sub(1).map(|ix| &hidden[ix])
            && end <= h.end
        {
            end = h.start;
        }
        start..end.max(start)
    }

    /// The span of `format` that holds all of `range`, with its syntax.
    fn styled(&self, format: Format, range: &Range<usize>) -> Option<Styled> {
        let spans = self.body.spans();
        spans[..spans.partition_point(|s| s.range.start <= range.start)]
            .iter()
            .rev()
            .filter(|s| format.is(s.style) && range.end <= s.range.end)
            .find_map(|s| {
                format.markers().iter().find_map(|&(open, close)| {
                    let before = s.range.start.checked_sub(open.len())?;
                    let after = s.range.end + close.len();
                    let fits = self.text.get(before..s.range.start) == Some(open)
                        && self.text.get(s.range.end..after) == Some(close);
                    fits.then(|| Styled {
                        text: s.range.clone(),
                        open,
                        close,
                    })
                })
            })
    }

    /// `range` with the hidden syntax right before and right after it.
    fn widen(&self, range: &Range<usize>) -> Range<usize> {
        let hidden = self.body.hidden();
        let before = hidden.partition_point(|h| h.end < range.start);
        let start = hidden
            .get(before)
            .filter(|h| h.end == range.start)
            .map_or(range.start, |h| h.start);
        let after = hidden.partition_point(|h| h.start < range.end);
        let end = hidden
            .get(after)
            .filter(|h| h.start == range.end)
            .map_or(range.end, |h| h.end);
        start..end
    }

    /// The spans of `format` that lie in `range` with their syntax, in order, apart.
    fn styled_inside(&self, format: Format, range: &Range<usize>) -> Vec<Styled> {
        let spans = self.body.spans();
        let mut found: Vec<Styled> = Vec::new();
        for span in &spans[spans.partition_point(|s| s.range.start < range.start)
            ..spans.partition_point(|s| s.range.start < range.end)]
        {
            if !format.is(span.style) {
                continue;
            }
            let markers = format.markers().iter().find(|&&(open, close)| {
                span.range.start >= range.start + open.len()
                    && span.range.end + close.len() <= range.end
                    && self
                        .text
                        .get(span.range.start - open.len()..span.range.start)
                        == Some(open)
                    && self.text.get(span.range.end..span.range.end + close.len()) == Some(close)
            });
            let Some(&(open, close)) = markers else {
                continue;
            };
            let after = found
                .last()
                .map_or(range.start, |last| last.text.end + last.close.len());
            if span.range.start - open.len() >= after {
                found.push(Styled {
                    text: span.range.clone(),
                    open,
                    close,
                });
            }
        }
        found
    }

    /// Makes each line the selection touches a heading of `level`, or a plain line with 0.
    /// When every such line already has `level`, they become plain lines.
    /// Returns whether the text changed.
    pub(crate) fn toggle_heading(&mut self, level: u8) -> bool {
        let selection = self.selection();
        let first = self.line_of(selection.start);
        let last = self.line_of(selection.end);
        let lines: Vec<_> = (first..=last)
            .map(|line| self.line_range(line))
            .filter(|line| first == last || !line.is_empty())
            .map(|line| {
                let prefix = heading_prefix(&self.text[line.clone()]);
                (line, prefix)
            })
            .collect();
        let level = if lines.iter().all(|(_, (old, _))| *old == level) {
            0
        } else {
            level
        };
        let prefix = if level == 0 {
            String::new()
        } else {
            format!("{} ", "#".repeat(usize::from(level)))
        };
        let edits: Vec<Edit> = lines
            .iter()
            .map(|(line, (_, len))| (line.start..line.start + len, prefix.clone()))
            .collect();
        let anchor = map_offset(&edits, self.anchor, Bias::After);
        let cursor = map_offset(&edits, self.cursor, Bias::After);
        self.edit(edits, (anchor, cursor), Group::Alone, None)
    }

    /// The list item at the start of `line`, when the parse puts the line in a list item.
    fn list_item(&self, line: usize) -> Option<ListLine> {
        let range = self.line_range(line);
        let item = list_line(&self.text[range.clone()], range.start)?;
        let kinds = self.blocks_at(item.indent.end);
        let in_item = kinds.iter().any(|k| matches!(k, BlockKind::Item { .. }));
        let in_code = kinds
            .iter()
            .any(|k| matches!(k, BlockKind::Code { .. } | BlockKind::Html));
        (in_item && !in_code).then_some(item)
    }

    /// Enter: in a list item it starts the next item, and on an empty item it ends the list.
    /// Elsewhere it types a line break. Returns whether the text changed.
    pub(crate) fn newline(&mut self, continue_lists: bool, now: std::time::Instant) -> bool {
        let selection = self.selection();
        let line = self.line_of(selection.start);
        let item = continue_lists
            .then(|| self.list_item(line))
            .flatten()
            .filter(|item| selection.start >= item.content);
        let Some(item) = item else {
            return self.type_text("\n", now);
        };
        let range = self.line_range(line);
        if item.content == range.end && selection.end == range.end {
            let start = item.indent.start;
            return self.edit(
                vec![(start..range.end, String::new())],
                (start, start),
                Group::Alone,
                None,
            );
        }
        let marker = match item.marker {
            Marker::Bullet(bullet) => bullet.to_string(),
            Marker::Number(number, delimiter) => format!("{}{delimiter}", number + 1),
        };
        let next = format!(
            "\n{}{}{marker} {}",
            &self.text[item.quote.clone()],
            &self.text[item.indent.clone()],
            if item.task { "[ ] " } else { "" },
        );
        let cursor = selection.start + next.len();
        self.edit(
            vec![(selection, next)],
            (cursor, cursor),
            Group::Alone,
            None,
        )
    }

    /// Tab and Shift+Tab: moves each list item that the selection touches one level in or out.
    /// Returns `None` when the selection touches no list item.
    pub(crate) fn indent(&mut self, out: bool) -> Option<bool> {
        let selection = self.selection();
        let first = self.line_of(selection.start);
        let last = self.line_of(selection.end);
        let mut edits: Vec<Edit> = Vec::new();
        for line in first..=last {
            let Some(item) = self.list_item(line) else {
                continue;
            };
            let width = item.indent.len();
            let new_width = if out {
                self.parent_indent(line, width)
            } else {
                self.child_indent(line, &item)
            };
            edits.push((item.indent.clone(), " ".repeat(new_width)));
        }
        if edits.is_empty() {
            return None;
        }
        let anchor = map_offset(&edits, self.anchor, Bias::After);
        let cursor = map_offset(&edits, self.cursor, Bias::After);
        Some(self.edit(edits, (anchor, cursor), Group::Alone, None))
    }

    /// The earlier list items in the same top level block as `line`, nearest first.
    fn items_before(&self, line: usize) -> impl Iterator<Item = ListLine> + '_ {
        let start = self.line_range(line).start;
        let blocks = self.body.blocks();
        let top = blocks[..blocks.partition_point(|b| b.range.start <= start)]
            .last()
            .map_or(start, |b| b.range.start);
        let top = self.line_of(top);
        (top..line).rev().filter_map(|line| self.list_item(line))
    }

    /// The indent that makes the item on `line` a child of the item above it.
    fn child_indent(&self, line: usize, item: &ListLine) -> usize {
        let width = item.indent.len();
        self.items_before(line)
            .find(|above| above.indent.len() <= width)
            .filter(|above| above.indent.len() == width)
            .map_or(width + (item.content - item.indent.end), |above| {
                above.content - above.indent.start
            })
    }

    /// Checks or clears the task box with the `[ ]` source at `range`, as one undo step.
    /// The cursor stays. Returns whether the text changed.
    pub(crate) fn toggle_task(&mut self, range: &Range<usize>) -> bool {
        self.toggle_task_boxes(std::slice::from_ref(range))
    }

    /// Checks or clears each task box on the lines that the selection touches.
    /// Returns whether the text changed.
    pub(crate) fn toggle_tasks(&mut self) -> bool {
        let selection = self.selection();
        let lines = self.line_range(self.line_of(selection.start)).start
            ..self.line_range(self.line_of(selection.end)).end;
        let mut boxes = Vec::new();
        collect_tasks(self.body.blocks(), &lines, &mut boxes);
        self.toggle_task_boxes(&boxes)
    }

    fn toggle_task_boxes(&mut self, boxes: &[Range<usize>]) -> bool {
        let edits: Vec<Edit> = boxes
            .iter()
            .filter(|range| {
                range.len() == 3 && self.text.get(range.start..range.start + 1) == Some("[")
            })
            .map(|range| {
                let mark = &self.text[range.start + 1..range.start + 2];
                let new = if mark == " " { "x" } else { " " };
                (range.start + 1..range.start + 2, new.to_owned())
            })
            .collect();
        self.edit(edits, self.ends(), Group::Alone, None)
    }

    /// The indent of the parent of the item on `line`, or 0 at the top level.
    fn parent_indent(&self, line: usize, width: usize) -> usize {
        self.items_before(line)
            .find(|above| above.indent.len() < width)
            .map_or(0, |above| above.indent.len())
    }
}

/// The `[ ]` sources of the task items that start in `lines`, in order.
fn collect_tasks(blocks: &[Block], lines: &Range<usize>, boxes: &mut Vec<Range<usize>>) {
    for block in blocks
        .iter()
        .filter(|b| b.range.start <= lines.end && lines.start <= b.range.end)
    {
        if let BlockKind::Item { task: Some(task) } = &block.kind
            && lines.start <= task.range.start
            && task.range.start <= lines.end
        {
            boxes.push(task.range.clone());
        }
        collect_tasks(&block.children, lines, boxes);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn model_with(text: &str, selection: Range<usize>) -> Model {
        let mut model = Model::new(text);
        model.select(selection);
        model
    }

    fn toggled(text: &str, selection: Range<usize>, format: Format) -> (String, Range<usize>) {
        let mut model = model_with(text, selection);
        model.toggle_format(format);
        (model.text().to_owned(), model.selection())
    }

    #[test]
    fn a_format_key_wraps_the_selection() {
        assert_eq!(
            toggled("a word", 2..6, Format::Bold),
            ("a **word**".into(), 4..8)
        );
        assert_eq!(
            toggled("a word", 2..6, Format::Underline),
            ("a <u>word</u>".into(), 5..9)
        );
        assert_eq!(
            toggled("a word", 2..6, Format::Highlight),
            ("a ==word==".into(), 4..8)
        );
    }

    #[test]
    fn a_format_key_joins_the_same_style_inside_the_selection() {
        let text = "Some ==big== ideas, a ==soft== mark.";
        let (new, selection) = toggled(text, 0..text.len(), Format::Highlight);
        assert_eq!(new, "==Some big ideas, a soft mark.==");
        assert_eq!(&new[selection], "Some big ideas, a soft mark.");
        assert_eq!(toggled("a **b** c", 0..9, Format::Bold).0, "**a b c**");
        assert_eq!(toggled("a *b* _c_ d", 0..11, Format::Italic).0, "*a b c d*");
        assert_eq!(
            toggled("a <u>b</u> c", 0..12, Format::Underline).0,
            "<u>a b c</u>"
        );
        assert_eq!(
            toggled("a ~~b~~ c", 0..9, Format::Strikethrough).0,
            "~~a b c~~"
        );
        // Other styles stay, and code keeps all text inside it as it is.
        assert_eq!(
            toggled("a **b** ==c==", 0..13, Format::Highlight).0,
            "==a **b** c=="
        );
        assert_eq!(toggled("a `b` c", 0..7, Format::Code).0, "`a `b` c`");
    }

    #[test]
    fn a_format_key_on_a_table_styles_each_cell() {
        let text = "| a | b c |\n|---|---|\n| 1 | 2 |\n";
        let (new, _) = toggled(text, 0..text.len(), Format::Highlight);
        assert_eq!(new, "| ==a== | ==b c== |\n|---|---|\n| ==1== | ==2== |\n");
        let (new, _) = toggled(text, 2..7, Format::Bold);
        assert_eq!(new, "| **a** | **b** c |\n|---|---|\n| 1 | 2 |\n");
    }

    #[test]
    fn a_format_key_skips_code_blocks() {
        let text = "a\n```\ncode\n```\nb";
        let (new, _) = toggled(text, 0..text.len(), Format::Bold);
        assert_eq!(new, "**a**\n```\ncode\n```\n**b**");
        assert!(!model_with(text, 6..10).toggle_format(Format::Bold));
    }

    #[test]
    fn a_second_format_key_removes_the_joined_style() {
        let mut model = model_with("Some ==big== idea", 0..17);
        model.toggle_format(Format::Highlight);
        assert_eq!(model.text(), "==Some big idea==");
        model.toggle_format(Format::Highlight);
        assert_eq!(model.text(), "Some big idea");
        model.undo();
        model.undo();
        assert_eq!(model.text(), "Some ==big== idea");
    }

    #[test]
    fn a_format_key_skips_list_markers_and_task_boxes() {
        let text = "- [ ] one\n- [x] two\n";
        let (new, _) = toggled(text, 0..text.len(), Format::Underline);
        assert_eq!(new, "- [ ] <u>one</u>\n- [x] <u>two</u>\n");
        let (new, _) = toggled("1. one", 0..6, Format::Bold);
        assert_eq!(new, "1. **one**");
        assert!(!model_with("- one", 0..2).toggle_format(Format::Bold));
    }

    #[test]
    fn a_format_key_removes_the_syntax_around_the_selection() {
        assert_eq!(
            toggled("a **word**", 4..8, Format::Bold),
            ("a word".into(), 2..6)
        );
        assert_eq!(
            toggled("a _word_", 3..7, Format::Italic),
            ("a word".into(), 2..6)
        );
        assert_eq!(
            toggled("a `code`", 3..7, Format::Code),
            ("a code".into(), 2..6)
        );
    }

    #[test]
    fn a_format_key_splits_a_span_around_a_part() {
        assert_eq!(
            toggled("**bold**", 3..5, Format::Bold),
            ("**b**ol**d**".into(), 5..7)
        );
        assert_eq!(
            toggled("<u>bold</u>", 3..5, Format::Underline),
            ("bo<u>ld</u>".into(), 0..2)
        );
    }

    #[test]
    fn a_format_key_skips_spaces_and_works_on_each_line() {
        assert_eq!(
            toggled("one \ntwo", 0..8, Format::Bold),
            ("**one** \n**two**".into(), 2..14)
        );
        assert_eq!(
            toggled("**one**\n**two**", 0..15, Format::Bold),
            ("one\ntwo".into(), 0..7)
        );
    }

    #[test]
    fn a_format_key_skips_the_syntax_at_the_ends_of_the_selection() {
        assert_eq!(toggled("# **a**", 0..7, Format::Bold), ("# a".into(), 2..3));
    }

    #[test]
    fn a_format_key_with_no_selection_adds_a_pair_or_removes_the_span() {
        assert_eq!(
            toggled("ab", 1..1, Format::Strikethrough),
            ("a~~~~b".into(), 3..3)
        );
        assert_eq!(
            toggled("**bold**", 4..4, Format::Bold),
            ("bold".into(), 2..2)
        );
    }

    #[test]
    fn a_heading_key_sets_and_removes_the_level() {
        let mut model = model_with("Plans", 2..2);
        model.toggle_heading(2);
        assert_eq!(model.text(), "## Plans");
        assert_eq!(model.cursor(), 5);
        model.toggle_heading(1);
        assert_eq!(model.text(), "# Plans");
        model.toggle_heading(1);
        assert_eq!(model.text(), "Plans");
        assert_eq!(model.cursor(), 2);
        let mut model = model_with("a\n\n# b", 0..6);
        model.toggle_heading(3);
        assert_eq!(model.text(), "### a\n\n### b");
    }

    fn enter(text: &str, at: usize) -> (String, usize) {
        let mut model = model_with(text, at..at);
        model.newline(true, Instant::now());
        (model.text().to_owned(), model.cursor())
    }

    #[test]
    fn enter_continues_a_list() {
        assert_eq!(enter("- one", 5), ("- one\n- ".into(), 8));
        assert_eq!(enter("9. one", 6), ("9. one\n10. ".into(), 11));
        assert_eq!(enter("- [x] done", 10), ("- [x] done\n- [ ] ".into(), 17));
        assert_eq!(enter("> * quote", 9), ("> * quote\n> * ".into(), 14));
        assert_eq!(enter("- a\n  - b", 9), ("- a\n  - b\n  - ".into(), 14));
        assert_eq!(enter("- onetwo", 5), ("- one\n- two".into(), 8));
    }

    #[test]
    fn enter_on_an_empty_item_ends_the_list() {
        assert_eq!(enter("- one\n- ", 8), ("- one\n".into(), 6));
    }

    #[test]
    fn enter_outside_a_list_is_a_line_break() {
        assert_eq!(enter("text", 4), ("text\n".into(), 5));
        assert_eq!(enter("```\n- a\n```", 7), ("```\n- a\n\n```".into(), 8));
        assert_eq!(enter("- one", 1), ("-\n one".into(), 2));
    }

    #[test]
    fn enter_without_the_list_help_is_a_line_break() {
        let mut model = model_with("- one", 5..5);
        model.newline(false, Instant::now());
        assert_eq!(model.text(), "- one\n");
    }

    fn indented(text: &str, selection: Range<usize>, out: bool) -> Option<String> {
        let mut model = model_with(text, selection);
        model.indent(out)?;
        Some(model.text().to_owned())
    }

    #[test]
    fn tab_makes_an_item_a_child_of_the_item_above() {
        assert_eq!(indented("- a\n- b", 6..6, false), Some("- a\n  - b".into()));
        assert_eq!(
            indented("10. a\n11. b", 8..8, false),
            Some("10. a\n    11. b".into())
        );
        assert_eq!(
            indented("- a\n  - b\n- c", 12..12, false),
            Some("- a\n  - b\n  - c".into())
        );
    }

    #[test]
    fn shift_tab_moves_an_item_to_its_parent_level() {
        assert_eq!(indented("- a\n  - b", 9..9, true), Some("- a\n- b".into()));
        assert_eq!(indented("- a", 1..1, true), Some("- a".into()));
    }

    #[test]
    fn tab_moves_each_selected_item() {
        assert_eq!(
            indented("- a\n- b\n- c", 4..11, false),
            Some("- a\n  - b\n  - c".into())
        );
    }

    #[test]
    fn tab_outside_a_list_is_not_an_indent() {
        assert_eq!(indented("text", 0..0, false), None);
    }

    #[test]
    fn list_lines_are_parsed() {
        assert_eq!(
            list_line("  1) [ ] x", 10),
            Some(ListLine {
                quote: 10..10,
                indent: 10..12,
                marker: Marker::Number(1, ')'),
                content: 19,
                task: true,
            })
        );
        assert_eq!(list_line("-x", 0), None);
        assert_eq!(list_line("text", 0), None);
        assert!(list_line("-", 0).is_some());
    }
}
