//! The writing helps: format keys, headings, and lists.

use std::ops::Range;

pub(crate) use holm_markdown::inline::Format;
use holm_markdown::inline::{Edit as MarkdownEdit, restyle};
use holm_markdown::list::{Move, relist};
use holm_markdown::{Block, BlockKind};

use super::{Edit, Group, Model, map_offset};

impl Model {
    /// Adds or removes the style of `format` on the selection, or at the cursor. A color key
    /// sets its color. Returns whether the text changed.
    pub(crate) fn toggle_format(&mut self, format: Format) -> bool {
        let Some(edit) = restyle(&self.text, &self.body, self.selection(), format) else {
            return false;
        };
        self.apply(edit)
    }

    /// Applies `edit` as one undo step. The selection keeps its direction.
    fn apply(&mut self, edit: MarkdownEdit) -> bool {
        let ends = if self.is_reversed() {
            (edit.selection.end, edit.selection.start)
        } else {
            (edit.selection.start, edit.selection.end)
        };
        self.edit(vec![(edit.range, edit.text)], ends, Group::Alone, None)
    }
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
        let cursor = map_offset(&edits, self.cursor());
        self.edit(edits, (anchor, cursor), Group::Alone, None)
    }

    /// Enter: in a list item it starts the next item and counts the list again, and on an empty
    /// item it ends the list. Elsewhere it types a line break. Returns whether the text changed.
    pub(crate) fn newline(&mut self, continue_lists: bool, now: std::time::Instant) -> bool {
        let edit = continue_lists
            .then(|| relist(&self.text, &self.body, self.selection(), Move::Next))
            .flatten();
        match edit {
            Some(edit) => self.apply(edit),
            None => self.type_text("\n", now),
        }
    }

    /// Tab and Shift+Tab: moves each list item that the selection touches one level in or out,
    /// with its children, and counts the lists it changes again.
    /// Returns `None` when the selection touches no list item.
    pub(crate) fn indent(&mut self, out: bool) -> Option<bool> {
        let key = if out { Move::Out } else { Move::In };
        let edit = relist(&self.text, &self.body, self.selection(), key)?;
        Some(self.apply(edit))
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
        assert_eq!(enter("- one\n- ", 8), ("- one\n\n".into(), 7));
        assert_eq!(enter("> - a\n> - ", 10), ("> - a\n>\n> ".into(), 10));
        assert_eq!(enter("- a\n  - b\n  - ", 14), ("- a\n  - b\n\n".into(), 11));
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
            Some("10. a\n    1. b".into())
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
    fn tab_and_shift_tab_count_each_level() {
        let mut model = model_with("1. Milk\n2. Eggs\n3. Bread", 13..13);
        model.indent(false);
        assert_eq!(model.text(), "1. Milk\n   1. Eggs\n2. Bread");
        assert_eq!(model.cursor(), 16);
        model.indent(true);
        assert_eq!(model.text(), "1. Milk\n2. Eggs\n3. Bread");
        assert_eq!(model.cursor(), 13);
    }

    #[test]
    fn enter_counts_the_list_again() {
        assert_eq!(
            enter("1. a\n2. b\n3. c", 9),
            ("1. a\n2. b\n3. \n4. c".into(), 13)
        );
    }

    #[test]
    fn a_list_key_is_one_undo_step() {
        let text = "1. a\n2. b\n3. c\n4. d";
        let mut model = model_with(text, 9..9);
        model.indent(false);
        assert_eq!(model.text(), "1. a\n   1. b\n2. c\n3. d");
        model.undo();
        assert_eq!((model.text(), model.cursor()), (text, 9));
        model.newline(true, Instant::now());
        assert_eq!(model.text(), "1. a\n2. b\n3. \n4. c\n5. d");
        model.undo();
        assert_eq!((model.text(), model.cursor()), (text, 9));
    }

    #[test]
    fn the_cursor_stays_on_its_item_when_the_number_grows() {
        let mut model = model_with("9. a\n   1. bc", 12..12);
        model.indent(true);
        assert_eq!((model.text(), model.cursor()), ("9. a\n10. bc", 10));
    }

    #[test]
    fn typing_deleting_and_pasting_do_not_count_a_list_again() {
        let mut model = model_with("1. a\n2. b\n3. c", 4..9);
        model.backspace(false, Instant::now());
        assert_eq!(model.text(), "1. a\n3. c");
        model.insert("\n7. x");
        assert_eq!(model.text(), "1. a\n7. x\n3. c");
        model.type_text("\n2. y", Instant::now());
        assert_eq!(model.text(), "1. a\n7. x\n2. y\n3. c");
    }

    #[test]
    fn tab_outside_a_list_is_not_an_indent() {
        assert_eq!(indented("text", 0..0, false), None);
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
