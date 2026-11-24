use std::collections::HashMap;

use crate::facts::Facts;
use crate::model::{Change, ChangeList, NoteId, NoteInfo, Source};
use crate::paths::{FolderPath, NotePath};
use crate::reconcile::Settled;
use crate::snapshot::Snapshot;

/// What one write touched: the note paths, the folders and the moves it made. The diff looks only there, so its cost
/// grows with the write, not with the notes folder.
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

    /// Adds a move that the write made: [`Change::Moved`], [`Change::FolderMoved`] or [`Change::TagMoved`].
    pub(crate) fn moved(&mut self, change: Change) {
        self.moves.push(change);
    }

    /// The facts after the write, and its change list from `source`. With a change, the facts get the next revision,
    /// and the list carries its snapshot. With no change, there is no list, and the facts keep their revision. From
    /// outside, each note that was read with a new content hash gets the revision of the facts after the write as its
    /// outside revision.
    ///
    /// The list holds the moves of the write first, then the note changes in path order, then [`Change::Folders`] and
    /// [`Change::Tags`]. A note that a move of the write explains gives no other change. A gone note and a new note
    /// with the same id are one [`Change::Moved`]. A note with a new content hash is [`Change::Changed`], also when
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
            after.mark_outside(&read, Some(revision));
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

/// In any order. A note has few tags.
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
mod tests;
