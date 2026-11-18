//! The core of Holm: notes, files, links, the index and file watching.
//!
//! It has no UI. The app and the later manager use it through one [`Core`] handle.

mod error;
mod facts;
mod index;
pub mod model;
mod nodes;
mod paths;
mod reconcile;
mod relink;
mod retag;
#[cfg(feature = "test-support")]
pub mod sample;
mod scan;
mod snapshot;
mod store;
mod watch;

use std::collections::{HashMap, hash_map};
use std::ffi::OsStr;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};

use futures_channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use futures_core::Stream;

pub use error::{Error, NameProblem, Result};
pub use paths::{FolderPath, NotePath};
pub use snapshot::Snapshot;
pub use watch::Feed;

use facts::{Delta, Facts};
use index::Index;
use model::{
    Change, ChangeList, DeleteCount, LinkTarget, Note, NoteId, NoteInfo, NoteSet, Order,
    SearchResults,
};
use nodes::{Node, Nodes};
use paths::Name;
use reconcile::Settled;
use retag::TagEdit;
use store::Store;
use watch::{Apply, Batch};

/// The stem of a pasted image before its hash.
const PASTED_IMAGE: &str = "Pasted image";

/// The handle to one notes folder.
///
/// It is cheap to clone and safe to share across threads. Its methods block,
/// so the app calls them away from the UI thread.
#[derive(Clone, Debug)]
pub struct Core {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    store: Store,
    index: Index,
    /// Held while a write, its index update and its snapshot update run,
    /// so a watch batch never sees half of it.
    writing: Mutex<()>,
    /// Changes only under `writing`.
    facts: Mutex<Facts>,
    listeners: Mutex<Vec<UnboundedSender<ChangeList>>>,
    _watcher: Option<watch::Watcher>,
}

impl Core {
    /// Opens the notes folder and brings the index in `index_dir` up to date with the files.
    ///
    /// Then it watches the notes folder. Changes from other tools update the index
    /// and the snapshot, and go to each [`Changes`]. When the watch can't start, the core works without it,
    /// with a log line.
    ///
    /// Then it keeps the facts of the notes folder in memory, as a [`Snapshot`].
    ///
    /// Holm can delete the index and rebuild it from the files at any time.
    /// It does so when SQLite fails on it, or when it comes from a Holm with another index shape.
    /// When the file can't be rebuilt, for example on a full disk, the index lives in memory,
    /// with a log line. So a write fails only when its change did not reach the disk.
    ///
    /// When two note files have the same id, the file that Holm already knows keeps it.
    /// The other file gets a new id, written into the file.
    ///
    /// # Errors
    ///
    /// - [`Error::NotesFolder`] when `notes_folder` is missing or is not a folder.
    /// - [`Error::Read`] when the notes folder can't be read.
    /// - [`Error::Index`] when the index can't be used, even in memory. Only a bug does this.
    pub fn open(notes_folder: impl AsRef<Path>, index_dir: impl AsRef<Path>) -> Result<Self> {
        let store = Store::open(notes_folder.as_ref())?;
        // The watch starts before the first index sync, so no change falls between the two.
        let (watcher, events) = match watch::start(store.root()) {
            Ok((watcher, events)) => (Some(watcher), Some(events)),
            Err(error) => {
                tracing::warn!(%error, path = %store.root().display(), "cannot watch the notes folder");
                (None, None)
            }
        };
        let core = Self::build(store, index_dir.as_ref(), watcher)?;
        if let Some(events) = events {
            events.run(core.apply());
        }
        Ok(core)
    }

    /// Opens the notes folder like [`Core::open`], but with no file watching.
    ///
    /// Changes on disk reach the core only through the [`Feed`]. Tests use it.
    ///
    /// # Errors
    ///
    /// As in [`Core::open`].
    pub fn open_with_feed(
        notes_folder: impl AsRef<Path>,
        index_dir: impl AsRef<Path>,
    ) -> Result<(Self, Feed)> {
        let store = Store::open(notes_folder.as_ref())?;
        let core = Self::build(store, index_dir.as_ref(), None)?;
        let feed = Feed {
            apply: core.apply(),
        };
        Ok((core, feed))
    }

    /// The facts of the notes folder now.
    ///
    /// It is never older than a write or a batch of outside changes that the core finished.
    pub fn snapshot(&self) -> Snapshot {
        self.inner.snapshot()
    }

