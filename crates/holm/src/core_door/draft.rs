//! The draft: the unsaved editor text of the open note.
//!
//! The draft keeps its text until a write of that text succeeds. A failed write keeps it, so
//! the next job writes it again. It follows the moves of its note, and an outside delete makes
//! it a restore of the note.

use holm_core::model::{Change, Note, NoteInfo, Source};
use holm_core::{Core, NotePath, Snapshot};

/// The editor text of a note, for a write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum NoteText {
    /// The new body of the note at `path`. The note keeps its other facts.
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

    /// Saves the body. A save keeps the other facts that the core has for the note.
    pub(super) fn write(self, core: &Core) -> holm_core::Result<()> {
        match self {
            Self::Save { path, body } => core.save(&path, &body).map(drop),
            Self::Restore { info, body } => core.restore(&Note { info, body }).map(drop),
        }
    }
}

/// One write of the draft. Its version tells the draft which text it wrote.
pub(super) struct DraftSave {
    pub(super) version: u64,
    pub(super) text: NoteText,
}

/// What the end of a write means for the user.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Written {
    /// The text of the write is on disk.
    Saved,
    /// The write failed, and the draft keeps the text.
    Failed,
    /// The draft dropped the text or took another note, so the end means nothing.
    Stale,
}

#[derive(Default)]
pub(super) struct Draft {
    text: Option<NoteText>,
    /// It grows with each new text.
    version: u64,
    /// The version at which the draft took its note. A write before it is of another text.
    since: u64,
    /// The version of the save in the write line, until it ends.
    writing: Option<u64>,
    /// The last write of the text failed.
    failed: bool,
}

impl Draft {
    /// The text of the note at `path` is now `body`. A restore of the note stays a restore.
    pub(super) fn set(&mut self, path: NotePath, body: String) {
        self.version += 1;
        match &mut self.text {
            Some(text) if *text.path() == path => *text.body_mut() = body,
            _ => {
                self.text = Some(NoteText::Save { path, body });
                self.since = self.version;
                self.failed = false;
            }
        }
    }

    /// Drops the text with no write.
    pub(super) fn drop_text(&mut self) {
        self.version += 1;
        self.since = self.version;
        self.text = None;
        self.failed = false;
    }

    /// Whether the draft has text that is not on disk.
    pub(super) fn unsaved(&self) -> bool {
        self.text.is_some()
    }

    /// Whether the text is not on disk, because its last write failed.
    pub(super) fn failed(&self) -> bool {
        self.failed
    }

    /// The save to put in the write line before a job. It is `None` with no text, or when the
    /// write line already holds a save of this text.
    pub(super) fn write(&mut self) -> Option<DraftSave> {
        let text = self.text.clone()?;
        if self.writing == Some(self.version) {
            return None;
        }
        self.writing = Some(self.version);
        Some(DraftSave {
            version: self.version,
            text,
        })
    }

    /// Takes the end of the write of `version`. The text goes only when this write saved it
    /// and no newer text came.
    pub(super) fn written(&mut self, version: u64, ok: bool) -> Written {
        if self.writing == Some(version) {
            self.writing = None;
        }
        if self.text.is_none() || version < self.since {
            return Written::Stale;
        }
        if ok {
            if version == self.version {
                self.text = None;
            }
            self.failed = false;
            Written::Saved
        } else {
            self.failed = true;
            Written::Failed
        }
    }

    /// Follows `change` from `source`. `before` is the snapshot before it.
    pub(super) fn follow(&mut self, source: Source, change: &Change, before: &Snapshot) {
        let Some(text) = &mut self.text else {
            return;
        };
        if let Some(new) = change.note_after(text.path()) {
            *text.path_mut() = new;
            return;
        }
        let Change::Removed(gone) = change else {
            return;
        };
        if source != Source::Outside || gone != text.path() {
            return;
        }
        if let NoteText::Save { path, body } = text
            && let Some(info) = before.note(path)
        {
            *text = NoteText::Restore {
                info: info.clone(),
                body: std::mem::take(body),
            };
        }
    }

    /// At quit: takes the text for its last write.
    pub(super) fn take(&mut self) -> Option<NoteText> {
        self.text.take()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;
    use crate::test_paths::note_path;

    fn save(path: &str, body: &str) -> NoteText {
        NoteText::Save {
            path: note_path(path),
            body: body.to_owned(),
        }
    }

    #[test]
    fn the_line_holds_one_write_of_a_text() {
        let mut draft = Draft::default();
        assert!(draft.write().is_none(), "no text");
        draft.set(note_path("A.md"), "one".into());
        let write = draft.write().unwrap();
        assert_eq!(write.text, save("A.md", "one"));
        assert!(draft.write().is_none());

        assert_eq!(draft.written(write.version, true), Written::Saved);
        assert!(!draft.unsaved());
        assert!(draft.write().is_none());
    }

    #[test]
    fn a_newer_text_stays_after_the_write_of_an_older_one() {
        let mut draft = Draft::default();
        draft.set(note_path("A.md"), "one".into());
        let write = draft.write().unwrap();
        draft.set(note_path("A.md"), "two".into());

        assert_eq!(draft.written(write.version, true), Written::Saved);
        assert!(draft.unsaved());
        assert_eq!(draft.write().unwrap().text, save("A.md", "two"));
    }

    #[test]
    fn a_failed_write_keeps_the_text_for_the_next_write() {
        let mut draft = Draft::default();
        draft.set(note_path("A.md"), "one".into());
        let write = draft.write().unwrap();

        assert_eq!(draft.written(write.version, false), Written::Failed);
        assert!(draft.unsaved());
        assert!(draft.failed());
        let again = draft.write().unwrap();
        assert_eq!(again.text, save("A.md", "one"));

        assert_eq!(draft.written(again.version, true), Written::Saved);
        assert!(!draft.failed());
    }

    #[test]
    fn the_end_of_a_write_of_dropped_text_means_nothing() {
        let mut draft = Draft::default();
        draft.set(note_path("A.md"), "one".into());
        let write = draft.write().unwrap();
        draft.drop_text();
        assert_eq!(draft.written(write.version, false), Written::Stale);
        assert!(!draft.failed());

        draft.set(note_path("A.md"), "one".into());
        let old = draft.write().unwrap();
        draft.set(note_path("B.md"), "two".into());
        assert_eq!(draft.written(old.version, false), Written::Stale);
        assert!(draft.unsaved());
    }

    #[test]
    fn the_text_follows_a_move_of_its_note() {
        let mut draft = Draft::default();
        draft.set(note_path("A.md"), "one".into());
        let moved = Change::Moved {
            from: note_path("A.md"),
            to: note_path("work/B.md"),
        };
        draft.follow(Source::Holm, &moved, &Snapshot::default());
        assert_eq!(draft.take(), Some(save("work/B.md", "one")));
    }
}
