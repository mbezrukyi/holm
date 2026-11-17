//! The notes that a popup picks from, with each title in lowercase. The popup makes them once
//! when it opens, so a key does not lowercase each title again.

use std::borrow::Cow;

use holm_core::model::NoteInfo;

pub(crate) struct PickNotes {
    notes: Vec<NoteInfo>,
    /// All titles in lowercase in a row, so the popup opens with few allocations.
    titles: String,
    /// The end of each title in `titles`.
    ends: Vec<usize>,
}

impl PickNotes {
    /// The notes keep the order of `notes`.
    pub(crate) fn new(notes: Vec<NoteInfo>) -> Self {
        let mut titles = String::new();
        let mut ends = Vec::with_capacity(notes.len());
        for note in &notes {
            push_lowercase(&mut titles, &note.title);
            ends.push(titles.len());
        }
        Self {
            notes,
            titles,
            ends,
        }
    }

    pub(crate) fn get(&self, at: usize) -> Option<&NoteInfo> {
        self.notes.get(at)
    }

    /// The title of each note, in the order of the notes.
    pub(crate) fn titles(&self) -> impl Iterator<Item = Lowercase<'_>> {
        let starts = std::iter::once(0).chain(self.ends.iter().copied());
        starts
            .zip(&self.ends)
            .map(|(start, end)| Lowercase(Cow::Borrowed(&self.titles[start..*end])))
    }

    /// The place of the note whose title is `text`.
    pub(crate) fn position(&self, text: &Lowercase<'_>) -> Option<usize> {
        self.titles()
            .position(|title| title.as_str() == text.as_str())
    }
}

impl std::ops::Index<usize> for PickNotes {
    type Output = NoteInfo;

    fn index(&self, at: usize) -> &NoteInfo {
        &self.notes[at]
    }
}

/// Text in lowercase. Two of them compare with no regard to capital letters.
pub(crate) struct Lowercase<'a>(Cow<'a, str>);

impl Lowercase<'static> {
    pub(crate) fn new(text: &str) -> Self {
        let mut lower = String::with_capacity(text.len());
        push_lowercase(&mut lower, text);
        Self(Cow::Owned(lower))
    }
}

impl Lowercase<'_> {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

fn push_lowercase(to: &mut String, text: &str) {
    if text.is_ascii() {
        let start = to.len();
        to.push_str(text);
        to[start..].make_ascii_lowercase();
    } else {
        to.extend(text.chars().flat_map(char::to_lowercase));
    }
}