    /// A new stream of the change lists, for writes of Holm and for changes from other tools.
    ///
    /// Each stream gets every list from the time it was made. A list comes for each new
    /// revision, with the snapshot of that revision. It can come before [`Core::snapshot`]
    /// shows that revision.
    /// A batch of outside changes gives the note changes in path order, then
    /// [`Change::Folders`] and [`Change::Tags`]. A write gives the moves it made, and
    /// [`Change::Notes`] and [`Change::Folders`] for the other facts it changed.
    pub fn changes(&self) -> Changes {
        let (send, receive) = mpsc::unbounded();
        lock(&self.inner.listeners).push(send);
        Changes { receive }
    }

    /// The notes whose title, text or tags hold every word of `text`, in `order`, with the
    /// snapshot they come from. A write that runs ends first, so the snapshot has each note.
    ///
    /// A word also matches longer words that start with it, so `pla` finds "plans".
    /// Capital letters and accents make no difference. With no words, the list is empty.
    ///
    /// # Errors
    ///
    /// [`Error::Index`] as in [`Core::open`].
    pub fn search(&self, text: &str, order: Order) -> Result<SearchResults> {
        let _writing = lock(&self.inner.writing);
        let paths = self.inner.index.search(text)?;
        Ok(SearchResults::new(self.inner.snapshot(), paths, order))
    }

    /// The text of each heading in the note at `path`, in order.
    ///
    /// # Errors
    ///
    /// As in [`Core::read`].
    pub fn headings(&self, path: &NotePath) -> Result<Vec<String>> {
        let body = self.read(path)?.body;
        let parsed = holm_markdown::Body::parse(&body);
        Ok(parsed
            .headings()
            .iter()
            .map(|heading| heading.text.clone())
            .collect())
    }

    /// The note links in the note at `path` that point to a missing note or heading,
    /// each once. A heading link needs a heading with the same text, also in capital letters.
    ///
    /// # Errors
    ///
    /// As in [`Core::read`], when a linked note can't be read.
    pub fn broken_links(&self, path: &NotePath) -> Result<Vec<LinkTarget>> {
        let snapshot = self.snapshot();
        let mut known: HashMap<NoteId, Option<Vec<String>>> = HashMap::new();
        let mut broken = Vec::new();
        for link in snapshot.links(path) {
            let LinkTarget::Note { id, heading } = &link.target else {
                continue;
            };
            let headings = match known.entry(id.clone()) {
                hash_map::Entry::Occupied(entry) => entry.into_mut(),
                hash_map::Entry::Vacant(entry) => entry.insert(self.all_headings(&snapshot, id)?),
            };
            let found = match (headings, heading) {
                (None, _) => false,
                (Some(_), None) => true,
                (Some(headings), Some(heading)) => headings.contains(heading),
            };
            if !found && !broken.contains(&link.target) {
                broken.push(link.target.clone());
            }
        }
        Ok(broken)
    }

    /// Every heading of the note with `id`, or `None` with no such note.
    fn all_headings(&self, snapshot: &Snapshot, id: &NoteId) -> Result<Option<Vec<String>>> {
        match snapshot.note_with_id(id.as_str()) {
            Some(info) => self.headings(&info.path).map(Some),
            None => Ok(None),
        }
    }

