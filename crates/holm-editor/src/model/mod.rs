//! This module has no GPUI. Every [`SourceOffset`] is on a char boundary. The text has only LF line breaks.

mod caret;
mod display;
mod format;
mod history;
mod links;

use std::borrow::Cow;
use std::collections::HashSet;
use std::ops::Range;
use std::time::Instant;

use holm_markdown::{Body, OffsetRange, SourceOffset, Target};
use ropey::{LineType, Rope};

pub(crate) use caret::{Affinity, Caret, Direction, GoalRow, Selection};
pub(crate) use display::{
    Cell, Chip, CodeLine, DisplayLine, Embed, Highlight, LineKind, LinkTarget, Look, Mark, Quote, Run, ScreenOffset,
    ScreenRange, Styles, Syntax, TextKind,
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
    changed: Option<(Range<SourceOffset>, SourceOffset)>,
    caret: Caret,
    anchor: SourceOffset,
    history: History,
    /// The link targets that the app knows are broken.
    broken: HashSet<Target>,
}

/// A range of the current text and its new text.
type Edit = (Range<SourceOffset>, String);

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
fn map_offset(edits: &[Edit], offset: SourceOffset) -> SourceOffset {
    let mut mapped = offset;
    for (range, text) in edits {
        let past = if range.is_empty() {
            range.start <= offset
        } else {
            range.end <= offset
        };
        if past {
            mapped = mapped + text.len() - range.bytes().len();
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
            anchor: SourceOffset::default(),
            history: History::default(),
            broken: HashSet::new(),
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn cursor(&self) -> SourceOffset {
        self.caret.offset()
    }

    pub(crate) fn caret(&self) -> Caret {
        self.caret
    }

    pub(crate) fn selection(&self) -> Range<SourceOffset> {
        self.cursor().min(self.anchor)..self.cursor().max(self.anchor)
    }

    pub(crate) fn selected_text(&self) -> &str {
        &self.text[self.selection().bytes()]
    }

    pub(crate) fn is_reversed(&self) -> bool {
        self.cursor() < self.anchor
    }

    /// Rounds `offset` down to a char boundary. With [`Selection::Extend`], the selection keeps its other end.
    pub(crate) fn set_cursor(&mut self, offset: SourceOffset, selection: Selection) {
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

    pub(crate) fn select(&mut self, range: Range<SourceOffset>) {
        self.set_cursor(range.start, Selection::Collapse);
        self.set_cursor(range.end, Selection::Extend);
    }

    pub(crate) fn select_all(&mut self) {
        self.select(SourceOffset::default()..self.end());
    }

    /// The offset after the last byte of the text.
    pub(crate) fn end(&self) -> SourceOffset {
        SourceOffset::new(self.text.len())
    }

    fn ends(&self) -> Ends {
        (self.anchor, self.cursor())
    }

    fn set_ends(&mut self, (anchor, cursor): Ends) {
        self.set_cursor(anchor, Selection::Collapse);
        self.set_cursor(cursor, Selection::Extend);
    }

    fn floor(&self, offset: SourceOffset) -> SourceOffset {
        SourceOffset::new(self.rope.floor_char_boundary(offset.min(self.end()).get()))
    }

    /// Makes no parse and no undo step.
    fn splice(&mut self, range: Range<SourceOffset>, text: &str) {
        self.rope.remove(range.bytes());
        self.rope.insert(range.start.get(), text);
        self.text.replace_range(range.bytes(), text);
        self.changed = Some(match self.changed.take() {
            None => (range.clone(), range.start + text.len()),
            Some((old, new_end)) => {
                // `end` is after both changes, so the old text has it `new_end - old.end` earlier.
                let end = new_end.max(range.end);
                (
                    old.start.min(range.start)..old.end + (end - new_end),
                    end + text.len() - range.bytes().len(),
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
            if self.text[(start..end).bytes()] == text {
                continue;
            }
            changes.push(TextChange {
                start,
                removed: self.text[(start..end).bytes()].to_owned(),
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

    pub(crate) fn type_in(&mut self, range: Range<SourceOffset>, text: &str, now: Instant) -> bool {
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
        let edits = vec![(
            SourceOffset::range(prefix..old.len() - suffix),
            text[prefix..new.len() - suffix].to_owned(),
        )];
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

    fn remove(&mut self, range: Range<SourceOffset>, now: Instant) -> bool {
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
    fn word_left(&self, offset: SourceOffset) -> SourceOffset {
        let mut start = offset;
        let mut chars = self.text[..offset.get()].char_indices().rev().peekable();
        while let Some(&(at, c)) = chars.peek()
            && char_kind(c) == CharKind::Space
        {
            start = SourceOffset::new(at);
            chars.next();
        }
        let Some((at, first)) = chars.next() else {
            return start;
        };
        start = SourceOffset::new(at);
        let kind = char_kind(first);
        if kind == CharKind::Break {
            return start;
        }
        for (at, c) in chars {
            if char_kind(c) != kind {
                break;
            }
            start = SourceOffset::new(at);
        }
        start
    }

    /// The offset after the spaces and then the run of one kind of chars after `offset`. A line break is a run of its
    /// own.
    fn word_right(&self, offset: SourceOffset) -> SourceOffset {
        let mut end = offset;
        let mut chars = self.text[offset.get()..].chars().peekable();
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
    pub(crate) fn word_at(&self, offset: SourceOffset) -> Range<SourceOffset> {
        let offset = self.floor(offset);
        let after = self.text[offset.get()..].chars().next();
        let before = self.text[..offset.get()].chars().next_back();
        let kind = match (before.map(char_kind), after.map(char_kind)) {
            (_, Some(kind)) | (Some(kind), _) if kind != CharKind::Break => kind,
            _ => return offset..offset,
        };
        let start = self.text[..offset.get()]
            .char_indices()
            .rev()
            .take_while(|(_, c)| char_kind(*c) == kind)
            .last()
            .map_or(offset, |(at, _)| SourceOffset::new(at));
        let end = self.text[offset.get()..]
            .char_indices()
            .take_while(|(_, c)| char_kind(*c) == kind)
            .last()
            .map_or(offset, |(at, c)| offset + at + c.len_utf8());
        start..end
    }

    /// With its line break, for a triple click.
    pub(crate) fn line_at(&self, offset: SourceOffset) -> Range<SourceOffset> {
        let range = self.line_range(self.line_of(offset));
        let end = if range.end < self.end() {
            range.end + 1
        } else {
            range.end
        };
        range.start..end
    }

    fn prev_char(&self, offset: SourceOffset) -> SourceOffset {
        match offset.get().checked_sub(1) {
            Some(before) => SourceOffset::new(self.rope.floor_char_boundary(before)),
            None => offset,
        }
    }

    fn next_char(&self, offset: SourceOffset) -> SourceOffset {
        if offset >= self.end() {
            self.end()
        } else {
            SourceOffset::new(self.rope.ceil_char_boundary(offset.get() + 1))
        }
    }

    /// An empty text has one line.
    pub(crate) fn line_count(&self) -> usize {
        self.rope.len_lines(LF)
    }

    pub(crate) fn last_line(&self) -> LineIndex {
        LineIndex(self.line_count() - 1)
    }

    pub(crate) fn line_of(&self, offset: SourceOffset) -> LineIndex {
        LineIndex(self.rope.byte_to_line_idx(offset.min(self.end()).get(), LF))
    }

    /// Without its line break.
    pub(crate) fn line_range(&self, line: LineIndex) -> Range<SourceOffset> {
        let start = SourceOffset::new(self.rope.line_to_byte_idx(line.0, LF));
        let end = if line < self.last_line() {
            SourceOffset::new(self.rope.line_to_byte_idx(line.next().0, LF) - 1)
        } else {
            self.end()
        };
        start..end
    }

    fn syntax_of(&self, range: &Range<SourceOffset>) -> Syntax {
        let selection = self.selection();
        if range.start <= selection.end && selection.start <= range.end {
            Syntax::Shown
        } else {
            Syntax::Hidden
        }
    }

    pub(crate) fn offset_to_utf16(&self, offset: SourceOffset) -> Utf16Offset {
        Utf16Offset(self.rope.byte_to_utf16_idx(offset.min(self.end()).get()))
    }

    pub(crate) fn utf16_to_offset(&self, utf16: Utf16Offset) -> SourceOffset {
        SourceOffset::new(self.rope.utf16_to_byte_idx(utf16.0.min(self.rope.len_utf16())))
    }
}

/// The index of a line of the text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct LineIndex(usize);

impl LineIndex {
    pub(crate) const FIRST: Self = Self(0);

    pub(crate) fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// `None` on the first line.
    pub(crate) fn previous(self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }

    /// The lines from this one to `last`, with both.
    pub(crate) fn through(self, last: Self) -> impl Iterator<Item = Self> {
        (self.0..=last.0).map(Self)
    }
}

/// An offset in UTF-16 code units, as the platform text input counts. The text counts bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Utf16Offset(usize);

impl Utf16Offset {
    pub(crate) fn range_from_platform(range: Range<usize>) -> Range<Self> {
        Self(range.start)..Self(range.end)
    }

    pub(crate) fn range_to_platform(range: Range<Self>) -> Range<usize> {
        range.start.0..range.end.0
    }

    pub(crate) fn to_platform(self) -> usize {
        self.0
    }
}

#[cfg(test)]
mod tests;
