//! The draft: the unsaved editor text of the open note, and of the notes the editor left.
//!
//! The draft keeps each text until a write of that text succeeds. A failed write keeps it, so the next job writes it
//! again. When the draft takes text of another note, it gives the write of the old text first. Only
//! [`Draft::drop_text`] drops a text with no write. Each text follows the moves of its note, and an outside delete
//! makes it a restore of the note.
//!
//! A disk question holds the text of its note: the draft gives no write of it until [`Draft::release`] or
//! [`Draft::drop_text`]. A write that the core refused after an outside change holds its text in the same way:
//! [`Draft::refused`].
//!
//! A read of a note file can replace its text: [`Draft::drop_for_read`]. The text gives no write while the read runs,
//! and it comes back when the read fails.

use holm_core::model::{Change, Note, NoteInfo, Source};
use holm_core::{Core, NotePath, Revision, Snapshot};

/// The editor text of a note, for a write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum NoteText {
    /// The note keeps its other facts.
    Save { path: NotePath, body: String },
    /// The note after another tool deleted its file: its last facts and its body.
    Restore { info: NoteInfo, body: String },
}

impl NoteText {
    fn path(&self) -> &NotePath {
        match self {
            Self::Save { path, .. } => path,
            Self::Restore { info, .. } => &info.path,
        }
    }

    fn path_mut(&mut self) -> &mut NotePath {
        match self {
            Self::Save { path, .. } => path,
            Self::Restore { info, .. } => &mut info.path,
        }
    }

    fn body_mut(&mut self) -> &mut String {
        match self {
            Self::Save { body, .. } | Self::Restore { body, .. } => body,
        }
    }

    /// With `seen`, the core refuses a save over an outside change after that revision.
    pub(super) fn write(self, core: &Core, seen: Option<Revision>) -> holm_core::Result<()> {
        match self {
            Self::Save { path, body } => core.save(&path, &body, seen).map(drop),
            Self::Restore { info, body } => core.restore(&Note { info, body }).map(drop),
        }
    }
}

/// A count of the bodies the draft took. It grows with each new body.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DraftVersion(u64);

impl DraftVersion {
    fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// One write of the draft. Its id and version tell the draft which text it wrote.
pub(super) struct DraftSave {
    pub(super) id: DraftVersion,
    pub(super) version: DraftVersion,
    pub(super) text: NoteText,
}

/// What the end of a write means for the user.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Written {
    Saved,
    /// The write failed, and the draft keeps the text. It is `first` when no other text of the draft had a failed
    /// write.
    Failed {
        first: bool,
    },
    /// The core refused the write of the open text after an outside change. The draft holds the text, as a disk
    /// question does.
    Changed,
    /// The end tells the user nothing. The draft dropped the text, or the open text gives its own end.
    Stale,
}

/// One text of the draft, and the state of its writes.
struct Text {
    note: NoteText,
    /// The version at which the draft took the note. The end of a write finds its text by it.
    id: DraftVersion,
    /// The version of the body.
    version: DraftVersion,
    /// The version of the save in the write line, until it ends.
    writing: Option<DraftVersion>,
    /// The reason why the last write of the text failed.
    failed: Option<&'static str>,
    /// A disk question holds the text, so it gives no write.
    held: bool,
}

impl Text {
    /// The save to put in the write line. It is `None` when the line already holds a save of this body.
    fn save(&mut self) -> Option<DraftSave> {
        if self.held || self.writing == Some(self.version) {
            return None;
        }
        self.writing = Some(self.version);
        Some(DraftSave {
            id: self.id,
            version: self.version,
            text: self.note.clone(),
        })
    }

    fn follow(&mut self, source: Source, change: &Change, before: &Snapshot) {
        if let Some(new) = change.note_after(self.note.path()) {
            *self.note.path_mut() = new;
            return;
        }
        let Change::Removed(gone) = change else {
            return;
        };
        if source != Source::Outside || gone != self.note.path() {
            return;
        }
        if let NoteText::Save { path, body } = &mut self.note
            && let Some(info) = before.note(path)
        {
            self.note = NoteText::Restore {
                info: info.clone(),
                body: std::mem::take(body),
            };
        }
    }
}

#[derive(Default)]
pub(super) struct Draft {
    /// The text of the note that the editor took last.
    open: Option<Text>,
    /// The texts of the notes that the editor left, until a write of each succeeds.
    left: Vec<Text>,
    /// The texts that a read of their file replaces, until the read ends.
    reading: Vec<Text>,
    version: DraftVersion,
}

impl Draft {
    /// The text of the note at `path` is now `body`. A restore of the note stays a restore. When the open text is of
    /// another note, it gives the write of that text, and the caller puts it in the write line.
    #[must_use = "the write of the old text goes in the write line"]
    pub(super) fn set(&mut self, path: NotePath, body: String) -> Option<DraftSave> {
        self.version = self.version.next();
        let version = self.version;
        if let Some(open) = self.open.as_mut().filter(|open| *open.note.path() == path) {
            *open.note.body_mut() = body;
            open.version = version;
            return None;
        }
        let mut new = match self.left.iter().position(|text| *text.note.path() == path) {
            Some(at) => self.left.remove(at),
            None => Text {
                note: NoteText::Save {
                    path,
                    body: String::new(),
                },
                id: version,
                version,
                writing: None,
                failed: None,
                held: false,
            },
        };
        *new.note.body_mut() = body;
        new.version = version;
        let mut old = self.open.replace(new)?;
        let save = old.save();
        self.left.push(old);
        save
    }

    /// Drops the text of the note at `path` with no write.
    pub(super) fn drop_text(&mut self, path: &NotePath) {
        if self.open.as_ref().is_some_and(|open| open.note.path() == path) {
            self.open = None;
        }
        self.left.retain(|text| text.note.path() != path);
        self.reading.retain(|text| text.note.path() != path);
    }

