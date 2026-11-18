//! The facts of the notes folder in memory: the snapshot, and the stamp of each note file.

use std::collections::HashMap;

use crate::model::{Link, Note, NoteInfo};
use crate::nodes::{Node, Nodes};
use crate::paths::{FolderPath, NotePath};
use crate::reconcile::Settled;
use crate::snapshot::Snapshot;
use crate::store::Stamp;

/// The snapshot and the stamp of each of its notes. Each note of the snapshot has a stamp.
///
/// It is cheap to clone.
#[derive(Clone, Debug, Default)]
pub(crate) struct Facts {
    snapshot: Snapshot,
    stamps: imbl::HashMap<NotePath, Stamp>,
}

impl Facts {
    /// The facts of `notes` and `folders`, with revision 0.
    pub(crate) fn new(notes: Vec<(NoteInfo, Vec<Link>, Stamp)>, folders: Vec<FolderPath>) -> Self {
        let mut facts = Self {
            snapshot: Snapshot::new(folders),
            stamps: imbl::HashMap::new(),
        };
        facts.apply(Vec::new(), notes);
        facts
    }

    pub(crate) fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// The stamps of the notes at or below each of `nodes`.
    pub(crate) fn stamps_in(&self, nodes: &Nodes) -> HashMap<NotePath, Stamp> {
        let mut stamps = HashMap::new();
        for node in nodes {
            match node {
                Node::Note(path) => stamps.extend(self.stamps.get_key_value(path).map(copied)),
                Node::Folder(folder) if folder.is_root() => {
                    stamps.extend(self.stamps.iter().map(copied));
                }
                Node::Folder(folder) => stamps.extend(
                    self.stamps
                        .iter()
                        .filter(|(path, _)| path.is_below(folder))
                        .map(copied),
                ),
            }
        }
        stamps
    }

    /// Puts in what `settled` changes, and returns the notes that were read. `links` holds
    /// the links of each put note, in order.
    pub(crate) fn settle(&mut self, settled: Settled, links: Vec<Vec<Link>>) -> Vec<Note> {
        let mut removed = settled.removed;
        let mut put = Vec::with_capacity(settled.put.len() + settled.moved.len());
        let mut notes = Vec::with_capacity(settled.put.len());
        for (read, links) in settled.put.into_iter().zip(links) {
            put.push((read.note.info.clone(), links, read.stamp));
            notes.push(read.note);
        }
        for moved in settled.moved {
            removed.push(moved.from);
            put.push((moved.info, moved.links, moved.stamp));
        }
        self.apply(removed, put);
        notes
    }

    /// Takes out the notes at `removed` first, so a note can move to a path and keep its id.
    /// Then it puts in `put`, each note with its stamp.
    fn apply(&mut self, removed: Vec<NotePath>, put: Vec<(NoteInfo, Vec<Link>, Stamp)>) {
        for path in &removed {
            self.stamps.remove(path);
        }
        for (info, _, stamp) in &put {
            self.stamps.insert(info.path.clone(), *stamp);
        }
        self.snapshot.apply(
            removed,
            put.into_iter().map(|(info, links, _)| (info, links)),
        );
    }

    /// Puts `found` in place of the folders at `folder` or below it. `found` must be sorted.
    pub(crate) fn set_folders(&mut self, folder: &FolderPath, found: Vec<FolderPath>) {
        self.snapshot.set_folders(folder, found);
    }

    /// The facts after these ones, with the next revision.
    pub(crate) fn next_revision(mut self) -> Self {
        self.snapshot = self.snapshot.next_revision();
        self
    }
}

fn copied((path, stamp): (&NotePath, &Stamp)) -> (NotePath, Stamp) {
    (path.clone(), *stamp)
}
