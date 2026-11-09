//! The writing helps: format keys, headings, and lists.

use std::ops::Range;

pub(crate) use holm_markdown::inline::Format;
use holm_markdown::inline::restyle;
use holm_markdown::{Block, BlockKind};

use super::{Edit, Group, Model, map_offset};

impl Model {
    /// Adds or removes the style of `format` on the selection, or at the cursor.
    /// Returns whether the text changed.
    pub(crate) fn toggle_format(&mut self, format: Format) -> bool {
        let Some(edit) = restyle(&self.text, &self.body, self.selection(), format) else {
            return false;
        };
        let ends = if self.is_reversed() {
            (edit.selection.end, edit.selection.start)
        } else {
            (edit.selection.start, edit.selection.end)
        };
        self.edit(vec![(edit.range, edit.text)], ends, Group::Alone, None)
    }
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
        let anchor = map_offset(&edits, self.anchor);
        let cursor = map_offset(&edits, self.cursor);
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
        let anchor = map_offset(&edits, self.anchor);
        let cursor = map_offset(&edits, self.cursor);
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
    fn a_format_key_wraps_the_selection_as_one_undo_step() {
        assert_eq!(
            toggled("a word", 2..6, Format::Bold),
            ("a **word**".into(), 4..8)
        );
        let mut model = Model::new("a word");
        model.set_cursor(6, false);
        model.set_cursor(2, true);
        model.toggle_format(Format::Underline);
        assert_eq!(model.text(), "a <u>word</u>");
        assert_eq!((model.cursor(), model.selection()), (5, 5..9));
        model.undo();
        assert_eq!(model.text(), "a word");
    }

    #[test]
    fn format_keys_keep_nested_styles_valid() {
        assert_eq!(
            toggled("**==Writ==**==e the report==", 4..8, Format::Bold).0,
            "==Write the report=="
        );
        assert_eq!(
            toggled("A **tip with strong** text", 0..5, Format::Italic).0,
            "*A* ***t*ip with strong** text"
        );
    }

    #[test]
    fn a_format_key_with_no_selection_takes_the_word_or_adds_a_pair() {
        let mut model = model_with("one two", 1..1);
        model.toggle_format(Format::Bold);
        assert_eq!((model.text(), model.cursor()), ("**one** two", 3));
        let mut model = model_with("one ", 4..4);
        model.toggle_format(Format::Highlight);
        assert_eq!((model.text(), model.cursor()), ("one ====", 6));
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
