use std::ops::Range;

pub(crate) use holm_markdown::inline::Format;
use holm_markdown::inline::{Edit as MarkdownEdit, restyle};
use holm_markdown::list::{Move, relist};
use holm_markdown::{Block, BlockKind, NewOffset, OffsetRange, SourceOffset};

use super::{Edit, Group, Model, map_offset};

impl Model {
    /// With no selection, it works at the cursor. A color key sets its color. Returns whether the text changed.
    pub(crate) fn toggle_format(&mut self, format: Format) -> bool {
        let Some(edit) = restyle(&self.text, &self.body, self.selection(), format) else {
            return false;
        };
        self.apply(edit)
    }

    /// The selection keeps its direction.
    fn apply(&mut self, edit: MarkdownEdit) -> bool {
        let ends = if self.is_reversed() {
            (edit.selection.end, edit.selection.start)
        } else {
            (edit.selection.start, edit.selection.end)
        };
        // Once the edit is made, the new text is the source.
        let in_source = |offset: NewOffset| SourceOffset::new(offset.get());
        let ends = (in_source(ends.0), in_source(ends.1));
        self.edit(vec![(edit.range, edit.text)], ends, Group::Alone, None)
    }
}

/// The ATX heading level of `line`, and the length of its `#`s and the spaces after them.
fn heading_prefix(line: &str) -> (u8, usize) {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    let Ok(level @ 1..=6) = u8::try_from(hashes) else {
        return (0, 0);
    };
    let rest = &line[hashes..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return (0, 0);
    }
    let spaces = rest.bytes().take_while(|b| *b == b' ').count();
    (level, hashes + spaces)
}

impl Model {
    /// Makes each line the selection touches a heading of `level`, or a plain line with 0. When every such line already
    /// has `level`, they become plain lines. Returns whether the text changed.
    pub(crate) fn toggle_heading(&mut self, level: u8) -> bool {
        let selection = self.selection();
        let first = self.line_of(selection.start);
        let last = self.line_of(selection.end);
        let lines: Vec<_> = first
            .through(last)
            .map(|line| self.line_range(line))
            .filter(|line| first == last || !line.is_empty())
            .map(|line| {
                let prefix = heading_prefix(&self.text[line.bytes()]);
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
            .map(|(line, (_, len))| (line.start..line.start + *len, prefix.clone()))
            .collect();
        let anchor = map_offset(&edits, self.anchor);
        let cursor = map_offset(&edits, self.cursor());
        self.edit(edits, (anchor, cursor), Group::Alone, None)
    }

    /// Enter: in a list item it starts the next item and counts the list again, and on an empty item it ends the list.
    /// Elsewhere it types a line break. Returns whether the text changed.
    pub(crate) fn newline(&mut self, now: std::time::Instant) -> bool {
        self.relist(Move::Next).unwrap_or_else(|| self.type_text("\n", now))
    }

    /// A list key, as [`Move`] tells. It counts the lists it changes again. Returns `None` when the selection touches
    /// no list item.
    pub(crate) fn relist(&mut self, key: Move) -> Option<bool> {
        let edit = relist(&self.text, &self.body, self.selection(), key)?;
        Some(self.apply(edit))
    }

    /// `range` is the source of the `[ ]`. The cursor stays. Returns whether the text changed.
    pub(crate) fn toggle_task(&mut self, range: &Range<SourceOffset>) -> bool {
        self.toggle_task_boxes(std::slice::from_ref(range))
    }

    /// Checks or clears the task boxes on the lines that the selection touches. Returns whether the text changed.
    pub(crate) fn toggle_tasks(&mut self) -> bool {
        let selection = self.selection();
        let lines =
            self.line_range(self.line_of(selection.start)).start..self.line_range(self.line_of(selection.end)).end;
        let mut boxes = Vec::new();
        collect_tasks(self.body.blocks(), &lines, &mut boxes);
        self.toggle_task_boxes(&boxes)
    }

    fn toggle_task_boxes(&mut self, boxes: &[Range<SourceOffset>]) -> bool {
        let edits: Vec<Edit> = boxes
            .iter()
            .filter(|range| {
                range.end - range.start == 3 && self.text.get((range.start..range.start + 1).bytes()) == Some("[")
            })
            .map(|range| {
                let mark = &self.text[(range.start + 1..range.start + 2).bytes()];
                let new = if mark == " " { "x" } else { " " };
                (range.start + 1..range.start + 2, new.to_owned())
            })
            .collect();
        self.edit(edits, self.ends(), Group::Alone, None)
    }
}

/// The `[ ]` sources of the task items that start in `lines`, in order.
fn collect_tasks(blocks: &[Block], lines: &Range<SourceOffset>, boxes: &mut Vec<Range<SourceOffset>>) {
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
mod tests;
