//! The facts of the notes folder in memory: the snapshot, and the stamp of each note file.

use std::collections::HashMap;

use crate::model::{Link, NoteInfo};
use crate::nodes::{Node, Nodes};
use crate::paths::{FolderPath, NotePath};
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

/// The notes that a change of the index took out and put in, each put note with its stamp.
///
/// [`Facts::apply`] takes them out first, so a note can move to a path and keep its id.
#[derive(Debug, Default)]
pub(crate) struct Delta {
    pub(crate) removed: Vec<NotePath>,
    pub(crate) put: Vec<(NoteInfo, Vec<Link>, Stamp)>,
}

impl Facts {
    /// The facts of `notes` and `folders`, with revision 0.
    pub(crate) fn new(notes: Vec<(NoteInfo, Vec<Link>, Stamp)>, folders: Vec<FolderPath>) -> Self {
        let mut facts = Self {
            snapshot: Snapshot::new(folders),
            stamps: imbl::HashMap::new(),
        };
        facts.apply(Delta {
            removed: Vec::new(),
            put: notes,
        });
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

    /// Takes out and puts in the notes of `delta`. It returns whether the snapshot changed.
    /// A new stamp alone changes no fact.
    pub(crate) fn apply(&mut self, delta: Delta) -> bool {
        for path in &delta.removed {
            self.stamps.remove(path);
        }
        for (info, _, stamp) in &delta.put {
            self.stamps.insert(info.path.clone(), *stamp);
        }
        self.snapshot.apply(
            delta.removed,
            delta.put.into_iter().map(|(info, links, _)| (info, links)),
        )
    }

    /// Puts `found` in place of the folders at `folder` or below it. It returns whether they
    /// changed. `found` must be sorted.
    pub(crate) fn set_folders(&mut self, folder: &FolderPath, found: Vec<FolderPath>) -> bool {
        self.snapshot.set_folders(folder, found)
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
