//! The steps of one write. Each step changes the disk, puts what it finds in the next facts,
//! and records what it touched. So the change list of a write misses nothing.

use crate::diff::Touched;
use crate::disk::Disk;
use crate::facts::Facts;
use crate::model::{Change, ChangeList, Note, NoteId, NoteInfo, NoteSet, Order, Source};
use crate::nodes::{Node, Nodes};
use crate::paths::{FolderPath, Name, NotePath};
use crate::reconcile::Settled;
use crate::retag::TagEdit;
use crate::store::{Stamp, Store};
use crate::tag::TagName;
use crate::watch::Batch;
use crate::{Error, Result, scan};

/// The next facts while one write runs, and what the write touched.
pub(crate) struct Write<'a> {
    disk: &'a mut Disk,
    next: Facts,
    touched: Touched,
}

/// Runs `op` on a copy of `facts`, which then replaces `facts`. With a change, the facts get
/// the next revision, and the change list from `source` carries its snapshot.
pub(crate) fn run<T>(
    disk: &mut Disk,
    facts: &mut Facts,
    source: Source,
    op: impl FnOnce(&mut Write<'_>) -> Result<T>,
) -> (Result<T>, Option<ChangeList>) {
    let mut write = Write {
        disk,
        next: facts.clone(),
        touched: Touched::default(),
    };
    let result = op(&mut write);
    let (after, list) = write.touched.list(source, facts, write.next);
    *facts = after;
    (result, list)
}

impl Write<'_> {
    /// Saves `body` as the body of the note at `path`.
    pub(crate) fn save(&mut self, path: &NotePath, body: &str) -> Result<Note> {
        let (note, stamp) = self.disk.store.save(path, body)?;
        self.update(&note, stamp)
    }

    /// Writes `note` again at its path.
    pub(crate) fn restore(&mut self, note: &Note) -> Result<Note> {
        let (note, stamp) = self.disk.store.restore(note)?;
        self.update(&note, stamp)
    }

    /// Creates an empty note at `path`.
    pub(crate) fn create(&mut self, path: &NotePath) -> Result<Note> {
        let (note, stamp) = self.disk.store.create(path)?;
        self.update(&note, stamp)
    }

    /// The id of the note. A note with no id gets one.
    pub(crate) fn link_id(&mut self, path: &NotePath) -> Result<NoteId> {
        let note = self.edit_fields(path, |text| {
            if text.id().is_none() {
                text.set_id(Some(NoteId::generate().as_str()));
            }
        })?;
        note.info.id.ok_or_else(|| self.lost(&note.info.path))
    }

    /// Adds the tag to the note, when it does not have it yet.
    pub(crate) fn add_tag(&mut self, path: &NotePath, tag: &TagName) -> Result<NoteInfo> {
        self.edit_fields(path, |text| {
            let mut tags = TagName::of_note(text);
            if !tags.contains(tag) {
                tags.push(tag.clone());
                text.set_tags(&tags);
            }
        })
        .map(|note| note.info)
    }

    /// Removes the tag from the note, but not its child tags.
    pub(crate) fn remove_tag(&mut self, path: &NotePath, tag: &TagName) -> Result<NoteInfo> {
        self.edit_fields(path, |text| {
            let mut tags = TagName::of_note(text);
            let count = tags.len();
            tags.retain(|other| other != tag);
            if tags.len() != count {
                text.set_tags(&tags);
            }
        })
        .map(|note| note.info)
    }

    /// Pins or unpins the note.
    pub(crate) fn set_pinned(&mut self, path: &NotePath, pinned: bool) -> Result<NoteInfo> {
        self.edit_fields(path, |text| {
            if text.pinned() != pinned {
                text.set_pinned(pinned);
            }
        })
        .map(|note| note.info)
    }

    /// Changes the frontmatter of the note with `edit`, and keeps its modification time.
    fn edit_fields(
        &mut self,
        path: &NotePath,
        edit: impl FnOnce(&mut holm_markdown::Note),
    ) -> Result<Note> {
        let (note, stamp) = self.disk.store.edit_fields(path, edit)?;
        self.update(&note, stamp)
    }

    /// Moves the note from `from` to `to`, and returns it at its new path.
    pub(crate) fn move_note(&mut self, from: &NotePath, to: NotePath) -> Result<NoteInfo> {
        let to = self.disk.store.move_note(from, to)?;
        if *from == to {
            return self.disk.store.read(&to).map(|(note, _)| note.info);
        }
        self.refresh(vec![Node::Note(from.clone()), Node::Note(to.clone())])?;
        let info = self.disk.store.read(&to)?.0.info;
        self.touched.moved(Change::Moved {
            from: from.clone(),
            to,
        });
        Ok(info)
    }

    /// Creates the folder `name` in `parent`, and returns its path.
    pub(crate) fn create_folder(&mut self, parent: &FolderPath, name: &Name) -> Result<FolderPath> {
        let new = self.disk.store.create_folder(parent, name)?;
        self.folders(&new)?;
        Ok(new)
    }

    /// Renames the folder to `name` in its parent, and returns its new path.
    pub(crate) fn rename_folder(&mut self, folder: &FolderPath, name: &Name) -> Result<FolderPath> {
        self.move_folder_to(folder, &folder.folder().unwrap_or_default(), name)
    }

    /// Moves the folder into `parent` with its name, and returns its new path.
    pub(crate) fn move_folder(
        &mut self,
        folder: &FolderPath,
        parent: &FolderPath,
    ) -> Result<FolderPath> {
        let Some((_, name)) = folder.split() else {
            return Err(Error::BadPath {
                path: folder.as_ref().to_owned(),
            });
        };
        self.move_folder_to(folder, parent, &name)
    }

    /// Moves the folder into `parent` as `name`, and returns its new path.
    fn move_folder_to(
        &mut self,
        folder: &FolderPath,
        parent: &FolderPath,
        name: &Name,
    ) -> Result<FolderPath> {
        let new = self.disk.store.move_folder(folder, parent, name)?;
        if new != *folder {
            self.touched.moved(Change::FolderMoved {
                from: folder.clone(),
                to: new.clone(),
            });
            let nodes = vec![Node::Folder(folder.clone()), Node::Folder(new.clone())];
            self.refresh(nodes)?;
            self.folders(folder)?;
            self.folders(&new)?;
        }
        Ok(new)
    }

    /// Deletes the note and its attachment files.
    pub(crate) fn delete_note(&mut self, path: &NotePath) -> Result<()> {
        self.delete(&Node::Note(path.clone()), |store| store.delete_note(path))
    }

    /// Deletes the folder, with every note and file in it, and their attachment files.
    pub(crate) fn delete_folder(&mut self, folder: &FolderPath) -> Result<()> {
        self.delete(&Node::Folder(folder.clone()), |store| {
            store.delete_folder(folder)
        })
    }

    /// Runs `edit` on the tags of each note with its tag, and returns how many notes changed.
    pub(crate) fn edit_tag(&mut self, edit: &TagEdit<'_>) -> Result<usize> {
        let paths = self
            .next
            .snapshot()
            .paths(NoteSet::WithTag(edit.tag()), Order::Title);
        let store = &self.disk.store;
        let rewritten = store.rewrite(&paths, |_, old| {
            let mut text = holm_markdown::Note::parse(old);
            let tags = TagName::of_note(&text);
            let new = edit.apply(&tags);
            (new != tags).then(|| {
                text.set_tags(&new);
                text.to_string()
            })
        });
        let mut written = Vec::with_capacity(rewritten.len());
        let mut first_error = None;
        for (path, result) in rewritten {
            match result.and_then(|()| store.read(&path)) {
                Ok(note) => written.push(note),
                Err(error) => {
                    tracing::warn!(%error, "kept the tags of a note");
                    first_error.get_or_insert(error);
                }
            }
        }
        let changed = written.len();
        if changed > 0 {
            self.update_all(written);
        }
        if changed > 0
            && let TagEdit::Rename { from, to } = edit
            && from != to
        {
            self.touched.moved(Change::TagMoved {
                from: (*from).clone(),
                to: (*to).clone(),
            });
        }
        first_error.map_or(Ok(changed), Err)
    }

    /// Checks the disk at each path of `batch` after a change from another tool.
    pub(crate) fn outside(&mut self, batch: Batch) -> Result<()> {
        let nodes = Nodes::outside(&self.disk.store, self.next.snapshot(), batch);
        if nodes.is_empty() {
            return Ok(());
        }
        let settled = scan::files(&self.disk.store, &self.next, &nodes)?;
        self.take(settled);
        for node in &nodes {
            if let Node::Folder(folder) = node {
                self.folders(folder)?;
            }
        }
        Ok(())
    }

    /// Reads the folders at `folder` or below it from the disk again.
    fn folders(&mut self, folder: &FolderPath) -> Result<()> {
        let found = self.disk.store.folders(folder)?;
        self.next.set_folders(folder, found);
        self.touched.folder(folder.clone());
        Ok(())
    }

    /// Puts a note that Holm just wrote into the index and the next facts, and returns it.
    fn update(&mut self, note: &Note, stamp: Stamp) -> Result<Note> {
        let notes = self.update_all(vec![(note.clone(), stamp)]);
        notes
            .into_iter()
            .next()
            .ok_or_else(|| self.lost(&note.info.path))
    }

    /// The error when the note that Holm just wrote at `path` does not read back as written.
    fn lost(&self, path: &NotePath) -> Error {
        Error::Write {
            path: self.disk.store.root().join(path),
            source: std::io::ErrorKind::InvalidData.into(),
        }
    }

    /// Puts notes that Holm just wrote into the index and the next facts, and returns them.
    fn update_all(&mut self, notes: Vec<(Note, Stamp)>) -> Vec<Note> {
        let settled = scan::wrote(&self.disk.store, &self.next, notes);
        self.take(settled)
    }

    /// Runs `delete` on `node`, then deletes its attachment files.
    /// It updates the index and the next facts also when `delete` fails, since a part may be gone.
    fn delete(&mut self, node: &Node, delete: impl FnOnce(&Store) -> Result<()>) -> Result<()> {
        let (_, files) = self.disk.removal(self.next.snapshot(), node);
        let deleted = delete(&self.disk.store);
        if deleted.is_ok() {
            for file in files {
                if let Err(error) = self.disk.store.delete_attachment(&file) {
                    tracing::warn!(%error, "kept an attachment");
                }
            }
        }
        self.refresh(vec![node.clone()])?;
        if let Node::Folder(folder) = node {
            self.folders(folder)?;
        }
        deleted
    }

    /// Updates the index and the next facts at each path after Holm's own change.
    fn refresh(&mut self, nodes: Vec<Node>) -> Result<()> {
        let settled = scan::files(&self.disk.store, &self.next, &Nodes::new(nodes))?;
        self.take(settled);
        Ok(())
    }

    /// Writes the rows of `settled` into the index and puts it in the next facts. It returns the
    /// notes that were read.
    fn take(&mut self, settled: Settled) -> Vec<Note> {
        self.disk.write_rows(&settled);
        self.touched.settled(&settled);
        self.next.apply(settled)
    }
}
