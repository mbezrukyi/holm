//! The change list of a write: a diff of the facts before and after it, plus the moves of
//! the write.

use std::collections::HashMap;

use crate::facts::Facts;
use crate::model::{Change, ChangeList, NoteId, NoteInfo, Source};
use crate::paths::{FolderPath, NotePath};
use crate::reconcile::Settled;
use crate::snapshot::Snapshot;

/// What one write touched: the note paths, the folders and the moves it made. The diff
/// looks only there, so its cost grows with the write, not with the notes folder.
#[derive(Debug, Default)]
pub(crate) struct Touched {
    /// In no order, maybe with repeats.
    notes: Vec<NotePath>,
    /// The notes that were read, in no order.
    read: Vec<NotePath>,
    folders: Vec<FolderPath>,
    moves: Vec<Change>,
}

impl Touched {
    /// Adds every path that `settled` changes.
    pub(crate) fn settled(&mut self, settled: &Settled) {
        self.notes.extend(settled.removed.iter().cloned());
        for moved in &settled.moved {
            self.notes.push(moved.from.clone());
            self.notes.push(moved.info.path.clone());
        }
        for put in &settled.put {
            self.notes.push(put.note.info.path.clone());
            self.read.push(put.note.info.path.clone());
        }
    }

    /// Adds the folders at `folder` or below it.
    pub(crate) fn folder(&mut self, folder: FolderPath) {
        self.folders.push(folder);
    }

    /// Adds a move that the write made: [`Change::Moved`], [`Change::FolderMoved`] or
    /// [`Change::TagMoved`].
    pub(crate) fn moved(&mut self, change: Change) {
        self.moves.push(change);
    }

    /// The facts after the write, and its change list from `source`. With a change, the
    /// facts get the next revision, and the list carries its snapshot. With no change, there is
    /// no list, and the facts keep their revision. From outside, each note that was read with a
    /// new content hash gets the revision of the facts after the write as its outside revision.
    ///
    /// The list holds the moves of the write first, then the note changes in path order,
    /// then [`Change::Folders`] and [`Change::Tags`]. A note that a move of the write
    /// explains gives no other change. A gone note and a new note with the same id are one
    /// [`Change::Moved`]. A note with a new content hash is [`Change::Changed`], also when
    /// what the snapshot shows of it stays.
    pub(crate) fn list(mut self, source: Source, before: &Facts, after: Facts) -> (Facts, Option<ChangeList>) {
        let mut read = std::mem::take(&mut self.read);
        read.retain(|path| before.hash(path) != after.hash(path));
        let changes = self.changes(before, &after);
        let mut after = if changes.is_empty() {
            after
        } else {
            after.next_revision()
        };
        if source == Source::Outside {
            let revision = after.snapshot().revision();
            after.mark_outside(&read, revision);
        }
        if changes.is_empty() {
            return (after, None);
        }
        let list = ChangeList {
            source,
            snapshot: after.snapshot().clone(),
            changes,
        };
        (after, Some(list))
    }

    fn changes(mut self, before_facts: &Facts, after_facts: &Facts) -> Vec<Change> {
        let (before, after) = (before_facts.snapshot(), after_facts.snapshot());
        self.notes.sort_unstable();
        self.notes.dedup();
        let mut tags = false;
        let mut gone: Vec<&NoteInfo> = Vec::new();
        let mut new: Vec<&NoteInfo> = Vec::new();
        let mut notes = Vec::new();
        for path in &self.notes {
            match (before.note_and_links(path), after.note_and_links(path)) {
                (Some((old, _)), None) => gone.push(old),
                (None, Some((note, _))) => new.push(note),
                (Some((old, old_links)), Some((note, links))) => {
                    if old != note || old_links != links || before_facts.hash(path) != after_facts.hash(path) {
                        tags = tags || !same_tags(old, note);
                        notes.push(Change::Changed(path.clone()));
                    }
                }
                (None, None) => {}
            }
        }
        let mut new: HashMap<&NotePath, &NoteInfo> = new.into_iter().map(|note| (&note.path, note)).collect();
        gone.retain(|old| {
            let Some(to) = self.moves.iter().find_map(|change| change.note_after(&old.path)) else {
                return true;
            };
            let Some(note) = new.remove(&to) else {
                return true;
            };
            tags = tags || !same_tags(old, note);
            false
        });
        let mut gone: Vec<Option<&NoteInfo>> = gone.into_iter().map(Some).collect();
        let mut ids: HashMap<&NoteId, usize> = HashMap::new();
        for (i, old) in gone.iter().enumerate() {
            if let Some(id) = old.and_then(|old| old.id.as_ref()) {
                ids.entry(id).or_insert(i);
            }
        }
        for note in new.into_values() {
            let from = note
                .id
                .as_ref()
                .and_then(|id| ids.remove(id))
                .and_then(|i| gone[i].take());
            if let Some(old) = from {
                tags = tags || !same_tags(old, note);
                notes.push(Change::Moved {
                    from: old.path.clone(),
                    to: note.path.clone(),
                });
            } else {
                tags = tags || !note.tags.is_empty();
                notes.push(Change::Added(note.path.clone()));
            }
        }
        for old in gone.into_iter().flatten() {
            tags = tags || !old.tags.is_empty();
            notes.push(Change::Removed(old.path.clone()));
        }
        notes.sort_by(|a, b| note_path(a).cmp(&note_path(b)));
        let folders = self.folders.iter().any(|folder| !same_folders(before, after, folder));
        let mut changes = self.moves;
        changes.extend(notes);
        if folders {
            changes.push(Change::Folders);
        }
        if tags {
            changes.push(Change::Tags);
        }
        changes
    }
}

