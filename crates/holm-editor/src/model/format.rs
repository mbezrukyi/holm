//! The writing helps: format keys, headings, and lists.

use std::ops::Range;

use holm_markdown::{Block, BlockKind, Span, Style};

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
        let at_cursor = selection.is_empty();
        let mut pieces = if at_cursor {
            // At the cursor in inline code, the style goes around the whole code.
            match self
                .code_span_at(self.cursor)
                .filter(|_| format != Format::Code)
            {
                Some(span) => vec![span],
                None => return self.toggle_at_cursor(format),
            }
        } else {
            self.pieces(&selection)
        };
        // Markers inside inline code show as plain text, so an end in code moves out of it.
        if format != Format::Code {
            for piece in &mut pieces {
                let start = self
                    .code_span_at(piece.start)
                    .map_or(piece.start, |s| s.start);
                let end = self.code_span_at(piece.end).map_or(piece.end, |s| s.end);
                *piece = start.min(piece.start)..end.max(piece.end);
            }
        }
        if pieces.is_empty() {
            return false;
        }
        let styled: Vec<_> = pieces.iter().map(|p| self.styled(format, p)).collect();
        let mut edits: Vec<Edit> = Vec::new();
        if styled.iter().all(Option::is_some) {
            for (piece, styled) in pieces.iter().zip(styled.into_iter().flatten()) {
                self.cut_out(&styled, piece, &mut edits);
            }
        } else {
            for (piece, styled) in pieces.iter().zip(&styled) {
                if styled.is_none() {
                    self.wrap(format, piece, &mut edits);
                }
            }
        }
        let first = pieces[0].start;
        let last = pieces[pieces.len() - 1].end;
        let start = map_offset(&edits, first, Bias::After);
        let end = map_offset(&edits, last, Bias::Before);
        let ends = if at_cursor {
            let cursor = map_offset(&edits, self.cursor, Bias::After);
            (cursor, cursor)
        } else if self.is_reversed() {
            (end, start)
        } else {
            (start, end)
        };
        self.edit(edits, ends, Group::Alone, None)
    }

    /// Removes the style of `styled` from `piece`, a part of its text. Where a cut falls inside
    /// another span in `styled`, that span closes before the cut and opens again after it.
    fn cut_out(&self, styled: &Styled, piece: &Range<usize>, edits: &mut Vec<Edit>) {
        let inside = |s: &Span| {
            styled.text.start <= s.range.start
                && s.range.end <= styled.text.end
                && s.range != styled.text
        };
        // A marker cannot close after a space or open before one, so the cuts move past spaces.
        let text = &self.text;
        let start = styled.text.start + text[styled.text.start..piece.start].trim_end().len();
        let end = piece.end
            + (text[piece.end..styled.text.end].len()
                - text[piece.end..styled.text.end].trim_start().len());
        let mut piece = start..end;
        // Syntax of inner spans at the ends of the text, like the `**` in `***b***`, does not
        // count. A piece that reaches it reaches the end of the span.
        let core = self.trim_syntax(styled.text.clone());
        let (before, after) = loop {
            let before = (piece.start > core.start)
                .then(|| self.split_at(piece.start, styled.close, &inside))
                .transpose();
            let after = (piece.end < core.end)
                .then(|| self.split_at(piece.end, styled.open, &inside))
                .transpose();
            match (before, after) {
                (Ok(before), Ok(after)) => break (before, after),
                (before, after) => {
                    // A span that cannot be split leaves the style as a whole.
                    for grow in [before.err(), after.err()].into_iter().flatten() {
                        piece.start = piece.start.min(grow.start).max(styled.text.start);
                        piece.end = piece.end.max(grow.end).min(styled.text.end);
                    }
                }
            }
        };
        match before {
            Some((closes, opens)) => {
                edits.push((piece.start..piece.start, closes + styled.close));
                let next = self.after_spaces(piece.start);
                edits.push((next..next, opens));
            }
            None => edits.push((
                styled.text.start - styled.open.len()..styled.text.start,
                String::new(),
            )),
        }
        match after {
            Some((closes, opens)) => {
                let last = self.before_spaces(piece.end);
                edits.push((last..last, closes));
                edits.push((piece.end..piece.end, styled.open.to_owned() + &opens));
            }
            None => edits.push((
                styled.text.end..styled.text.end + styled.close.len(),
                String::new(),
            )),
        }
    }

    /// Puts `format` around `piece`. Where an end falls inside a span of another style, that
    /// span closes before the new marker and opens again after it.
    fn wrap(&self, format: Format, piece: &Range<usize>, edits: &mut Vec<Edit>) {
        // A span of the same style that the piece cuts joins the new span.
        let mut piece = if format == Format::Code {
            piece.clone()
        } else {
            self.grow_to_style(format, piece)
        };
        let (range, open, close) = loop {
            // The new markers go around the syntax of the spans in the piece.
            let range = self.around_syntax(&piece);
            let crosses = |s: &Span| !(s.range.start <= range.start && range.end <= s.range.end);
            let open = self.split_at(range.start, format.open(), &crosses);
            let close = self.split_at(range.end, format.close(), &crosses);
            match (open, close) {
                (Ok(open), Ok(close)) => break (range, open, close),
                (open, close) => {
                    // A span that cannot be split goes into the new span as a whole.
                    for grow in [open.err(), close.err()].into_iter().flatten() {
                        piece.start = piece.start.min(grow.start);
                        piece.end = piece.end.max(grow.end);
                    }
                }
            }
        };
        // The same style inside the piece cannot nest, so its markers go.
        let inside = if format == Format::Code {
            Vec::new()
        } else {
            self.styled_inside(format, &range)
        };
        // Syntax cannot open before a space or close after one. So split spans close before
        // the spaces in front of the new span, and open again after the spaces behind it.
        let last = self.before_spaces(range.start);
        edits.push((last..last, open.0));
        edits.push((range.start..range.start, format.open().to_owned() + &open.1));
        for inner in inside {
            let open = inner.text.start - inner.open.len()..inner.text.start;
            let close = inner.text.end..inner.text.end + inner.close.len();
            edits.push((open, String::new()));
            edits.push((close, String::new()));
        }
        edits.push((range.end..range.end, close.0 + format.close()));
        let next = self.after_spaces(range.end);
        edits.push((next..next, close.1));
    }

    /// The offset before the spaces and tabs that end right at `at`.
    fn before_spaces(&self, at: usize) -> usize {
        at - self.text.as_bytes()[..at]
            .iter()
            .rev()
            .take_while(|b| matches!(b, b' ' | b'\t'))
            .count()
    }

    /// The offset after the spaces and tabs that start right at `at`.
    fn after_spaces(&self, at: usize) -> usize {
        at + self.text.as_bytes()[at..]
            .iter()
            .take_while(|b| matches!(b, b' ' | b'\t'))
            .count()
    }

    /// The syntax that closes and opens again the spans that `cuts` picks and that hold `at`,
    /// so `marker` can go at `at` between the two.
    ///
    /// It fails with the source of those spans when one of them cannot be split: a link, inline
    /// code, or markers that would touch the same marker char, like `**` and `*`.
    fn split_at(
        &self,
        at: usize,
        marker: &str,
        cuts: &dyn Fn(&Span) -> bool,
    ) -> Result<(String, String), Range<usize>> {
        let spans = self.body.spans();
        let from = self.chain_at(at).first().map_or(0, |b| b.range.start);
        let mut crossed: Vec<&Span> = spans[spans.partition_point(|s| s.range.start < from)
            ..spans.partition_point(|s| s.range.start < at)]
            .iter()
            .filter(|s| s.range.start < at && at < s.range.end && cuts(s))
            .collect();
        if crossed.is_empty() {
            return Ok((String::new(), String::new()));
        }
        // The outer span first.
        crossed.sort_by_key(|s| (s.range.start, std::cmp::Reverse(s.range.end)));
        let syntax: Vec<_> = crossed.iter().map(|s| self.span_syntax(s)).collect();
        // The source of all crossed spans, for a caller that cannot split them.
        let start = crossed.iter().map(|s| s.range.start).min().unwrap_or(at);
        let end = crossed.iter().map(|s| s.range.end).max().unwrap_or(at);
        let whole = syntax
            .iter()
            .flatten()
            .fold(start..end, |r, (open, close)| {
                r.start.min(open.start)..r.end.max(close.end)
            });
        if syntax.iter().any(Option::is_none) {
            return Err(whole);
        }
        let syntax: Vec<_> = syntax.into_iter().flatten().collect();
        let mut parts: Vec<&str> = syntax
            .iter()
            .rev()
            .map(|(_, close)| &self.text[close.clone()])
            .collect();
        parts.push(marker);
        parts.extend(syntax.iter().map(|(open, _)| &self.text[open.clone()]));
        let touch = parts.windows(2).any(|pair| {
            let last = pair[0].chars().next_back();
            last.is_some_and(|c| "*_~=".contains(c)) && last == pair[1].chars().next()
        });
        if touch {
            return Err(whole);
        }
        let at_marker = syntax.len();
        Ok((parts[..at_marker].concat(), parts[at_marker + 1..].concat()))
    }

    /// The opening and closing syntax of `span`, or `None` for a link or inline code.
    fn span_syntax(&self, span: &Span) -> Option<(Range<usize>, Range<usize>)> {
        let forms = match span.style {
            Style::Strong => Format::Bold.markers(),
            Style::Emphasis => Format::Italic.markers(),
            Style::Strikethrough => Format::Strikethrough.markers(),
            Style::Underline => Format::Underline.markers(),
            Style::Highlight(None) => Format::Highlight.markers(),
            Style::Highlight(Some(_)) => return self.tag_syntax(span, "</mark>"),
            Style::Color(_) => return self.tag_syntax(span, "</span>"),
            Style::Code | Style::Link => return None,
        };
        forms.iter().find_map(|&(open, close)| {
            let before = span.range.start.checked_sub(open.len())?;
            let after = span.range.end + close.len();
            let fits = self.text.get(before..span.range.start) == Some(open)
                && self.text.get(span.range.end..after) == Some(close);
            fits.then_some((before..span.range.start, span.range.end..after))
        })
    }

    /// The syntax of a span in an HTML tag with attributes, like `<span style="...">`.
    fn tag_syntax(&self, span: &Span, close: &str) -> Option<(Range<usize>, Range<usize>)> {
        let before = &self.text[..span.range.start];
        if !before.ends_with('>') {
            return None;
        }
        let open = before.rfind('<')?;
        self.text[span.range.end..].starts_with(close).then_some((
            open..span.range.start,
            span.range.end..span.range.end + close.len(),
        ))
    }

    /// Removes the format of the span around the cursor, or puts in an empty pair of markers.
    /// In a code block it does nothing.
    fn toggle_at_cursor(&mut self, format: Format) -> bool {
        let cursor = self.cursor;
        if self
            .blocks_at(cursor)
            .iter()
            .any(|kind| matches!(kind, BlockKind::Code { .. }))
        {
            return false;
        }
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
            let chain = self.blocks_of_line(&line);
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
        let piece = self.trim_syntax(self.trim_spaces(range));
        // Syntax can stand between the text and a space, like `== c`.
        let piece = self.trim_spaces(piece);
        (!piece.is_empty()).then_some(piece)
    }

    /// `range` without spaces at its ends.
    fn trim_spaces(&self, range: Range<usize>) -> Range<usize> {
        let text = &self.text[range.clone()];
        let start = range.start + (text.len() - text.trim_start().len());
        let end = range.end - (text.len() - text.trim_end().len());
        start..end.max(start)
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

    /// The source of the inline code span, with its backticks, whose text holds `offset`.
    fn code_span_at(&self, offset: usize) -> Option<Range<usize>> {
        let spans = self.body.spans();
        spans[..spans.partition_point(|s| s.range.start <= offset)]
            .iter()
            .rev()
            .find(|s| s.style == Style::Code && offset <= s.range.end)
            .map(|s| {
                let bytes = self.text.as_bytes();
                let open = bytes[..s.range.start]
                    .iter()
                    .rev()
                    .take_while(|&&b| b == b'`')
                    .count();
                let close = bytes[s.range.end..]
                    .iter()
                    .take_while(|&&b| b == b'`')
                    .count();
                s.range.start - open..s.range.end + close
            })
    }

    /// `range` with the syntax at its ends that belongs only to spans opened or closed in it.
    ///
    /// The parse joins touching syntax, like the `**==` after `==**b`. A span whose opening
    /// syntax is in `range` must close in it, so the end moves past that syntax. The start moves
    /// the same way. A span that holds exactly `range` keeps its syntax outside.
    fn around_syntax(&self, range: &Range<usize>) -> Range<usize> {
        let hidden = self.body.hidden();
        let spans = self.body.spans();
        let from = self
            .chain_at(range.start)
            .first()
            .map_or(0, |b| b.range.start);
        let spans = &spans[spans.partition_point(|s| s.range.start < from)..];
        let mut out = range.clone();

        // Syntax after the text: the closing markers of inner spans come before those of the
        // spans that hold all of `range`. The end moves past the inner ones only.
        let ix = hidden.partition_point(|h| h.end <= range.end);
        if let Some(h) = hidden.get(ix).filter(|h| h.start <= range.end) {
            let near = &spans[..spans.partition_point(|s| s.range.start <= h.end)];
            let opens = near
                .iter()
                .any(|s| h.start < s.range.start && s.range.start <= h.end);
            let (inner, outer): (Vec<_>, Vec<_>) = near
                .iter()
                .filter(|s| h.start <= s.range.end && s.range.end < h.end)
                .partition(|s| s.range.start > range.start);
            if !opens && !inner.is_empty() {
                out.end = outer.iter().map(|s| s.range.end).min().unwrap_or(h.end);
            }
        }

        // Syntax before the text: the opening markers of the spans that hold all of `range`
        // come before those of inner spans. The start moves before the inner ones only.
        let ix = hidden.partition_point(|h| h.end < range.start);
        if let Some(h) = hidden.get(ix).filter(|h| h.start < range.start) {
            let near = &spans[..spans.partition_point(|s| s.range.start <= h.end)];
            let closes = near
                .iter()
                .any(|s| h.start <= s.range.end && s.range.end < h.end);
            let (inner, outer): (Vec<_>, Vec<_>) = near
                .iter()
                .filter(|s| h.start < s.range.start && s.range.start <= h.end)
                .partition(|s| s.range.end < out.end);
            if !closes && !inner.is_empty() {
                out.start = outer.iter().map(|s| s.range.start).max().unwrap_or(h.start);
            }
        }
        out
    }

    /// `range` grown to hold, with their syntax, the spans of `format` whose text it touches.
    fn grow_to_style(&self, format: Format, range: &Range<usize>) -> Range<usize> {
        let spans = self.body.spans();
        let mut out = range.clone();
        loop {
            let grown = spans[..spans.partition_point(|s| s.range.start < out.end)]
                .iter()
                .filter(|s| format.is(s.style) && out.start < s.range.end)
                .filter_map(|s| {
                    format.markers().iter().find_map(|&(open, close)| {
                        let before = s.range.start.checked_sub(open.len())?;
                        let after = s.range.end + close.len();
                        let fits = self.text.get(before..s.range.start) == Some(open)
                            && self.text.get(s.range.end..after) == Some(close);
                        fits.then_some(before..after)
                    })
                })
                .fold(out.clone(), |out, span| {
                    out.start.min(span.start)..out.end.max(span.end)
                });
            if grown == out {
                return out;
            }
            out = grown;
        }
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
        assert!(!model_with(text, 8..8).toggle_format(Format::Bold));
        assert!(!model_with(text, 2..2).toggle_format(Format::Bold));
    }

    #[test]
    fn a_format_key_in_inline_code_styles_the_whole_code() {
        let text = "Also `inline code`, more";
        // An end of the selection inside the code moves out of it.
        assert_eq!(
            toggled(text, 6..19, Format::Highlight).0,
            "Also ==`inline code`,== more"
        );
        assert_eq!(
            toggled(text, 0..10, Format::Bold).0,
            "**Also `inline code`**, more"
        );
        // A selection or the cursor inside the code styles all of it, and a second key removes it.
        assert_eq!(
            toggled(text, 8..12, Format::Highlight).0,
            "Also ==`inline code`==, more"
        );
        let mut model = model_with(text, 9..9);
        assert!(model.toggle_format(Format::Bold));
        assert_eq!(model.text(), "Also **`inline code`**, more");
        assert_eq!(model.selection(), 11..11);
        assert!(model.toggle_format(Format::Bold));
        assert_eq!(model.text(), text);
        // The code key still removes the code.
        let mut model = model_with(text, 9..9);
        assert!(model.toggle_format(Format::Code));
        assert_eq!(model.text(), "Also inline code, more");
    }

    #[test]
    fn new_markers_go_around_the_syntax_of_inner_spans() {
        for (text, want) in [
            ("x ==**b**==", "**x ==b==**"),
            ("x **b**==c==", "**x b==c==**"),
            ("x ==b==", "**x ==b==**"),
            ("==b== x", "**==b== x**"),
            ("a ==**b**== c", "**a ==b== c**"),
        ] {
            assert_eq!(toggled(text, 0..text.len(), Format::Bold).0, want, "{text}");
        }
        // A span of the same style that the selection cuts joins the new span.
        assert_eq!(
            toggled("A tip with **strong** text.", 2..15, Format::Bold).0,
            "A **tip with strong** text."
        );
        assert_eq!(
            toggled("A **tip with** strong", 8..21, Format::Bold).0,
            "A **tip with strong**"
        );
        assert_eq!(
            toggled("> ==A tip with **strong** text.==", 6..19, Format::Bold).0,
            "> ==A **tip with strong** text.=="
        );
        // The markers of a span that holds the selection stay outside the new markers.
        assert_eq!(
            toggled("**x b==c==**", 2..8, Format::Highlight).0,
            "**==x bc==**"
        );
        assert_eq!(
            toggled("**==a==b c**", 4..10, Format::Highlight).0,
            "**==ab c==**"
        );
        // Syntax and a space at the start of the selection stay outside the new markers.
        assert_eq!(toggled("==a b== c", 5..9, Format::Bold).0, "==a b== **c**");
    }

    #[test]
    fn a_cut_splits_the_spans_it_crosses() {
        for (text, selected, format, want) in [
            (
                "==A **tip with strong** text.==",
                "ip with st",
                Format::Highlight,
                "==A **t**==**ip with st**==**rong** text.==",
            ),
            (
                "A **tip with strong** text",
                "A **t",
                Format::Highlight,
                "==A **t**==**ip with strong** text",
            ),
            (
                "A **tip with strong** text",
                "A **t",
                Format::Underline,
                "<u>A **t**</u>**ip with strong** text",
            ),
            (
                "a <span style=\"color: #c4553a\">red text</span> b",
                "a <span style=\"color: #c4553a\">red",
                Format::Bold,
                "**a <span style=\"color: #c4553a\">red</span>** <span style=\"color: #c4553a\">text</span> b",
            ),
            ("**==a b c==**", "b", Format::Highlight, "**==a== b ==c==**"),
            (
                "==A t**ip with strong** text.==",
                "with ",
                Format::Highlight,
                "==A t**ip**== **with** ==**strong** text.==",
            ),
        ] {
            let start = text.find(selected).unwrap_or(0);
            let got = toggled(text, start..start + selected.len(), format).0;
            assert_eq!(got, want, "{text}");
        }
    }

    #[test]
    fn a_span_that_cannot_be_split_goes_in_whole() {
        // A link, and `*` next to `**`, cannot be split.
        assert_eq!(
            toggled("a [link text](x) b", 0..7, Format::Bold).0,
            "**a [link text](x)** b"
        );
        assert_eq!(
            toggled("A **tip with strong** text", 0..5, Format::Italic).0,
            "*A **tip with strong*** text"
        );
    }

    #[test]
    fn a_format_key_removes_a_style_around_other_syntax() {
        let mut model = model_with("Also strong.", 0..12);
        for format in [Format::Highlight, Format::Bold, Format::Italic] {
            model.toggle_format(format);
        }
        assert_eq!(model.text(), "==***Also strong.***==");
        model.toggle_format(Format::Italic);
        assert_eq!(model.text(), "==**Also strong.**==");
        model.toggle_format(Format::Bold);
        assert_eq!(model.text(), "==Also strong.==");
        model.toggle_format(Format::Highlight);
        assert_eq!(model.text(), "Also strong.");
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

    const STYLES: [Format; 5] = [
        Format::Bold,
        Format::Italic,
        Format::Underline,
        Format::Strikethrough,
        Format::Highlight,
    ];

    proptest::proptest! {
        /// Each key pressed twice on the same selection, in any order, gives back the text.
        #[test]
        fn keys_pressed_twice_give_back_the_text(
            words in proptest::collection::vec("[a-z]{1,5}", 1..5),
            keys in proptest::collection::vec(0..STYLES.len(), 1..6),
        ) {
            let text = words.join(" ");
            let mut model = model_with(&text, 0..text.len());
            for key in keys.iter().chain(&keys) {
                model.toggle_format(STYLES[*key]);
            }
            proptest::prop_assert_eq!(model.text(), text.as_str());
        }
    }
}
