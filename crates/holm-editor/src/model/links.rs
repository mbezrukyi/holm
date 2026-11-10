//! Links in the text, and find.

use std::collections::HashSet;
use std::ops::Range;

use holm_markdown::{Link, Target, escape_text};

use super::{Group, Model, map_offset};

impl Model {
    /// Sets the targets that the app knows are broken.
    pub(crate) fn set_broken(&mut self, broken: HashSet<Target>) {
        self.broken = broken;
    }

    /// Takes the broken targets out, to keep them for a new model.
    pub(crate) fn take_broken(&mut self) -> HashSet<Target> {
        std::mem::take(&mut self.broken)
    }

    /// Whether a link to `target` gets the broken style. A heading in this note is broken when
    /// no heading has its text. Other targets are broken when the app says so.
    pub(crate) fn is_broken(&self, target: &Target) -> bool {
        match target {
            Target::Heading(name) => self.heading_start(name).is_none(),
            _ => self.broken.contains(target),
        }
    }

    /// The start of the first heading with the text `name`.
    pub(crate) fn heading_start(&self, name: &str) -> Option<usize> {
        self.body
            .headings()
            .iter()
            .find(|heading| heading.text == name)
            .map(|heading| heading.range.start)
    }

    /// The innermost link whose source holds `offset`. With none, a link that ends at `offset`.
    pub(crate) fn link_at(&self, offset: usize) -> Option<&Link> {
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
    pub(super) fn link_over(&self, piece: &Range<usize>, line_start: usize) -> Option<&Link> {
        let links = self.body.links();
        links[..links.partition_point(|l| l.range.start <= piece.start)]
            .iter()
            .rev()
            .take_while(|l| l.range.end >= line_start)
            .find(|l| !l.embedded && piece.end <= l.range.end)
    }

    /// Puts a link to `target` in place of the selection, and the cursor after it. The selected
    /// source becomes the link text. With no selection, the link text is `text`.
    /// `embedded` writes `![..](..)`.
    pub(crate) fn insert_link(&mut self, text: &str, target: &Target, embedded: bool) -> bool {
        let range = self.selection();
        let label = if range.is_empty() {
            escape_text(text)
        } else {
            self.text[range.clone()].to_owned()
        };
        let bang = if embedded { "!" } else { "" };
        let source = format!("{bang}[{label}]({})", target.to_destination());
        let cursor = range.start + source.len();
        self.edit(vec![(range, source)], (cursor, cursor), Group::Alone, None)
    }

    /// Writes `target` in place of the target of the link at the cursor. The link text stays.
    /// It does nothing for a reference link, whose target is in its definition.
    pub(crate) fn set_link_target(&mut self, target: &Target) -> bool {
        let Some(range) = self
            .link_at(self.cursor)
            .and_then(|link| link.target_range.clone())
        else {
            return false;
        };
        let edits = vec![(range, target.to_destination())];
        let (anchor, cursor) = self.ends();
        let ends = (map_offset(&edits, anchor), map_offset(&edits, cursor));
        self.edit(edits, ends, Group::Alone, None)
    }

    /// Each place where `query` is in the text, in order and apart. Case does not matter.
    pub(crate) fn find_all(&self, query: &str) -> Vec<Range<usize>> {
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
                found.push(at..end);
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
mod tests {
    use super::*;

    fn model(text: &str, selection: Range<usize>) -> Model {
        let mut model = Model::new(text);
        model.select(selection);
        model
    }

    fn note(id: &str, heading: Option<&str>) -> Target {
        Target::Note {
            id: id.to_owned(),
            heading: heading.map(str::to_owned),
        }
    }

    #[test]
    fn the_link_at_an_offset_is_the_innermost_one() {
        let text = "a [![img](i.png) b](holm:X) [c](d.md)";
        let model = Model::new(text);
        assert_eq!(model.link_at(0), None);
        assert_eq!(
            model.link_at(5).map(|l| &l.target),
            Some(&Target::File("i.png".into()))
        );
        assert_eq!(model.link_at(18).map(|l| &l.target), Some(&note("X", None)));
        // The end of a link counts when no other link starts there.
        assert_eq!(model.link_at(27).map(|l| &l.target), Some(&note("X", None)));
        assert_eq!(
            model.link_at(28).map(|l| &l.target),
            Some(&Target::File("d.md".into()))
        );
    }

    #[test]
    fn a_new_link_takes_the_selection_as_its_text() {
        let mut m = model("see plans here", 4..9);
        assert!(m.insert_link("Plans", &note("X", Some("Bug list")), false));
        assert_eq!(m.text(), "see [plans](<holm:X#Bug list>) here");
        assert_eq!(m.selection(), 30..30);
    }

    #[test]
    fn a_new_link_with_no_selection_escapes_its_text() {
        let mut m = model("x ", 2..2);
        assert!(m.insert_link("A *bold* [idea]", &note("X", None), false));
        assert_eq!(m.text(), r"x [A \*bold\* \[idea\]](holm:X)");
        let mut m = model("", 0..0);
        assert!(m.insert_link("photo", &Target::File("attachments/a b.png".into()), true));
        assert_eq!(m.text(), "![photo](<attachments/a b.png>)");
        assert!(m.undo());
        assert_eq!(m.text(), "");
    }

    #[test]
    fn a_new_target_keeps_the_link_text_and_the_cursor() {
        let mut m = model("go [there](https://a.org) now", 6..6);
        assert!(m.set_link_target(&note("X", None)));
        assert_eq!(m.text(), "go [there](holm:X) now");
        assert_eq!(m.cursor(), 6);
        let mut m = model("[a][ref]\n\n[ref]: b.md\n", 1..1);
        assert!(!m.set_link_target(&note("X", None)));
        let mut m = model("no link", 1..1);
        assert!(!m.set_link_target(&note("X", None)));
    }

    #[test]
    fn same_note_heading_links_are_broken_without_their_heading() {
        let mut m = Model::new("# Plans\n\n## Bug list\n");
        assert!(!m.is_broken(&Target::Heading("Bug list".into())));
        assert!(m.is_broken(&Target::Heading("bug list".into())));
        assert!(!m.is_broken(&note("X", Some("a"))));
        m.set_broken(HashSet::from([note("X", Some("a"))]));
        assert!(m.is_broken(&note("X", Some("a"))));
        assert_eq!(m.heading_start("Bug list"), Some(9));
    }

    #[test]
    fn broken_links_get_their_own_look() {
        let mut m = model("[a](holm:X#b) [c](#d) [e](holm:Y)\n\n# d\n", 40..40);
        m.set_broken(HashSet::from([note("X", Some("b"))]));
        let looks: Vec<_> = m
            .display_line(0)
            .shown
            .runs
            .iter()
            .filter(|run| run.look.link)
            .map(|run| run.look.broken)
            .collect();
        assert_eq!(looks, [true, false, false]);
    }

    #[test]
    fn find_ignores_case_and_gives_separate_matches() {
        let m = Model::new("Aaa ÄB äb aaa");
        assert_eq!(m.find_all("aa"), vec![0..2, 12..14]);
        assert_eq!(m.find_all("äb"), vec![4..7, 8..11]);
        assert_eq!(m.find_all(""), Vec::<Range<usize>>::new());
        assert_eq!(m.find_all("zz"), Vec::<Range<usize>>::new());
    }
}