/// Whether both notes have the same tags, in any order. A note has few tags.
fn same_tags(a: &NoteInfo, b: &NoteInfo) -> bool {
    a.tags == b.tags || (a.tags.iter().all(|tag| b.tags.contains(tag)) && b.tags.iter().all(|tag| a.tags.contains(tag)))
}

/// Whether both snapshots have the same folders at `folder` or below it.
fn same_folders(before: &Snapshot, after: &Snapshot, folder: &FolderPath) -> bool {
    let held = |other: &&FolderPath| *other == folder || other.is_below(folder);
    before.folders().filter(held).eq(after.folders().filter(held))
}

fn note_path(change: &Change) -> Option<&NotePath> {
    match change {
        Change::Added(path) | Change::Changed(path) | Change::Moved { to: path, .. } | Change::Removed(path) => {
            Some(path)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use std::time::SystemTime;

    use super::*;
    use crate::links::LinkSource;
    use crate::store::{ContentHash, Stamp};
    use crate::tag::TagName;

    const STAMP: Stamp = Stamp {
        modified: 1,
        changed: 1,
        size: 1,
    };

    fn path(path: &str) -> NotePath {
        NotePath::new(path).unwrap()
    }

    fn folder(path: &str) -> FolderPath {
        FolderPath::new(path).unwrap()
    }

    fn info(at: &str, id: Option<&str>, tags: &[&str]) -> NoteInfo {
        NoteInfo {
            path: path(at),
            id: id.map(|id| NoteId::from_field(id.to_owned())),
            title: String::new(),
            preview: String::new(),
            tags: tags.iter().map(|tag| TagName::new(tag).unwrap()).collect(),
            pinned: false,
            modified: SystemTime::UNIX_EPOCH,
        }
    }

    fn facts(notes: &[NoteInfo], folders: &[&str]) -> Facts {
        Facts::new(
            notes
                .iter()
                .map(|note| {
                    (
                        note.clone(),
                        Vec::new(),
                        STAMP,
                        ContentHash::default(),
                        LinkSource::default(),
                    )
                })
                .collect(),
            folders.iter().map(|at| folder(at)).collect(),
        )
    }

    /// The changes from `before` to `after` at `notes` and `folders`, with `moves`.
    fn changes(before: &Facts, after: Facts, notes: &[&str], folders: &[&str], moves: Vec<Change>) -> Vec<Change> {
        let touched = Touched {
            notes: notes.iter().map(|at| path(at)).collect(),
            read: Vec::new(),
            folders: folders.iter().map(|at| folder(at)).collect(),
            moves,
        };
        let (_, list) = touched.list(Source::Holm, before, after);
        list.map(|list| list.changes).unwrap_or_default()
    }

    #[test]
    fn no_change_gives_no_list_and_keeps_the_revision() {
        let before = facts(&[info("a.md", None, &[])], &["work"]);
        let touched = Touched {
            notes: vec![path("a.md")],
            read: Vec::new(),
            folders: vec![folder("work")],
            moves: Vec::new(),
        };

        let (after, list) = touched.list(Source::Holm, &before, before.clone());

        assert!(list.is_none());
        assert_eq!(after.snapshot().revision(), before.snapshot().revision());
    }

    #[test]
    fn a_list_carries_its_source_and_the_next_revision() {
        let before = facts(&[], &[]);
        let after = facts(&[info("a.md", None, &[])], &[]);
        let touched = Touched {
            notes: vec![path("a.md")],
            ..Touched::default()
        };

        let (after, list) = touched.list(Source::Outside, &before, after);

        let list = list.unwrap();
        assert_eq!(list.source, Source::Outside);
        assert_eq!(list.snapshot.revision(), before.snapshot().revision() + 1);
        assert_eq!(after.snapshot().revision(), list.snapshot.revision());
        assert_eq!(list.changes, [Change::Added(path("a.md"))]);
    }

    #[test]
    fn notes_come_in_path_order_and_tags_once() {
        let before = facts(&[info("b.md", None, &["x"]), info("c.md", None, &[])], &[]);
        let after = facts(&[info("a.md", None, &["y"]), info("c.md", None, &["x"])], &[]);

        assert_eq!(
            changes(&before, after, &["a.md", "b.md", "c.md"], &[], Vec::new()),
            [
                Change::Added(path("a.md")),
                Change::Removed(path("b.md")),
                Change::Changed(path("c.md")),
                Change::Tags,
            ]
        );
    }

    #[test]
    fn a_gone_note_and_a_new_note_with_one_id_are_a_move() {
        let before = facts(&[info("a.md", Some("A"), &[]), info("b.md", None, &[])], &[]);
        let after = facts(&[info("x/a.md", Some("A"), &[]), info("x/b.md", None, &[])], &[]);
        let notes = ["a.md", "b.md", "x/a.md", "x/b.md"];

        assert_eq!(
            changes(&before, after, &notes, &[], Vec::new()),
            [
                Change::Removed(path("b.md")),
                Change::Moved {
                    from: path("a.md"),
                    to: path("x/a.md"),
                },
                Change::Added(path("x/b.md")),
            ]
        );
    }

    #[test]
    fn a_move_of_the_write_explains_its_notes() {
        let before = facts(&[info("a.md", None, &[]), info("work/b.md", None, &["x"])], &["work"]);
        let after = facts(&[info("c.md", None, &[]), info("job/b.md", None, &["x"])], &["job"]);
        let moves = vec![
            Change::Moved {
                from: path("a.md"),
                to: path("c.md"),
            },
            Change::FolderMoved {
                from: folder("work"),
                to: folder("job"),
            },
        ];
        let notes = ["a.md", "c.md", "work/b.md", "job/b.md"];

        assert_eq!(
            changes(&before, after, &notes, &["work", "job"], moves.clone()),
            [moves, vec![Change::Folders]].concat()
        );
    }

    /// Facts with one note at `a.md`, whose file holds `text`.
    fn with_hash(text: &str) -> Facts {
        let note = (
            info("a.md", None, &[]),
            Vec::new(),
            STAMP,
            ContentHash::of(text),
            LinkSource::default(),
        );
        Facts::new(vec![note], Vec::new())
    }

    #[test]
    fn a_new_content_hash_is_a_change_with_the_same_info() {
        let before = with_hash("one");

        assert_eq!(
            changes(&before, with_hash("two"), &["a.md"], &[], Vec::new()),
            [Change::Changed(path("a.md"))]
        );
        assert_eq!(changes(&before, with_hash("one"), &["a.md"], &[], Vec::new()), []);
    }

    #[test]
    fn only_a_read_with_a_new_content_hash_marks_an_outside_change() {
        let before = with_hash("one");
        let touched = || Touched {
            notes: vec![path("a.md")],
            read: vec![path("a.md")],
            ..Touched::default()
        };

        let (same, _) = touched().list(Source::Outside, &before, with_hash("one"));
        assert!(!same.changed_after(&path("a.md"), 0));
        let (new, _) = touched().list(Source::Outside, &before, with_hash("two"));
        assert!(new.changed_after(&path("a.md"), 0));
    }

    #[test]
    fn folders_change_only_inside_the_touched_folders() {
        let before = facts(&[], &["a", "b"]);
        let after = facts(&[], &["a", "a/x", "c"]);

        assert_eq!(
            changes(&before, after.clone(), &[], &["a"], Vec::new()),
            [Change::Folders]
        );
        assert_eq!(changes(&before, after, &[], &["d"], Vec::new()), []);
    }
}
