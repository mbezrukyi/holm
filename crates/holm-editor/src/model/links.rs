use std::collections::HashSet;
use std::ops::Range;

use holm_markdown::{Link, LinkKind, OffsetRange, SourceOffset, Target, escape_text};

use super::{Group, Model, map_offset};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinkForm {
    /// `[..](..)` at the cursor.
    Text,
    /// `![..](..)` at the cursor, which shows a file chip. It gets a space before it and after it, where the text
    /// around it needs one.
    Inline,
    /// `![..](..)` on a line of its own, which shows an image. The cursor goes to the next line.
    Block,
}

impl Model {
    pub(crate) fn set_broken(&mut self, broken: HashSet<Target>) {
        self.broken = broken;
    }

    /// To keep them for a new model.
    pub(crate) fn take_broken(&mut self) -> HashSet<Target> {
        std::mem::take(&mut self.broken)
    }

    /// Whether a link to `target` gets the broken style. A heading in this note is broken when no heading has its text.
    /// Other targets are broken when the app says so.
    pub(crate) fn is_broken(&self, target: &Target) -> bool {
        match target {
            Target::Heading(name) => self.heading_start(name).is_none(),
            _ => self.broken.contains(target),
        }
    }

    pub(crate) fn heading_start(&self, name: &str) -> Option<SourceOffset> {
        self.body
            .headings()
            .iter()
            .find(|heading| heading.text == name)
            .map(|heading| heading.range.start)
    }

    /// The innermost link whose source holds `offset`. With none, a link that ends at `offset`.
    pub(crate) fn link_at(&self, offset: SourceOffset) -> Option<&Link> {
        let links = self.body.links();
        let before = &links[..links.partition_point(|l| l.range.start <= offset)];
        // Links are in order of their start, so the innermost one comes first from the back.
        before
            .iter()
            .rev()
            .find(|l| offset < l.range.end)
            .or_else(|| before.iter().rev().find(|l| l.range.end == offset))
    }

    /// The link that holds the link text `piece` of a line that starts at `line_start`.
    pub(super) fn link_over(&self, piece: &Range<SourceOffset>, line_start: SourceOffset) -> Option<&Link> {
        let links = self.body.links();
        links[..links.partition_point(|l| l.range.start <= piece.start)]
            .iter()
            .rev()
            .take_while(|l| l.range.end >= line_start)
            .find(|l| l.kind == LinkKind::Plain && piece.end <= l.range.end)
    }

    /// The selected source becomes the link text. With no selection, the link text is `text`.
    pub(crate) fn insert_link(&mut self, text: &str, target: &Target, form: LinkForm) -> bool {
        let range = self.selection();
        let label = if range.is_empty() {
            escape_text(text)
        } else {
            self.text[range.bytes()].to_owned()
        };
        let bang = if form == LinkForm::Text { "" } else { "!" };
        let link = format!("{bang}[{label}]({})", target.to_destination());
        let before = self.text[..range.start.get()].chars().next_back();
        let after = self.text[range.end.get()..].chars().next();
        let (start, end, cursor_after) = match form {
            LinkForm::Text => ("", "", 0),
            LinkForm::Inline => (
                if before.is_some_and(|c| !c.is_whitespace()) {
                    " "
                } else {
                    ""
                },
                if after.is_some_and(char::is_alphanumeric) {
                    " "
                } else {
                    ""
                },
                0,
            ),
            LinkForm::Block => (
                if before.is_none_or(|c| c == '\n') { "" } else { "\n" },
                if after == Some('\n') { "" } else { "\n" },
                1,
            ),
        };
        let cursor = range.start + start.len() + link.len() + cursor_after;
        self.edit(
            vec![(range, format!("{start}{link}{end}"))],
            (cursor, cursor),
            Group::Alone,
            None,
        )
    }

    /// Writes `target` in place of the target of the link at the cursor. The link text stays. It does nothing for a
    /// reference link, whose target is in its definition.
    pub(crate) fn set_link_target(&mut self, target: &Target) -> bool {
        let Some(range) = self.link_at(self.cursor()).and_then(|link| link.target_range.clone()) else {
            return false;
        };
        let edits = vec![(range, target.to_destination())];
        let (anchor, cursor) = self.ends();
        let ends = (map_offset(&edits, anchor), map_offset(&edits, cursor));
        self.edit(edits, ends, Group::Alone, None)
    }

    /// Each place where `query` is in the text, in order and apart. Case does not matter.
    pub(crate) fn find_all(&self, query: &str) -> Vec<Range<SourceOffset>> {
        let query: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
        let Some(&first) = query.first() else {
            return Vec::new();
        };
        let mut found = Vec::new();
        let mut next = 0;
        for (at, c) in self.text.char_indices() {
            if at < next || (c.is_ascii() && first.is_ascii() && c.to_ascii_lowercase() != first) {
                continue;
            }
            if let Some(end) = self.match_at(at, &query) {
                found.push(SourceOffset::range(at..end));
                next = end;
            }
        }
        found
    }

    /// The end of `query` when the text at `start` matches it.
    fn match_at(&self, start: usize, query: &[char]) -> Option<usize> {
        let mut want = query.iter();
        for (at, c) in self.text[start..].char_indices() {
            for lower in c.to_lowercase() {
                if want.next() != Some(&lower) {
                    return None;
                }
            }
            if want.len() == 0 {
                return Some(start + at + c.len_utf8());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests;
