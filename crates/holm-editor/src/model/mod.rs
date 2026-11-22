//! This module has no GPUI. Every offset is a byte offset in the source text, on a char boundary. The text has only LF
//! line breaks.

mod caret;
mod display;
mod format;
mod history;
mod links;

use std::borrow::Cow;
use std::collections::HashSet;
use std::ops::Range;
use std::time::Instant;

use holm_markdown::{Body, Target};
use ropey::{LineType, Rope};

pub(crate) use caret::{Affinity, Caret, Direction, GoalRow, Selection};
pub(crate) use display::{
    Cell, Chip, DisplayLine, Embed, Highlight, LineKind, LinkTarget, Look, Mark, Quote, Run, Styles, Syntax, TextKind,
};
pub(crate) use format::Format;
pub(crate) use history::Group;
use history::{Ends, History, TextChange, UndoStep};
pub(crate) use links::LinkForm;

const LF: LineType = LineType::LF;

pub(crate) struct Model {
    rope: Rope,
    /// The same text as `rope`. The parser and the text shaper need one `&str`.
    text: String,
    body: Body,
    /// What the splices since the last parse changed: a range of the old text, and the end of its new text.
    changed: Option<(Range<usize>, usize)>,
    caret: Caret,
    anchor: usize,
    history: History,
    /// The link targets that the app knows are broken.
    broken: HashSet<Target>,
}

/// A range of the current text and its new text.
type Edit = (Range<usize>, String);

/// How far Backspace or Delete reaches with no selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Char,
    /// To the edge of the word, as a word move goes.
    Word,
}

/// For word moves and double clicks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CharKind {
    Word,
    Space,
    Break,
    Other,
}

fn char_kind(c: char) -> CharKind {
    if c == '\n' {
        CharKind::Break
    } else if c.is_alphanumeric() || c == '_' {
        CharKind::Word
    } else if c.is_whitespace() {
        CharKind::Space
    } else {
        CharKind::Other
    }
}

fn lf_only(text: &str) -> Cow<'_, str> {
    if text.contains('\r') {
        Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(text)
    }
}

/// `edits` are in order and apart. An offset inside a replaced range lands at the end of its new text, and an offset on
/// an insert lands after it.
fn map_offset(edits: &[Edit], offset: usize) -> usize {
    let mut mapped = offset;
    for (range, text) in edits {
        let past = if range.is_empty() {
            range.start <= offset
        } else {
            range.end <= offset
        };
        if past {
            mapped = mapped + text.len() - range.len();
        } else if range.start <= offset && offset < range.end {
            return mapped - (offset - range.start) + text.len();
        } else {
            break;
        }
    }
    mapped
}

