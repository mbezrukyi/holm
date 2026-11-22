use std::collections::HashMap;

use crate::links::LinkSource;
use crate::model::{Link, Note, NoteInfo};
use crate::nodes::{Node, Nodes};
use crate::paths::{FolderPath, NotePath};
use crate::reconcile::Settled;
use crate::snapshot::Snapshot;
use crate::store::{ContentHash, Stamp};

/// Each note of the snapshot has file facts.
///
/// It is cheap to clone.
#[derive(Clone, Debug, Default)]
pub(crate) struct Facts {
    snapshot: Snapshot,
    files: imbl::HashMap<NotePath, File>,
}

#[derive(Clone, Copy, Debug)]
struct File {
    stamp: Stamp,
    hash: ContentHash,
    source: LinkSource,
    /// The revision of the last outside change that gave the file new content, or 0. A move
    /// keeps it, and so does a write of Holm.
    outside: u64,
}

/// The facts of one note: what the snapshot shows, its links, its stamp, its content hash and
/// its link source.
pub(crate) type NoteFacts = (NoteInfo, Vec<Link>, Stamp, ContentHash, LinkSource);

impl Facts {
    /// With revision 0.
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
                Node::Note(path) => stamps.extend(self.files.get_key_value(path).map(stamp)),
                Node::Folder(folder) if folder.is_root() => {
                    stamps.extend(self.files.iter().map(stamp));
                }
                Node::Folder(folder) => {
                    stamps.extend(self.files.iter().filter(|(path, _)| path.is_below(folder)).map(stamp));
                }
            }
        }
        stamps
    }

    pub(crate) fn link_source(&self, path: &NotePath) -> Option<LinkSource> {
        self.files.get(path).map(|file| file.source)
    }

    pub(crate) fn hash(&self, path: &NotePath) -> Option<ContentHash> {
        self.files.get(path).map(|file| file.hash)
    }

    /// Whether an outside change after revision `seen` gave the file of the note at `path` new
    /// content.
    pub(crate) fn changed_after(&self, path: &NotePath, seen: u64) -> bool {
        self.files.get(path).is_some_and(|file| file.outside > seen)
    }

    /// The revision of the last outside change that gave the file of the note at `path` new
    /// content, or 0.
    pub(crate) fn outside(&self, path: &NotePath) -> u64 {
        self.files.get(path).map_or(0, |file| file.outside)
    }

    /// Records that an outside change of `revision` gave each file at `paths` new content. A
    /// newer revision that a file has stays.
    pub(crate) fn mark_outside<'p>(&mut self, paths: impl IntoIterator<Item = &'p NotePath>, revision: u64) {
        for path in paths {
            if let Some(file) = self.files.get_mut(path) {
                file.outside = file.outside.max(revision);
            }
        }
    }

    /// Puts in what `settled` changes, and returns the notes that were read. A moved note keeps
    /// the outside revision of its old path.
    pub(crate) fn apply(&mut self, settled: Settled) -> Vec<Note> {
        let mut removed = settled.removed;
        let mut put = Vec::with_capacity(settled.put.len() + settled.moved.len());
        let mut notes = Vec::with_capacity(settled.put.len());
        let mut carried = Vec::with_capacity(settled.moved.len());
        for read in settled.put {
            put.push((read.note.info.clone(), read.links, read.stamp, read.hash, read.source));
            notes.push(read.note);
        }
        for moved in settled.moved {
            carried.push((moved.info.path.clone(), self.outside(&moved.from)));
            removed.push(moved.from);
            put.push((moved.info, moved.links, moved.stamp, moved.hash, moved.source));
        }
        self.remove_and_put(removed, put);
        for (path, outside) in carried {
            self.mark_outside([&path], outside);
        }
        notes
    }

    /// Takes out the notes at `removed` first, so a note can move to a path and keep its id.
    /// Then it puts in `put`. A note keeps the outside revision of its path.
    fn remove_and_put(&mut self, removed: Vec<NotePath>, put: Vec<NoteFacts>) {
        for path in &removed {
            self.files.remove(path);
        }
        for (info, _, stamp, hash, source) in &put {
            let file = File {
                stamp: *stamp,
                hash: *hash,
                source: *source,
                outside: self.outside(&info.path),
            };
            self.files.insert(info.path.clone(), file);
        }
        self.snapshot
            .swap(removed, put.into_iter().map(|(info, links, _, _, _)| (info, links)));
    }

    /// Puts `found` in place of the folders at `folder` or below it. `found` must be sorted.
    pub(crate) fn set_folders(&mut self, folder: &FolderPath, found: Vec<FolderPath>) {
        self.snapshot.set_folders(folder, found);
    }

    pub(crate) fn next_revision(mut self) -> Self {
        self.snapshot = self.snapshot.next_revision();
        self
    }
}

fn stamp((path, file): (&NotePath, &File)) -> (NotePath, Stamp) {
    (path.clone(), file.stamp)
}