    /// The path for a link from the note at `note` to `file`, with no copy. A file inside the
    /// notes folder gets a path from the folder of the note. A file outside gets its absolute
    /// path.
    ///
    /// # Errors
    ///
    /// [`Error::BadPath`] when `file` is not absolute.
    pub fn file_link(&self, note: &NotePath, file: impl AsRef<Path>) -> Result<PathBuf> {
        let file = file.as_ref();
        if !file.is_absolute() {
            return Err(Error::BadPath {
                path: file.to_owned(),
            });
        }
        let file = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_owned());
        Ok(relink::link(self.inner.store.root(), note, &file))
    }

    /// Reads one note.
    ///
    /// # Errors
    ///
    /// [`Error::Read`] or [`Error::NotUtf8`] when the file can't be read as text.
    pub fn read(&self, path: &NotePath) -> Result<Note> {
        self.inner.store.read(path).map(|(note, _)| note)
    }

    /// Saves `body` as the body of the note at `path`, and returns the note as it is now on disk.
    ///
    /// The frontmatter comes from the file, so a change from another tool stays.
    /// Every frontmatter field that Holm does not know stays as it is.
    ///
    /// - A note with no `id:` gets one.
    /// - A body with no final line break gets one.
    /// - When the body stays the same, the file keeps its old modification time.
    /// - With no change at all, Holm does not write the file.
    ///
    /// The write goes to a temp file first, which then replaces the note in one step.
    ///
    /// # Errors
    ///
    /// - [`Error::Read`] or [`Error::NotUtf8`] when the file on disk can't be read.
    /// - [`Error::Write`] when the new file can't be written. The old file then stays.
    pub fn save(&self, path: &NotePath, body: &str) -> Result<Note> {
        self.inner.write(|draft| {
            let (note, stamp) = self.inner.store.save(path, body)?;
            self.inner.update(draft, &note, stamp)
        })
    }

    /// Writes `note` again at its path after another tool deleted its file, and returns it.
    /// The note keeps its id, so links to it still work. It also keeps its tags and its pin.
    /// When a file is at the path again, it saves the body over it like [`Core::save`].
    ///
    /// # Errors
    ///
    /// [`Error::Write`] when the file can't be written, for example when the folder is missing.
    pub fn restore(&self, note: &Note) -> Result<Note> {
        self.inner.write(|draft| {
            let (note, stamp) = self.inner.store.restore(note)?;
            self.inner.update(draft, &note, stamp)
        })
    }

    /// Creates an empty note with a new id in `folder`, and returns it.
    ///
    /// The file is `<title>.md`, with no spaces around `title`. Holm never adds a number.
    ///
    /// # Errors
    ///
    /// - [`Error::BadName`] when `title` is empty, holds `/`, starts with `.`, or is too long
    ///   with `.md`.
    /// - [`Error::NameTaken`] when something in `folder` has the file name, also with other
    ///   capital letters.
    /// - [`Error::Write`] when the file can't be written, for example when the folder is missing.
    pub fn create(&self, folder: &FolderPath, title: &str) -> Result<Note> {
        self.inner.write(|draft| {
            let (note, stamp) = self
                .inner
                .store
                .create(&folder.note(&Name::title(title)?))?;
            self.inner.update(draft, &note, stamp)
        })
    }

    /// Gives the note the title `title`: its file becomes `<title>.md` in the same folder.
    /// It returns the note at its new path.
    ///
    /// Titles work as in [`Core::create`]. A change of only the capital letters works.
    /// The note keeps its text and its modification time.
    /// No other note changes, because note links use the id.
    ///
    /// # Errors
    ///
    /// - [`Error::BadName`] and [`Error::NameTaken`] as in [`Core::create`]. Nothing moves.
    /// - [`Error::Read`] or [`Error::NotUtf8`] when the file can't be read as text.
    /// - [`Error::Write`] when the file can't be moved. The note then stays where it was.
    pub fn rename_note(&self, path: &NotePath, title: &str) -> Result<NoteInfo> {
        self.inner.write(|draft| {
            let new = self
                .inner
                .store
                .move_note(path, path.folder().note(&Name::title(title)?))?;
            self.inner.settle(draft, path, &new)
        })
    }

    /// Moves the note to `folder` with its file name, and returns it at its new path.
    ///
    /// Relative file links in the note change, so they point to the same files.
    /// No other note changes, because note links use the id.
    ///
    /// # Errors
    ///
    /// - [`Error::NameTaken`] when something in `folder` has the file name, also with other
    ///   capital letters. Nothing moves.
    /// - Else as in [`Core::rename_note`].
    pub fn move_note(&self, path: &NotePath, folder: &FolderPath) -> Result<NoteInfo> {
        self.inner.write(|draft| {
            let new = self.inner.store.move_note(path, path.in_folder(folder))?;
            self.inner.settle(draft, path, &new)
        })
    }

    /// Creates the folder `name` in `parent`, and returns its path.
    ///
    /// Holm trims `name`, and never adds a number.
    ///
    /// # Errors
    ///
    /// - [`Error::BadName`] when `name` is empty, holds `/`, starts with `.`, is too long,
    ///   or is `attachments` at the root.
    /// - [`Error::NameTaken`] when something in `parent` has the name, also with other
    ///   capital letters.
    /// - [`Error::Write`] when the folder can't be made, for example when `parent` is missing.
    /// - [`Error::Read`] when the new folder can't be read.
    pub fn create_folder(&self, parent: &FolderPath, name: &str) -> Result<FolderPath> {
        self.inner.write(|draft| {
            let new = self
                .inner
                .store
                .create_folder(parent, &Name::typed(name)?)?;
            draft.wrote_folders(&new)?;
            Ok(new)
        })
    }

    /// Renames the folder to `name`, and returns its new path.
    ///
    /// Names work as in [`Core::create_folder`]. A change of only the capital letters works.
    /// Relative file links in its notes change, so they point to the same files.
    /// A note whose links can't change keeps them, with a log line.
    /// Each note changes in one step, but the whole job does not. After a crash, the folder has
    /// moved, but some notes can still have their old links.
    ///
    /// # Errors
    ///
    /// - [`Error::BadName`] and [`Error::NameTaken`] as in [`Core::create_folder`].
    ///   Nothing moves.
    /// - [`Error::BadPath`] when `folder` is the root or `attachments/`.
    /// - [`Error::Read`] when `folder` is missing or is not a folder.
    /// - [`Error::Write`] when the folder can't be moved. It then stays where it was.
    pub fn rename_folder(&self, folder: &FolderPath, name: &str) -> Result<FolderPath> {
        let name = Name::typed(name)?;
        self.move_folder_to(folder, &folder.folder().unwrap_or_default(), &name)
    }

    /// Moves the folder into `parent` with its name, and returns its new path.
    ///
    /// It works as [`Core::rename_folder`], also after a crash.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] also when `parent` is `folder` or inside it.
    /// - Else as in [`Core::rename_folder`].
    pub fn move_folder(&self, folder: &FolderPath, parent: &FolderPath) -> Result<FolderPath> {
        match folder.split() {
            Some((_, name)) => self.move_folder_to(folder, parent, &name),
            None => self.inner.write(|_| {
                Err(Error::BadPath {
                    path: folder.as_ref().to_owned(),
                })
            }),
        }
    }

    /// What [`Core::delete_note`] removes: the note and its attachment files.
    ///
    /// # Errors
    ///
    /// [`Error::Index`] as in [`Core::search`].
    pub fn note_delete_count(&self, path: &NotePath) -> Result<DeleteCount> {
        self.inner
            .removal(&Node::Note(path.clone()))
            .map(|(count, _)| count)
    }

    /// What [`Core::delete_folder`] removes: the notes in the folder and below it,
    /// and their attachment files.
    ///
    /// # Errors
    ///
    /// [`Error::Index`] as in [`Core::search`].
    pub fn folder_delete_count(&self, folder: &FolderPath) -> Result<DeleteCount> {
        self.inner
            .removal(&Node::Folder(folder.clone()))
            .map(|(count, _)| count)
    }

    /// Deletes the note forever. Holm has no trash.
    ///
    /// It also deletes each file in `attachments/` that the note links and no other note links.
    /// It never deletes a linked file outside `attachments/`.
    /// A note that is already gone is no error.
    ///
    /// # Errors
    ///
    /// [`Error::Write`] when the note can't be deleted.
    /// An attachment that can't be deleted stays, with a log line.
    pub fn delete_note(&self, path: &NotePath) -> Result<()> {
        self.inner.write(|draft| {
            self.inner
                .delete(draft, &Node::Note(path.clone()), |store| {
                    store.delete_note(path)
                })
        })
    }

    /// Deletes the folder forever, with every note and file in it.
    ///
    /// Attachments go as in [`Core::delete_note`]. A folder that is already gone is no error.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] when `folder` is the root or `attachments/`.
    /// - [`Error::Read`] when `folder` is not a folder.
    /// - [`Error::Write`] when the folder can't be deleted. Some files in it may be gone.
    pub fn delete_folder(&self, folder: &FolderPath) -> Result<()> {
        self.inner.write(|draft| {
            self.inner
                .delete(draft, &Node::Folder(folder.clone()), |store| {
                    store.delete_folder(folder)
                })
        })
    }

    /// The id of the note, for a link to it.
    ///
    /// A note with no id gets one. Only the `id:` line is added, and the file keeps
    /// its modification time.
    ///
    /// # Errors
    ///
    /// - [`Error::Read`] or [`Error::NotUtf8`] when the file can't be read as text.
    /// - [`Error::Write`] when the id can't be written.
    pub fn link_id(&self, path: &NotePath) -> Result<NoteId> {
        self.inner.write(|draft| {
            let note = self.inner.edit_fields(draft, path, |text| {
                if text.id().is_none() {
                    text.set_id(Some(NoteId::generate().as_str()));
                }
            })?;
            note.info.id.ok_or_else(|| Error::Write {
                path: self.inner.store.root().join(&note.info.path),
                source: std::io::ErrorKind::InvalidData.into(),
            })
        })
    }

    /// Adds the tag to the note and returns it. Its body, other tags and modification time stay.
    ///
    /// The tag is cleaned like a typed tag. A note that has it already does not change.
    ///
    /// # Errors
    ///
    /// - [`Error::BadTag`] when nothing is left of `tag`.
    /// - [`Error::Read`] or [`Error::NotUtf8`] when the file can't be read as text.
    /// - [`Error::Write`] when the file can't be written. The old file then stays.
    pub fn add_tag(&self, path: &NotePath, tag: &str) -> Result<NoteInfo> {
        self.inner.write(|draft| {
            let tag = retag::clean(tag)?;
            self.inner
                .edit_fields(draft, path, |text| {
                    let mut tags = text.tags();
                    if !tags.contains(&tag) {
                        tags.push(tag);
                        text.set_tags(&tags);
                    }
                })
                .map(|note| note.info)
        })
    }

    /// Removes the tag from the note and returns it. Its child tags, its body, other tags and
    /// modification time stay. A note without the tag does not change.
    ///
    /// # Errors
    ///
    /// - [`Error::BadTag`] when `tag` is not a tag name.
    /// - The errors of [`Core::add_tag`] for the file.
    pub fn remove_tag(&self, path: &NotePath, tag: &str) -> Result<NoteInfo> {
        self.inner.write(|draft| {
            let tag = retag::check(tag)?;
            self.inner
                .edit_fields(draft, path, |text| {
                    let mut tags = text.tags();
                    let count = tags.len();
                    tags.retain(|other| other != tag);
                    if tags.len() != count {
                        text.set_tags(&tags);
                    }
                })
                .map(|note| note.info)
        })
    }

    /// Pins or unpins the note and returns it. Its body and modification time stay.
    ///
    /// # Errors
    ///
    /// As in [`Core::add_tag`].
    pub fn set_pinned(&self, path: &NotePath, pinned: bool) -> Result<NoteInfo> {
        self.inner.write(|draft| {
            self.inner
                .edit_fields(draft, path, |text| {
                    if text.pinned() != pinned {
                        text.set_pinned(pinned);
                    }
                })
                .map(|note| note.info)
        })
    }

    /// How many notes [`Core::rename_tag`] changes.
    ///
    /// # Errors
    ///
    /// [`Error::BadTag`] as in [`Core::rename_tag`].
    pub fn tag_rename_count(&self, from: &str, to: &str) -> Result<usize> {
        Ok(self.tag_edit_count(&TagEdit::rename(from, to)?))
    }

    /// Renames the tag `from` to `to` in every note, and returns how many notes changed.
    ///
    /// Child tags follow: `work` to `job` makes `work/holm` into `job/holm`.
    /// When a note already has the new name, the two tags merge.
    /// `to` is cleaned like a typed tag. The notes keep their modification times.
    ///
    /// A note that can't be changed keeps its tags, with a log line. The other notes still change.
    /// Each note changes in one step, but the whole job does not. After a crash, some notes can
    /// have the new tag and some the old one. A second rename then finishes the job.
    ///
    /// # Errors
    ///
    /// - [`Error::BadTag`] when `from` is not a tag name, or nothing is left of `to`.
    /// - The first error of a note, as in [`Core::add_tag`].
    pub fn rename_tag(&self, from: &str, to: &str) -> Result<usize> {
        self.inner
            .write(|draft| self.inner.edit_tag(draft, &TagEdit::rename(from, to)?))
    }

    /// How many notes [`Core::delete_tag`] changes.
    ///
    /// # Errors
    ///
    /// [`Error::BadTag`] when `tag` is not a tag name.
    pub fn tag_delete_count(&self, tag: &str) -> Result<usize> {
        Ok(self.tag_edit_count(&TagEdit::delete(tag)?))
    }

    /// Removes the tag and its child tags from every note, and returns how many notes changed.
    ///
    /// It works as [`Core::rename_tag`], also after a crash.
    ///
    /// # Errors
    ///
    /// - [`Error::BadTag`] when `tag` is not a tag name.
    /// - The first error of a note, as in [`Core::add_tag`].
    pub fn delete_tag(&self, tag: &str) -> Result<usize> {
        self.inner
            .write(|draft| self.inner.edit_tag(draft, &TagEdit::delete(tag)?))
    }

    /// Copies the file into `attachments/` at the root of the notes folder,
    /// and returns its path from the folder of `note`, for a link.
    ///
    /// The copy has a short hash of the content in its name, like `photo-3fa9c2e1.png`.
    /// So the same file twice is one copy. A leading `.` becomes `-`.
    /// Holm makes `attachments/` when it is missing.
    /// The note itself does not change, and neither does the snapshot.
    ///
    /// # Errors
    ///
    /// - [`Error::Read`] when `file` can't be read or is not a file.
    /// - [`Error::Write`] when the copy can't be written.
    pub fn attach_file(&self, note: &NotePath, file: impl AsRef<Path>) -> Result<PathBuf> {
        let file = file.as_ref();
        let read_error = |source| Error::Read {
            path: file.to_owned(),
            source,
        };
        let source = File::open(file).map_err(read_error)?;
        if !source.metadata().map_err(read_error)?.is_file() {
            return Err(read_error(std::io::ErrorKind::IsADirectory.into()));
        }
        let stem = file.file_stem().unwrap_or_default().to_string_lossy();
        let ext = file.extension().map(OsStr::to_string_lossy);
        self.attach(note, &stem, ext.as_deref(), source)
    }

    /// Saves a pasted image into `attachments/`, and returns its path from the folder
    /// of `note`, for a link. `extension` names the image format, like `png`.
    ///
    /// The name is like `Pasted image-3fa9c2e1.png`. Else it works as [`Core::attach_file`].
    ///
    /// # Errors
    ///
    /// [`Error::Write`] when the image can't be written.
    pub fn paste_image(&self, note: &NotePath, extension: &str, bytes: &[u8]) -> Result<PathBuf> {
        self.attach(note, PASTED_IMAGE, Some(extension), bytes)
    }

    fn attach(
        &self,
        note: &NotePath,
        stem: &str,
        ext: Option<&str>,
        content: impl std::io::Read,
    ) -> Result<PathBuf> {
        let file = self.inner.store.add_attachment(stem, ext, content)?;
        Ok(relink::link(self.inner.store.root(), note, &file))
    }

    fn tag_edit_count(&self, edit: &TagEdit<'_>) -> usize {
        self.snapshot()
            .matching(NoteSet::WithTag(edit.tag()))
            .filter(|note| edit.apply(&note.tags) != note.tags)
            .count()
    }

    fn move_folder_to(
        &self,
        folder: &FolderPath,
        parent: &FolderPath,
        name: &Name,
    ) -> Result<FolderPath> {
        self.inner.write(|draft| {
            let new = self.inner.store.move_folder(folder, parent, name)?;
            if new != *folder {
                draft.changes.push(Change::FolderMoved {
                    from: folder.clone(),
                    to: new.clone(),
                });
                let nodes = vec![Node::Folder(folder.clone()), Node::Folder(new.clone())];
                self.inner.refresh(draft, nodes)?;
                draft.folders(folder)?;
                draft.folders(&new)?;
            }
            Ok(new)
        })
    }

    fn build(store: Store, index_dir: &Path, watcher: Option<watch::Watcher>) -> Result<Self> {
        let index = Index::open(index_dir, &store)?;
        let mut facts = Facts::new(index.entries()?, store.folders(&FolderPath::root())?);
        let settled = scan::files(&store, &facts, &Nodes::root())?;
        let links = index.write(&settled)?;
        let (delta, _) = settled.into_delta(links);
        facts.apply(delta);
        let inner = Inner {
            store,
            index,
            writing: Mutex::new(()),
            facts: Mutex::new(facts),
            listeners: Mutex::new(Vec::new()),
            _watcher: watcher,
        };
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    /// The target of the watch seam. It holds the core weakly, so the watch thread
    /// does not keep the core alive.
    fn apply(&self) -> Apply {
        let inner = Arc::downgrade(&self.inner);
        Arc::new(move |batch| inner.upgrade().map(|inner| inner.apply(batch)))
    }
}

