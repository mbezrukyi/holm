//! The notes that a popup picks from, with each title in lowercase. The popup makes them once
//! when it opens, so a key does not lowercase each title again.

use holm_core::model::NoteInfo;

use crate::lowercase::{Lowercase, LowercaseList};

pub(crate) struct PickNotes {
    notes: Vec<NoteInfo>,
    titles: LowercaseList,
}

impl PickNotes {
    /// The notes keep the order of `notes`.
    pub(crate) fn new(notes: Vec<NoteInfo>) -> Self {
        let titles = LowercaseList::new(notes.iter().map(|note| note.title.as_str()));
        Self { notes, titles }
    }

    pub(crate) fn get(&self, at: usize) -> Option<&NoteInfo> {
        self.notes.get(at)
    }

    /// In the order of the notes.
    pub(crate) fn titles(&self) -> impl Iterator<Item = Lowercase<'_>> {
        self.titles.iter()
    }

    /// The place of the note whose title is `text`.
    pub(crate) fn position(&self, text: &Lowercase<'_>) -> Option<usize> {
        self.titles().position(|title| title.as_str() == text.as_str())
    }
}

impl std::ops::Index<usize> for PickNotes {
    type Output = NoteInfo;

    fn index(&self, at: usize) -> &NoteInfo {
        &self.notes[at]
    }
}
