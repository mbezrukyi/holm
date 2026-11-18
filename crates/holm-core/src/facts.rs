//! The facts of the notes folder in memory: the snapshot, and the stamp of each note file.

use std::collections::HashMap;

use crate::model::{Link, Note, NoteInfo};
use crate::nodes::{Node, Nodes};
use crate::paths::{FolderPath, NotePath};
use crate::reconcile::Settled;
use crate::relink::LinkSource;
use crate::snapshot::Snapshot;
use crate::store::Stamp;

/// The snapshot, and the stamp and link source of each of its notes. Each note of the snapshot
/// has both.
///
/// It is cheap to clone.
#[derive(Clone, Debug, Default)]
pub(crate) struct Facts {
    snapshot: Snapshot,
    files: imbl::HashMap<NotePath, (Stamp, LinkSource)>,
}

/// The facts of one note: what the snapshot shows, its links, its stamp and its link source.
pub(crate) type NoteFacts = (NoteInfo, Vec<Link>, Stamp, LinkSource);

impl Facts {
    /// The facts of `notes` and `folders`, with revision 0.
    pub(crate) fn new(notes: Vec<NoteFacts>, folders: Vec<FolderPath>) -> Self {
        let mut facts = Self {
            snapshot: Snapshot::new(folders),
            files: imbl::HashMap::new(),
        };
        facts.remove_and_put(Vec::new(), notes);
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
                Node::Note(path) => stamps.extend(self.files.get_key_value(path).map(copied)),
                Node::Folder(folder) if folder.is_root() => {
                    stamps.extend(self.files.iter().map(copied));
                }
                Node::Folder(folder) => stamps.extend(
                    self.files
                        .iter()
                        .filter(|(path, _)| path.is_below(folder))
                        .map(copied),
                ),
            }
        }
        stamps
    }

    /// The link source of the note at `path`.
    pub(crate) fn link_source(&self, path: &NotePath) -> Option<LinkSource> {
        self.files.get(path).map(|(_, source)| *source)
    }

    /// Puts in what `settled` changes, and returns the notes that were read.
    pub(crate) fn apply(&mut self, settled: Settled) -> Vec<Note> {
        let mut removed = settled.removed;
        let mut put = Vec::with_capacity(settled.put.len() + settled.moved.len());
        let mut notes = Vec::with_capacity(settled.put.len());
        for read in settled.put {
            put.push((read.note.info.clone(), read.links, read.stamp, read.source));
            notes.push(read.note);
        }
        for moved in settled.moved {
            removed.push(moved.from);
            put.push((moved.info, moved.links, moved.stamp, moved.source));
        }
        self.remove_and_put(removed, put);
        notes
    }

    /// Takes out the notes at `removed` first, so a note can move to a path and keep its id.
    /// Then it puts in `put`.
    fn remove_and_put(&mut self, removed: Vec<NotePath>, put: Vec<NoteFacts>) {
        for path in &removed {
            self.files.remove(path);
        }
        for (info, _, stamp, source) in &put {
            self.files.insert(info.path.clone(), (*stamp, *source));
        }
        self.snapshot.apply(
            removed,
            put.into_iter().map(|(info, links, _, _)| (info, links)),
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

fn copied((path, (stamp, _)): (&NotePath, &(Stamp, LinkSource))) -> (NotePath, Stamp) {
    (path.clone(), *stamp)
}