    /// Takes out the text of the note at `path` for a read of its file, and gives the id of the text. Until
    /// [`Self::read_ended`], the text gives no write and is not unsaved.
    pub(super) fn drop_for_read(&mut self, path: &NotePath) -> Option<DraftVersion> {
        let text = if self.open.as_ref().is_some_and(|open| open.note.path() == path) {
            self.open.take()
        } else {
            let at = self.left.iter().position(|text| text.note.path() == path)?;
            Some(self.left.remove(at))
        }?;
        let id = text.id;
        self.reading.push(text);
        Some(id)
    }

    /// The read for the text `id` ended. A failed read gives the text back, with its hold. When a newer text of the
    /// note came during the read, the newer text stays and takes the hold.
    pub(super) fn read_ended(&mut self, id: DraftVersion, read: &holm_core::Result<Note>) {
        let Some(at) = self.reading.iter().position(|text| text.id == id) else {
            return;
        };
        let text = self.reading.remove(at);
        if read.is_ok() {
            return;
        }
        if let Some(newer) = self.text_mut(text.note.path()) {
            newer.held |= text.held;
        } else if self.open.is_none() {
            self.open = Some(text);
        } else {
            self.left.push(text);
        }
    }

    /// Holds the text of the note at `path` while its disk question is open.
    pub(super) fn hold(&mut self, path: &NotePath) {
        if let Some(text) = self.text_mut(path) {
            text.held = true;
        }
    }

    /// Ends the hold of the text of the note at `path`, so the next writes give it.
    pub(super) fn release(&mut self, path: &NotePath) {
        if let Some(text) = self.text_mut(path) {
            text.held = false;
        }
    }

    /// A change of the text changes it.
    pub(super) fn version(&self) -> DraftVersion {
        self.version
    }

    /// Whether the draft has text that is not on disk.
    pub(super) fn unsaved(&self) -> bool {
        self.open.is_some() || !self.left.is_empty()
    }

    /// Whether the draft has text of the note at `path` that is not on disk.
    pub(super) fn unsaved_note(&self, path: &NotePath) -> bool {
        self.texts().any(|text| text.note.path() == path)
    }

    /// Whether a text is not on disk, because its last write failed.
    fn failed(&self) -> bool {
        self.texts().any(|text| text.failed.is_some())
    }

    /// Why a text is not on disk: the reason of its failed write. The reason of the open text wins.
    pub(super) fn not_saved(&self) -> Option<&'static str> {
        self.texts().filter_map(|text| text.failed).last()
    }

    /// Whether a text stays off the disk after the writes in the line: its last write failed, or a disk question holds
    /// it.
    pub(super) fn kept_back(&self) -> bool {
        self.texts().any(|text| text.failed.is_some() || text.held)
    }

    /// The saves to put in the write line before a job, the left texts first. A text whose save is already in the line
    /// gives none.
    pub(super) fn writes(&mut self) -> Vec<DraftSave> {
        self.left
            .iter_mut()
            .chain(&mut self.open)
            .filter_map(Text::save)
            .collect()
    }

    /// Takes the end of the write of `version` of the text `id`, with the reason of its `failure`. The text goes only
    /// when this write saved it and no newer body came. A saved left text tells the user only when no other text waits.
    pub(super) fn written(
        &mut self,
        id: DraftVersion,
        version: DraftVersion,
        failure: Option<&'static str>,
    ) -> Written {
        let first = !self.failed();
        let is_open = self.open.as_ref().is_some_and(|open| open.id == id);
        let text = if is_open {
            self.open.as_mut()
        } else {
            self.left.iter_mut().find(|text| text.id == id)
        };
        let Some(text) = text else {
            return Written::Stale;
        };
        if text.writing == Some(version) {
            text.writing = None;
        }
        text.failed = failure;
        if failure.is_some() {
            return Written::Failed { first };
        }
        if version == text.version {
            if is_open {
                self.open = None;
            } else {
                self.left.retain(|text| text.id != id);
            }
        }
        if is_open || (self.open.is_none() && !self.failed()) {
            Written::Saved
        } else {
            Written::Stale
        }
    }

    /// Takes the end of the write of `version` of the text `id`, which the core refused after an outside change. The
    /// text stays, and the draft holds it until [`Self::release`].
    pub(super) fn refused(&mut self, id: DraftVersion, version: DraftVersion) -> Written {
        let is_open = self.open.as_ref().is_some_and(|open| open.id == id);
        let text = self.left.iter_mut().chain(&mut self.open).find(|text| text.id == id);
        let Some(text) = text else {
            return Written::Stale;
        };
        if text.writing == Some(version) {
            text.writing = None;
        }
        text.held = true;
        if is_open { Written::Changed } else { Written::Stale }
    }

    /// `before` is the snapshot before `change`.
    pub(super) fn follow(&mut self, source: Source, change: &Change, before: &Snapshot) {
        let texts = self.left.iter_mut().chain(&mut self.open);
        for text in texts.chain(&mut self.reading) {
            text.follow(source, change, before);
        }
    }

    /// At quit: takes every text for its last write, the left texts first.
    pub(super) fn take(&mut self) -> Vec<NoteText> {
        self.left
            .drain(..)
            .chain(self.open.take())
            .map(|text| text.note)
            .collect()
    }

    fn texts(&self) -> impl Iterator<Item = &Text> {
        self.left.iter().chain(&self.open)
    }

    fn text_mut(&mut self, path: &NotePath) -> Option<&mut Text> {
        self.left
            .iter_mut()
            .chain(&mut self.open)
            .find(|text| text.note.path() == path)
    }
}

#[cfg(test)]
mod tests;