/// The next snapshot, while one write runs.
struct Draft<'a> {
    store: &'a Store,
    next: Facts,
    changed: bool,
    /// The change list of the next revision.
    changes: Vec<Change>,
}

impl Draft<'_> {
    /// Applies `delta`, and returns whether a note changed.
    fn notes(&mut self, delta: Delta) -> bool {
        let changed = self.next.apply(delta);
        self.changed |= changed;
        changed
    }

    /// Adds [`Change::Notes`] once, after a write of Holm changed notes with no move.
    fn wrote_notes(&mut self) {
        if !self.changes.contains(&Change::Notes) {
            self.changes.push(Change::Notes);
        }
    }

    /// Reads the folders at `folder` or below it again after a write of Holm, and adds
    /// [`Change::Folders`] once when they changed.
    fn wrote_folders(&mut self, folder: &FolderPath) -> Result<()> {
        if self.folders(folder)? && !self.changes.contains(&Change::Folders) {
            self.changes.push(Change::Folders);
        }
        Ok(())
    }

    /// Reads the folders at `folder` or below it from the disk again. It returns whether they
    /// changed.
    fn folders(&mut self, folder: &FolderPath) -> Result<bool> {
        let found = self.store.folders(folder)?;
        let changed = self.next.set_folders(folder, found);
        self.changed |= changed;
        Ok(changed)
    }
}