impl Model {
    pub(crate) fn new(text: &str) -> Self {
        let text = lf_only(text).into_owned();
        Self {
            rope: Rope::from_str(&text),
            body: Body::parse(&text),
            changed: None,
            text,
            caret: Caret::default(),
            anchor: 0,
            history: History::default(),
            broken: HashSet::new(),
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn cursor(&self) -> usize {
        self.caret.offset()
    }

    pub(crate) fn caret(&self) -> Caret {
        self.caret
    }

    pub(crate) fn selection(&self) -> Range<usize> {
        self.cursor().min(self.anchor)..self.cursor().max(self.anchor)
    }

    pub(crate) fn selected_text(&self) -> &str {
        &self.text[self.selection()]
    }

    pub(crate) fn is_reversed(&self) -> bool {
        self.cursor() < self.anchor
    }

    /// Rounds `offset` down to a char boundary. With [`Selection::Extend`], the selection keeps its other end.
    pub(crate) fn set_cursor(&mut self, offset: usize, selection: Selection) {
        self.set_caret(Caret::at(offset), selection);
    }

    /// Moves the cursor like [`Self::set_cursor`], and keeps the affinity and the goals of `caret` while its offset is
    /// on a char boundary.
    pub(crate) fn set_caret(&mut self, caret: Caret, selection: Selection) {
        let offset = self.floor(caret.offset());
        self.caret = if offset == caret.offset() {
            caret
        } else {
            Caret::at(offset)
        };
        if selection == Selection::Collapse {
            self.anchor = offset;
        }
    }

    pub(crate) fn take_goal_row(&mut self) -> Option<GoalRow> {
        let goal = self.caret.goal_row();
        self.caret = self.caret.with_goal_row(None);
        goal
    }

    pub(crate) fn select(&mut self, range: Range<usize>) {
        self.set_cursor(range.start, Selection::Collapse);
        self.set_cursor(range.end, Selection::Extend);
    }

    pub(crate) fn select_all(&mut self) {
        self.select(0..self.text.len());
    }

    fn ends(&self) -> Ends {
        (self.anchor, self.cursor())
    }

    fn set_ends(&mut self, (anchor, cursor): Ends) {
        self.set_cursor(anchor, Selection::Collapse);
        self.set_cursor(cursor, Selection::Extend);
    }

    fn floor(&self, offset: usize) -> usize {
        self.rope.floor_char_boundary(offset.min(self.text.len()))
    }

    /// Makes no parse and no undo step.
    fn splice(&mut self, range: Range<usize>, text: &str) {
        self.rope.remove(range.clone());
        self.rope.insert(range.start, text);
        self.text.replace_range(range.clone(), text);
        self.changed = Some(match self.changed.take() {
            None => (range.clone(), range.start + text.len()),
            Some((old, new_end)) => {
                // `end` is after both changes, so the old text has it `new_end - old.end` earlier.
                let end = new_end.max(range.end);
                (
                    old.start.min(range.start)..end + old.end - new_end,
                    end + text.len() - range.len(),
                )
            }
        });
    }

    fn reparse(&mut self) {
        if let Some((old, new_end)) = self.changed.take() {
            self.body.reparse(&self.text, old.clone(), new_end - old.start);
        }
    }

    /// The edits are in order and apart, in the current text. `ends` are in the new text. Returns whether the text
    /// changed.
    fn edit(&mut self, edits: Vec<Edit>, ends: Ends, group: Group, now: Option<Instant>) -> bool {
        let before = self.ends();
        let mut changes = Vec::with_capacity(edits.len());
        for (range, text) in edits.into_iter().rev() {
            let end = self.floor(range.end);
            let start = self.floor(range.start.min(end));
            if self.text[start..end] == text {
                continue;
            }
            changes.push(TextChange {
                start,
                removed: self.text[start..end].to_owned(),
                inserted: text.clone(),
            });
            self.splice(start..end, &text);
        }
        if changes.is_empty() {
            return false;
        }
        self.reparse();
        self.set_ends(ends);
        self.history.record(UndoStep {
            changes,
            before,
            after: self.ends(),
            group,
            time: now,
        });
        true
    }

    pub(crate) fn type_in(&mut self, range: Range<usize>, text: &str, now: Instant) -> bool {
        let text = lf_only(text);
        let end = self.floor(range.end);
        let start = self.floor(range.start.min(end));
        let cursor = start + text.len();
        self.edit(
            vec![(start..end, text.into_owned())],
            (cursor, cursor),
            Group::Typing,
            Some(now),
        )
    }

    pub(crate) fn type_text(&mut self, text: &str, now: Instant) -> bool {
        self.type_in(self.selection(), text, now)
    }

    /// Its own undo step, like a paste.
    pub(crate) fn insert(&mut self, text: &str) -> bool {
        let text = lf_only(text);
        let range = self.selection();
        let cursor = range.start + text.len();
        self.edit(vec![(range, text.into_owned())], (cursor, cursor), Group::Alone, None)
    }

    /// One undo step, with only the changed middle of the text. The cursor stays near its place.
    pub(crate) fn reload(&mut self, text: &str) -> bool {
        let text = lf_only(text);
        let old = self.text.as_bytes();
        let new = text.as_bytes();
        let mut prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        while !self.text.is_char_boundary(prefix) || !text.is_char_boundary(prefix) {
            prefix -= 1;
        }
        let max_suffix = old.len().min(new.len()) - prefix;
        let mut suffix = old
            .iter()
            .rev()
            .zip(new.iter().rev())
            .take(max_suffix)
            .take_while(|(a, b)| a == b)
            .count();
        while !self.text.is_char_boundary(old.len() - suffix) || !text.is_char_boundary(new.len() - suffix) {
            suffix -= 1;
        }
        let edits = vec![(prefix..old.len() - suffix, text[prefix..new.len() - suffix].to_owned())];
        let ends = (map_offset(&edits, self.anchor), map_offset(&edits, self.cursor()));
        self.edit(edits, ends, Group::Alone, None)
    }

    /// Returns whether the text changed.
    pub(crate) fn undo(&mut self) -> bool {
        let Some(step) = self.history.pop_undo() else {
            return false;
        };
        for change in step.changes.iter().rev() {
            let range = change.start..change.start + change.inserted.len();
            self.splice(range, &change.removed);
        }
        self.reparse();
        self.set_ends(step.before);
        self.history.push_redo(step);
        true
    }

    /// Returns whether the text changed.
    pub(crate) fn redo(&mut self) -> bool {
        let Some(step) = self.history.pop_redo() else {
            return false;
        };
        for change in &step.changes {
            let range = change.start..change.start + change.removed.len();
            self.splice(range, &change.inserted);
        }
        self.reparse();
        self.set_ends(step.after);
        self.history.push_undo(step);
        true
    }

    /// Returns whether the text changed.
    pub(crate) fn backspace(&mut self, step: Step, now: Instant) -> bool {
        let mut range = self.selection();
        if range.is_empty() {
            range.start = match step {
                Step::Char => self.prev_char(self.cursor()),
                Step::Word => self.word_left(self.cursor()),
            };
        }
        self.remove(range, now)
    }

    /// Returns whether the text changed.
    pub(crate) fn delete(&mut self, step: Step, now: Instant) -> bool {
        let mut range = self.selection();
        if range.is_empty() {
            range.end = match step {
                Step::Char => self.next_char(self.cursor()),
                Step::Word => self.word_right(self.cursor()),
            };
        }
        self.remove(range, now)
    }

    /// Its own undo step, like a cut.
    pub(crate) fn remove_selection(&mut self) -> bool {
        let range = self.selection();
        self.edit(
            vec![(range.clone(), String::new())],
            (range.start, range.start),
            Group::Alone,
            None,
        )
    }

    fn remove(&mut self, range: Range<usize>, now: Instant) -> bool {
        if range.is_empty() {
            return false;
        }
        self.edit(
            vec![(range.clone(), String::new())],
            (range.start, range.start),
            Group::Deleting,
            Some(now),
        )
    }

    pub(crate) fn move_left(&mut self, selection: Selection) {
        let selected = self.selection();
        if selection == Selection::Collapse && !selected.is_empty() {
            self.set_cursor(selected.start, Selection::Collapse);
        } else {
            self.set_cursor(self.prev_char(self.cursor()), selection);
        }
    }

    pub(crate) fn move_right(&mut self, selection: Selection) {
        let selected = self.selection();
        if selection == Selection::Collapse && !selected.is_empty() {
            self.set_cursor(selected.end, Selection::Collapse);
        } else {
            self.set_cursor(self.next_char(self.cursor()), selection);
        }
    }

    pub(crate) fn move_word_left(&mut self, selection: Selection) {
        self.set_cursor(self.word_left(self.cursor()), selection);
    }

    pub(crate) fn move_word_right(&mut self, selection: Selection) {
        self.set_cursor(self.word_right(self.cursor()), selection);
    }

    /// The offset before the spaces and then the run of one kind of chars before `offset`. A line break is a run of its
    /// own.
    fn word_left(&self, offset: usize) -> usize {
        let mut start = offset;
        let mut chars = self.text[..offset].char_indices().rev().peekable();
        while let Some(&(at, c)) = chars.peek()
            && char_kind(c) == CharKind::Space
        {
            start = at;
            chars.next();
        }
        let Some((at, first)) = chars.next() else {
            return start;
        };
        start = at;
        let kind = char_kind(first);
        if kind == CharKind::Break {
            return start;
        }
        for (at, c) in chars {
            if char_kind(c) != kind {
                break;
            }
            start = at;
        }
        start
    }

    /// The offset after the spaces and then the run of one kind of chars after `offset`. A line break is a run of its
    /// own.
    fn word_right(&self, offset: usize) -> usize {
        let mut end = offset;
        let mut chars = self.text[offset..].chars().peekable();
        while let Some(&c) = chars.peek()
            && char_kind(c) == CharKind::Space
        {
            end += c.len_utf8();
            chars.next();
        }
        let Some(first) = chars.next() else {
            return end;
        };
        end += first.len_utf8();
        let kind = char_kind(first);
        if kind == CharKind::Break {
            return end;
        }
        for c in chars {
            if char_kind(c) != kind {
                break;
            }
            end += c.len_utf8();
        }
        end
    }

    /// The run of one kind of chars at `offset`, for a double click. A line break is never part of it.
    pub(crate) fn word_at(&self, offset: usize) -> Range<usize> {
        let offset = self.floor(offset);
        let after = self.text[offset..].chars().next();
        let before = self.text[..offset].chars().next_back();
        let kind = match (before.map(char_kind), after.map(char_kind)) {
            (_, Some(kind)) | (Some(kind), _) if kind != CharKind::Break => kind,
            _ => return offset..offset,
        };
        let start = self.text[..offset]
            .char_indices()
            .rev()
            .take_while(|(_, c)| char_kind(*c) == kind)
            .last()
            .map_or(offset, |(at, _)| at);
        let end = self.text[offset..]
            .char_indices()
            .take_while(|(_, c)| char_kind(*c) == kind)
            .last()
            .map_or(offset, |(at, c)| offset + at + c.len_utf8());
        start..end
    }

    /// With its line break, for a triple click.
    pub(crate) fn line_at(&self, offset: usize) -> Range<usize> {
        let range = self.line_range(self.line_of(offset));
        let end = if range.end < self.text.len() {
            range.end + 1
        } else {
            range.end
        };
        range.start..end
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

    /// An empty text has one line.
    pub(crate) fn line_count(&self) -> usize {
        self.rope.len_lines(LF)
    }

    pub(crate) fn line_of(&self, offset: usize) -> usize {
        self.rope.byte_to_line_idx(offset.min(self.text.len()), LF)
    }

    /// Without its line break.
    pub(crate) fn line_range(&self, line: usize) -> Range<usize> {
        let start = self.rope.line_to_byte_idx(line, LF);
        let end = if line + 1 < self.line_count() {
            self.rope.line_to_byte_idx(line + 1, LF) - 1
        } else {
            self.text.len()
        };
        start..end
    }

    fn syntax_of(&self, range: &Range<usize>) -> Syntax {
        let selection = self.selection();
        if range.start <= selection.end && selection.start <= range.end {
            Syntax::Shown
        } else {
            Syntax::Hidden
        }
    }

    /// For the platform text input.
    pub(crate) fn offset_to_utf16(&self, offset: usize) -> usize {
        self.rope.byte_to_utf16_idx(offset.min(self.text.len()))
    }

    /// For the platform text input.
    pub(crate) fn utf16_to_offset(&self, offset: usize) -> usize {
        self.rope.utf16_to_byte_idx(offset.min(self.rope.len_utf16()))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use display::plain_chip;
    use gpui_kit::px;
    use holm_markdown::list::Move;
    use proptest::prelude::*;

    fn now() -> Instant {
        Instant::now()
    }

    fn model_at(text: &str, cursor: usize) -> Model {
        let mut model = Model::new(text);
        model.set_cursor(cursor, Selection::Collapse);
        model
    }

    fn row_caret(offset: usize) -> Caret {
        let goal_row = GoalRow {
            direction: Direction::Down,
            selection: Selection::Collapse,
        };
        Caret::at(offset)
            .with_affinity(Affinity::RowStart)
            .with_goal_x(px(40.))
            .with_goal_row(Some(goal_row))
    }

    #[test]
    fn a_row_move_keeps_the_affinity_and_the_goals() {
        let mut model = Model::new("one two three");
        model.set_caret(row_caret(4), Selection::Collapse);
        assert_eq!(model.caret(), row_caret(4));
    }

    #[test]
    fn any_other_cursor_change_drops_the_affinity_and_the_goals() {
        let mut model = Model::new("one two three");
        model.set_caret(row_caret(4), Selection::Collapse);
        model.move_right(Selection::Collapse);
        assert_eq!(model.caret(), Caret::at(5));

        model.set_caret(row_caret(4), Selection::Collapse);
        model.type_text("x", now());
        assert_eq!(model.caret(), Caret::at(5));

        model.set_caret(row_caret(4), Selection::Collapse);
        model.undo();
        assert_eq!(model.caret(), Caret::at(4));
    }

    #[test]
    fn a_caret_off_a_char_boundary_drops_the_affinity_and_the_goals() {
        let mut model = Model::new("aé");
        model.set_caret(row_caret(2), Selection::Collapse);
        assert_eq!(model.caret(), Caret::at(1));
    }

    #[test]
    fn the_goal_row_goes_once_and_the_goal_x_stays() {
        let mut model = Model::new("one two three");
        model.set_caret(row_caret(4), Selection::Collapse);
        assert!(model.take_goal_row().is_some());
        assert_eq!(model.take_goal_row(), None);
        assert_eq!(model.caret().goal_x(), Some(px(40.)));
        assert_eq!(model.caret().affinity(), Affinity::RowStart);
    }

    #[test]
    fn typing_replaces_the_selection() {
        let mut model = Model::new("one two");
        model.select(4..7);
        model.type_text("three", now());
        assert_eq!(model.text(), "one three");
        assert_eq!(model.cursor(), 9);
        assert!(model.selection().is_empty());
    }

    #[test]
    fn backspace_and_delete_remove_whole_chars() {
        let mut model = model_at("aé🙂b", 7);
        assert!(model.backspace(Step::Char, now()));
        assert_eq!(model.text(), "aéb");
        assert_eq!(model.cursor(), 3);
        assert!(model.backspace(Step::Char, now()));
        assert_eq!(model.text(), "ab");
        assert!(model.delete(Step::Char, now()));
        assert_eq!(model.text(), "a");
        assert!(!model.delete(Step::Char, now()));
        model.set_cursor(0, Selection::Collapse);
        assert!(!model.backspace(Step::Char, now()));
    }

    #[test]
    fn arrows_collapse_a_selection() {
        let mut model = Model::new("abcdef");
        model.select(2..4);
        model.move_left(Selection::Collapse);
        assert_eq!(model.selection(), 2..2);
        model.move_right(Selection::Extend);
        model.move_right(Selection::Extend);
        assert_eq!(model.selection(), 2..4);
        model.move_right(Selection::Collapse);
        assert_eq!(model.selection(), 4..4);
    }

    #[test]
    fn word_moves_skip_spaces_and_one_run_of_chars() {
        let mut model = Model::new("one, two\nthree");
        model.move_word_right(Selection::Collapse);
        assert_eq!(model.cursor(), 3);
        model.move_word_right(Selection::Collapse);
        assert_eq!(model.cursor(), 4);
        model.move_word_right(Selection::Collapse);
        assert_eq!(model.cursor(), 8);
        model.move_word_right(Selection::Collapse);
        assert_eq!(model.cursor(), 9);
        model.move_word_left(Selection::Extend);
        assert_eq!(model.selection(), 8..9);
        model.move_word_left(Selection::Collapse);
        assert_eq!(model.cursor(), 5);
    }

    #[test]
    fn word_keys_remove_a_word() {
        let mut model = model_at("one two", 7);
        assert!(model.backspace(Step::Word, now()));
        assert_eq!(model.text(), "one ");
        model.set_cursor(0, Selection::Collapse);
        assert!(model.delete(Step::Word, now()));
        assert_eq!(model.text(), " ");
    }

    #[test]
    fn a_double_click_picks_a_word_and_a_triple_click_a_line() {
        let model = Model::new("say héllo!\nnext");
        assert_eq!(model.word_at(6), 4..10);
        assert_eq!(model.word_at(4), 4..10);
        assert_eq!(model.word_at(11), 10..11);
        assert_eq!(model.word_at(3), 3..4);
        assert_eq!(model.line_at(2), 0..12);
        assert_eq!(model.line_at(13), 12..16);
        assert_eq!(Model::new("a\n\nb").word_at(2), 2..2);
    }

    #[test]
    fn the_text_has_only_lf_line_breaks() {
        let mut model = Model::new("a\r\nb\rc");
        assert_eq!(model.text(), "a\nb\nc");
        model.set_cursor(5, Selection::Collapse);
        model.insert("\r\nd");
        assert_eq!(model.text(), "a\nb\nc\nd");
    }

    #[test]
    fn undo_takes_back_typing_one_word_at_a_time() {
        let start = now();
        let mut model = Model::new("");
        for (ix, c) in "one two".chars().enumerate() {
            model.type_text(&c.to_string(), start + Duration::from_millis(ix as u64 * 100));
        }
        assert!(model.undo());
        assert_eq!(model.text(), "one ");
        assert_eq!(model.cursor(), 4);
        assert!(model.undo());
        assert_eq!(model.text(), "");
        assert!(!model.undo());
        assert!(model.redo());
        assert!(model.redo());
        assert_eq!(model.text(), "one two");
        assert_eq!(model.cursor(), 7);
        assert!(!model.redo());
    }

    #[test]
    fn a_pause_starts_a_new_undo_step() {
        let start = now();
        let mut model = Model::new("");
        model.type_text("a", start);
        model.type_text("b", start + Duration::from_millis(500));
        model.type_text("c", start + Duration::from_millis(1600));
        model.undo();
        assert_eq!(model.text(), "ab");
    }

    #[test]
    fn removals_join_one_undo_step() {
        let start = now();
        let mut model = model_at("abcd", 2);
        model.backspace(Step::Char, start);
        model.backspace(Step::Char, start);
        model.delete(Step::Char, start);
        assert_eq!(model.text(), "d");
        model.undo();
        assert_eq!(model.text(), "abcd");
        assert_eq!(model.cursor(), 2);
    }

    #[test]
    fn undo_puts_back_the_cursor_and_the_selection() {
        let mut model = Model::new("one two");
        model.select(4..7);
        model.toggle_format(Format::Bold);
        model.set_cursor(0, Selection::Collapse);
        model.undo();
        assert_eq!(model.text(), "one two");
        assert_eq!(model.selection(), 4..7);
    }

    #[test]
    fn a_new_edit_clears_the_redo_steps() {
        let mut model = Model::new("");
        model.insert("a");
        model.undo();
        model.insert("b");
        assert!(!model.redo());
        assert_eq!(model.text(), "b");
    }

    #[test]
    fn a_reload_is_one_undo_step_and_keeps_the_cursor_near() {
        let mut model = model_at("one two three", 10);
        assert!(model.reload("one 2 three"));
        assert_eq!(model.cursor(), 8);
        assert!(!model.reload("one 2 three"));
        model.undo();
        assert_eq!(model.text(), "one two three");
        assert_eq!(model.cursor(), 10);
        assert!(model.reload("é"));
        assert!(model.reload("è"));
        assert_eq!(model.text(), "è");
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
        let line = model.display_line(0, &mut plain_chip);
        assert_eq!(line.syntax, Syntax::Hidden);
        assert_eq!(line.shown.text, "a b c");
        assert_eq!(line.shown.runs.len(), 3);
        assert!(line.shown.runs[1].look.styles.contains(Styles::STRONG));
        assert_eq!(line.to_source(2), 4);
        assert_eq!(line.to_source(3), 7);
        assert_eq!(line.to_display(4), 2);
        assert_eq!(line.to_display(2), 2);
    }

    #[test]
    fn syntax_shows_on_the_cursor_line() {
        let model = model_at("a **b** c\n\nnext", 1);
        let line = model.display_line(0, &mut plain_chip);
        assert_eq!(line.syntax, Syntax::Shown);
        assert_eq!(line.shown.text, "a **b** c");
        assert_eq!(line.shown.runs[1].look.kind, TextKind::Syntax);
        assert_eq!(line.to_source(5), 5);
    }

    #[test]
    fn a_selection_shows_syntax_on_each_line_it_touches() {
        let mut model = Model::new("**a**\n**b**\n**c**");
        model.select(2..8);
        assert_eq!(model.display_line(0, &mut plain_chip).syntax, Syntax::Shown);
        assert_eq!(model.display_line(1, &mut plain_chip).syntax, Syntax::Shown);
        assert_eq!(model.display_line(2, &mut plain_chip).syntax, Syntax::Hidden);
    }

    #[test]
    fn embedded_images_hide_and_are_listed() {
        let model = model_at("top\n\n![cat](cat.png)\n", 0);
        let line = model.display_line(2, &mut |_| Embed::Image);
        assert_eq!(line.shown.text, "");
        assert_eq!(line.images, ["cat.png"]);
        assert_eq!(line.shown.chips, []);
        assert_eq!(line.to_source(0), line.source().start);
    }

    #[test]
    fn an_embedded_file_shows_as_a_chip_in_its_line() {
        let text = "top\n\nSee ![f](docs/report.pdf) now\n";
        let model = model_at(text, 0);
        let line = model.display_line(2, &mut |_| Embed::Chip);
        assert_eq!(line.images, [] as [String; 0]);
        let [chip] = line.shown.chips.as_slice() else {
            panic!("one chip");
        };
        assert_eq!(chip.target, "docs/report.pdf");
        assert_eq!(line.shown.text, "See \u{fffc} now");
        assert_eq!(&line.shown.text[..chip.range.start], "See ");
        assert_eq!(&line.shown.text[chip.range.end..], " now");
        let bang = text.find('!').unwrap_or_default();
        assert_eq!(line.to_source(chip.range.start), bang);
        assert_eq!(line.to_display(bang + 3), chip.range.start);
        assert_eq!(line.to_source(chip.range.end), text.find(" now").unwrap_or_default());
    }

    #[test]
    fn the_right_edge_of_a_chip_at_the_end_of_a_line_maps_to_the_end_of_its_link() {
        for text in ["![](a)\nb", "c ![](a\n)"] {
            let model = model_at(text, text.len());
            let line = model.display_line(0, &mut plain_chip);
            let [chip] = line.shown.chips.as_slice() else {
                panic!("one chip in {text:?}");
            };
            assert_eq!(chip.range.end, line.shown.text.len(), "{text:?}");
            let end = line.source().end;
            assert_eq!(line.to_source(chip.range.end), end, "{text:?}");
            assert_eq!(line.to_display(end), chip.range.end, "{text:?}");
            let bang = text.find('!').unwrap_or_default();
            assert_eq!(line.to_source(chip.range.start), bang, "{text:?}");
            assert_eq!(line.to_display(bang), chip.range.start, "{text:?}");
        }
    }

    #[test]
    fn a_line_with_the_cursor_shows_the_chip_as_markdown() {
        let model = model_at("![f](report.pdf)\n", 3);
        let line = model.display_line(0, &mut plain_chip);
        assert_eq!(line.shown.text, "![f](report.pdf)");
        assert_eq!(line.shown.chips, []);
    }

    #[test]
    fn heading_lines_know_their_level() {
        let model = model_at("x\n\n## Plans\n", 0);
        let line = model.display_line(2, &mut plain_chip);
        assert_eq!(line.heading, Some(2));
        assert_eq!(line.shown.text, "Plans");
    }

    #[test]
    fn splices_join_into_one_changed_range() {
        let mut model = Model::new("abcdefgh");
        model.splice(6..7, "XYZ");
        model.splice(1..3, "");
        assert_eq!(model.text(), "adefXYZh");
        assert_eq!(model.changed, Some((1..7, 7)));
        model.reparse();
        assert_eq!(model.changed, None);
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
        Type(String, u64),
        Insert(String),
        Backspace(Step),
        Delete(Step),
        Left(Selection),
        Right(Selection),
        Cursor(usize, Selection),
        Format(usize),
        Heading(u8),
        Newline,
        Indent(Move),
        Undo,
        Redo,
    }

    const FORMATS: [Format; 6] = [
        Format::Bold,
        Format::Italic,
        Format::Underline,
        Format::Strikethrough,
        Format::Highlight,
        Format::Code,
    ];

    fn selection() -> impl Strategy<Value = Selection> {
        prop_oneof![Just(Selection::Collapse), Just(Selection::Extend)]
    }

    fn step() -> impl Strategy<Value = Step> {
        prop_oneof![Just(Step::Char), Just(Step::Word)]
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            (text(), 0..1500u64).prop_map(|(text, ms)| Op::Type(text, ms)),
            text().prop_map(Op::Insert),
            step().prop_map(Op::Backspace),
            step().prop_map(Op::Delete),
            selection().prop_map(Op::Left),
            selection().prop_map(Op::Right),
            (0..60usize, selection()).prop_map(|(at, selection)| Op::Cursor(at, selection)),
            (0..FORMATS.len()).prop_map(Op::Format),
            (0..=6u8).prop_map(Op::Heading),
            Just(Op::Newline),
            prop_oneof![Just(Move::In), Just(Move::Out)].prop_map(Op::Indent),
            Just(Op::Undo),
            Just(Op::Redo),
        ]
    }

    /// The clock moves on by the pause in each [`Op::Type`].
    fn run(model: &mut Model, op: Op, clock: &mut Instant) {
        match op {
            Op::Type(text, ms) => {
                *clock += Duration::from_millis(ms);
                model.type_text(&text, *clock);
            }
            Op::Insert(text) => {
                model.insert(&text);
            }
            Op::Backspace(step) => {
                model.backspace(step, *clock);
            }
            Op::Delete(step) => {
                model.delete(step, *clock);
            }
            Op::Left(selection) => model.move_left(selection),
            Op::Right(selection) => model.move_right(selection),
            Op::Cursor(at, selection) => model.set_cursor(at, selection),
            Op::Format(ix) => {
                model.toggle_format(FORMATS[ix]);
            }
            Op::Heading(level) => {
                model.toggle_heading(level);
            }
            Op::Newline => {
                model.newline(*clock);
            }
            Op::Indent(key) => {
                model.relist(key);
            }
            Op::Undo => {
                model.undo();
            }
            Op::Redo => {
                model.redo();
            }
        }
    }

    proptest! {
        #[test]
        fn the_cursor_stays_on_a_char_boundary(start in text(), ops in prop::collection::vec(op(), 0..30)) {
            let mut model = Model::new(&start);
            let mut clock = now();
            for op in ops {
                run(&mut model, op, &mut clock);
                prop_assert!(model.text().is_char_boundary(model.cursor()));
                prop_assert!(model.text().is_char_boundary(model.selection().start));
                prop_assert_eq!(model.rope.to_string(), model.text());
            }
        }

        #[test]
        fn the_parse_after_each_edit_equals_a_full_parse(start in text(), ops in prop::collection::vec(op(), 0..30)) {
            let mut model = Model::new(&start);
            let mut clock = now();
            for op in ops {
                run(&mut model, op, &mut clock);
                let full = Body::parse(model.text());
                prop_assert_eq!(model.body.blocks(), full.blocks());
                prop_assert_eq!(model.body.spans(), full.spans());
                prop_assert_eq!(model.body.hidden(), full.hidden());
                prop_assert_eq!(model.body.links(), full.links());
                prop_assert_eq!(model.body.headings(), full.headings());
            }
        }

        #[test]
        fn undo_then_redo_gives_the_same_text(start in text(), ops in prop::collection::vec(op(), 0..30)) {
            let mut model = Model::new(&start);
            let mut clock = now();
            for op in ops {
                run(&mut model, op, &mut clock);
            }
            let end = model.text().to_owned();
            let mut steps = 0;
            while model.undo() {
                steps += 1;
                prop_assert!(model.text().is_char_boundary(model.cursor()));
            }
            let start = lf_only(&start).into_owned();
            prop_assert_eq!(model.text(), start);
            for _ in 0..steps {
                prop_assert!(model.redo());
            }
            prop_assert_eq!(model.text(), end);
            prop_assert_eq!(model.rope.to_string(), model.text());
        }

        #[test]
        fn screen_lines_map_back_into_their_source(start in text(), at in 0..60usize, image in any::<bool>()) {
            let model = model_at(&start, at);
            let embed = if image { Embed::Image } else { Embed::Chip };
            for line in 0..model.line_count() {
                let display = model.display_line(line, &mut |_| embed);
                let source = display.source();
                prop_assert_eq!(display.shown.runs.iter().map(|r| r.len).sum::<usize>(), display.shown.text.len());
                for offset in (0..=display.shown.text.len()).filter(|o| display.shown.text.is_char_boundary(*o)) {
                    let mapped = display.to_source(offset);
                    prop_assert!(source.start <= mapped && mapped <= source.end);
                    prop_assert!(model.text().is_char_boundary(mapped));
                    prop_assert_eq!(display.to_display(mapped), offset);
                }
            }
        }
    }
}