impl Inner {
    fn snapshot(&self) -> Snapshot {
        lock(&self.facts).snapshot().clone()
    }

    /// Runs `op` under the write lock, with a draft of the next facts.
    /// The draft replaces the facts, so they keep each new stamp. When a fact changed, the draft
    /// gets the next revision. The change list carries the snapshot of its revision.
    fn write<T>(&self, op: impl FnOnce(&mut Draft<'_>) -> Result<T>) -> Result<T> {
        let _writing = lock(&self.writing);
        let mut draft = Draft {
            store: &self.store,
            next: lock(&self.facts).clone(),
            changed: false,
            changes: Vec::new(),
        };
        let result = op(&mut draft);
        debug_assert!(
            !draft.changed || !draft.changes.is_empty(),
            "a new revision with no change"
        );
        let mut facts = draft.next;
        if draft.changed {
            facts = facts.next_revision();
        }
        if !draft.changes.is_empty() {
            let list = ChangeList {
                snapshot: facts.snapshot().clone(),
                changes: draft.changes,
            };
            lock(&self.listeners).retain(|listener| listener.unbounded_send(list.clone()).is_ok());
        }
        *lock(&self.facts) = facts;
        result
    }

    /// Puts a note that Holm just wrote into the index and the draft, and returns it.
    fn update(&self, draft: &mut Draft<'_>, note: &Note, stamp: store::Stamp) -> Result<Note> {
        let notes = self.update_all(draft, vec![(note.clone(), stamp)])?;
        notes.into_iter().next().ok_or_else(|| Error::Write {
            path: self.store.root().join(&note.info.path),
            source: std::io::ErrorKind::InvalidData.into(),
        })
    }

    /// Puts notes that Holm just wrote into the index and the draft, and returns them.
    fn update_all(
        &self,
        draft: &mut Draft<'_>,
        notes: Vec<(Note, store::Stamp)>,
    ) -> Result<Vec<Note>> {
        let settled = scan::wrote(&self.store, &draft.next, notes);
        let (notes, changed) = self.take(draft, settled)?;
        if changed {
            draft.wrote_notes();
        }
        Ok(notes)
    }

    /// Changes the frontmatter of the note with `edit`, keeps its modification time,
    /// and updates the index and the draft.
    fn edit_fields(
        &self,
        draft: &mut Draft<'_>,
        path: &NotePath,
        edit: impl FnOnce(&mut holm_markdown::Note),
    ) -> Result<Note> {
        let (note, stamp) = self.store.edit_fields(path, edit)?;
        self.update(draft, &note, stamp)
    }

    fn edit_tag(&self, draft: &mut Draft<'_>, edit: &TagEdit<'_>) -> Result<usize> {
        let paths = draft
            .next
            .snapshot()
            .paths(NoteSet::WithTag(edit.tag()), Order::Title);
        let rewritten = self.store.rewrite(&paths, |_, old| {
            let mut text = holm_markdown::Note::parse(old);
            let tags = text.tags();
            let new = edit.apply(&tags);
            (new != tags).then(|| {
                text.set_tags(&new);
                text.to_string()
            })
        });
        let mut written = Vec::with_capacity(rewritten.len());
        let mut first_error = None;
        for (path, result) in rewritten {
            match result.and_then(|()| self.store.read(&path)) {
                Ok(note) => written.push(note),
                Err(error) => {
                    tracing::warn!(%error, "kept the tags of a note");
                    first_error.get_or_insert(error);
                }
            }
        }
        let changed = written.len();
        if changed > 0 {
            self.update_all(draft, written)?;
        }
        if changed > 0
            && let TagEdit::Rename { from, to } = edit
            && from != to
        {
            draft.changes.push(Change::TagMoved {
                from: (*from).to_owned(),
                to: to.clone(),
            });
        }
        first_error.map_or(Ok(changed), Err)
    }

    /// The count of a delete of `node`, and the attachment files it removes.
    fn removal(&self, node: &Node) -> Result<(DeleteCount, Vec<PathBuf>)> {
        let removal = self.index.removal(node)?;
        let files = self.store.attachments(removal.files);
        let count = DeleteCount {
            notes: removal.notes,
            attachments: files.len(),
        };
        Ok((count, files))
    }

    /// Runs `delete` on `node`, then deletes its attachment files.
    /// It updates the index and the draft also when `delete` fails, since a part may be gone.
    fn delete(
        &self,
        draft: &mut Draft<'_>,
        node: &Node,
        delete: impl FnOnce(&Store) -> Result<()>,
    ) -> Result<()> {
        let (_, files) = self.removal(node)?;
        let deleted = delete(&self.store);
        if deleted.is_ok() {
            for file in files {
                if let Err(error) = self.store.delete_attachment(&file) {
                    tracing::warn!(%error, "kept an attachment");
                }
            }
        }
        if self.refresh(draft, vec![node.clone()])? {
            draft.wrote_notes();
        }
        if let Node::Folder(folder) = node {
            draft.wrote_folders(folder)?;
        }
        deleted
    }

    /// Updates the index and the draft after a note moved from `from` to `to`, and returns the note.
    fn settle(&self, draft: &mut Draft<'_>, from: &NotePath, to: &NotePath) -> Result<NoteInfo> {
        if from == to {
            return self.store.read(to).map(|(note, _)| note.info);
        }
        self.refresh(
            draft,
            vec![Node::Note(from.clone()), Node::Note(to.clone())],
        )?;
        let info = self.store.read(to)?.0.info;
        draft.changes.push(Change::Moved {
            from: from.clone(),
            note: info.clone(),
        });
        Ok(info)
    }

    /// Updates the index and the draft at each path after Holm's own change, and returns
    /// whether a note changed. It adds no change to the list, so the write adds its own.
    fn refresh(&self, draft: &mut Draft<'_>, nodes: Vec<Node>) -> Result<bool> {
        let settled = scan::files(&self.store, &draft.next, &Nodes::new(nodes))?;
        Ok(self.take(draft, settled)?.1)
    }

    /// Writes the rows of `settled` into the index and puts it in the draft. It returns the
    /// notes that were read, and whether a note changed.
    fn take(&self, draft: &mut Draft<'_>, settled: Settled) -> Result<(Vec<Note>, bool)> {
        let links = self.index.write(&settled)?;
        let (delta, notes) = settled.into_delta(links);
        Ok((notes, draft.notes(delta)))
    }

    /// Checks the disk at each path of `batch`, and updates the index, the snapshot and the
    /// change list.
    fn apply(&self, batch: Batch) -> Result<()> {
        self.write(|draft| {
            let nodes = Nodes::outside(&self.store, draft.next.snapshot(), batch);
            if nodes.is_empty() {
                return Ok(());
            }
            let settled = scan::files(&self.store, &draft.next, &nodes)?;
            let refresh = reconcile::changes(&draft.next, &settled);
            self.take(draft, settled)?;
            let mut folders = false;
            for node in &nodes {
                if let Node::Folder(folder) = node {
                    folders |= draft.folders(folder)?;
                }
            }
            draft.changes.extend(refresh.notes);
            if folders {
                draft.changes.push(Change::Folders);
            }
            if refresh.tags {
                draft.changes.push(Change::Tags);
            }
            Ok(())
        })
    }
}

/// A stream of change lists. [`Core::changes`] makes it.
///
/// It ends when every [`Core`] handle is gone.
#[derive(Debug)]
pub struct Changes {
    receive: UnboundedReceiver<ChangeList>,
}

impl Changes {
    /// The next change list, with no wait.
    ///
    /// It returns `None` when no list waits, or when every [`Core`] handle is gone.
    pub fn try_next(&mut self) -> Option<ChangeList> {
        self.receive.try_recv().ok()
    }
}

impl Stream for Changes {
    type Item = ChangeList;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<ChangeList>> {
        Pin::new(&mut self.receive).poll_next(cx)
    }
}

/// A lock that still works after a panic in another thread. The data it guards stays valid.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
