//! The core of Holm: notes, files, links, the index and file watching.
//!
//! It has no UI. The app and the later manager use it through one [`Core`] handle.

mod error;
mod index;
pub mod model;
mod relink;
mod retag;
mod snapshot;
mod store;
mod watch;

use std::collections::{HashMap, hash_map};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

pub use error::{Error, NameProblem, Result};
pub use snapshot::Snapshot;
pub use watch::Feed;

use index::Index;
use model::{Change, DeleteCount, LinkTarget, Note, NoteId, NoteInfo, Order};
use retag::TagEdit;
use snapshot::Delta;
use store::{Store, folder_of};
use watch::{Apply, Batch};

/// The stem of a pasted image before its hash.
const PASTED_IMAGE: &str = "Pasted image";

/// The handle to one notes folder.
///
/// It is cheap to clone and safe to share across threads. Its methods block,
/// so the app calls them away from the UI thread.
///
/// All note and folder paths are relative to the notes folder, like `work/Plans.md`.
/// The root folder is the empty path.
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
    snapshot: Mutex<Snapshot>,
    listeners: Mutex<Vec<Sender<Change>>>,
    _watcher: Option<watch::Watcher>,
}

impl Core {
    /// Opens the notes folder and brings the index in `index_dir` up to date with the files.
    ///
    /// Then it watches the notes folder. Changes from other tools update the index
    /// and go to each [`Changes`]. When the watch can't start, the core works without it,
    /// with a log line.
    ///
    /// Then it keeps the facts of the notes folder in memory, as a [`Snapshot`].
    ///
    /// Holm can delete the index and rebuild it from the files at any time.
    /// It does so when the index is broken, or when it comes from a Holm with another index shape.
    ///
    /// When two note files have the same id, the file that the index already knows keeps it.
    /// The other file gets a new id, written into the file.
    ///
    /// # Errors
    ///
    /// - [`Error::NotesFolder`] when `notes_folder` is missing or is not a folder.
    /// - [`Error::Read`] when the notes folder can't be read.
    /// - [`Error::Index`] when the index can't be written, for example on a full disk.
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

    /// A new stream of the changes that come from outside Holm.
    ///
    /// Each stream gets every change from the time it was made.
    /// A change list is short and comes once for each batch of file events:
    /// first the note changes in path order, then [`Change::Folders`] and [`Change::Tags`].
    pub fn changes(&self) -> Changes {
        let (send, receive) = mpsc::channel();
        lock(&self.inner.listeners).push(send);
        Changes { receive }
    }

    /// The notes whose title, text or tags hold every word of `text`.
    ///
    /// A word also matches longer words that start with it, so `pla` finds "plans".
    /// Capital letters and accents make no difference. With no words, the list is empty.
    ///
    /// # Errors
    ///
    /// [`Error::Index`] when the index can't be read, also after a rebuild.
    pub fn search(&self, text: &str, order: Order) -> Result<Vec<NoteInfo>> {
        self.inner.index.search(&self.inner.store, text, order)
    }

    /// The text of each heading in the note at `path`, in order.
    ///
    /// # Errors
    ///
    /// As in [`Core::read`].
    pub fn headings(&self, path: impl AsRef<Path>) -> Result<Vec<String>> {
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
    /// - [`Error::BadPath`] when `path` is not a `.md` file inside the notes folder.
    /// - As in [`Core::read`], when a linked note can't be read.
    pub fn broken_links(&self, path: impl AsRef<Path>) -> Result<Vec<LinkTarget>> {
        let path = path.as_ref();
        Store::check_note(path)?;
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
    /// notes folder gets a path from the folder of the note. A file outside keeps its absolute
    /// path.
    ///
    /// # Errors
    ///
    /// [`Error::BadPath`] when `note` is not a `.md` path inside the notes folder, or `file` is
    /// not absolute.
    pub fn file_link(&self, note: impl AsRef<Path>, file: impl AsRef<Path>) -> Result<PathBuf> {
        let (note, file) = (note.as_ref(), file.as_ref());
        Store::check_note(note)?;
        if !file.is_absolute() {
            return Err(Error::BadPath {
                path: file.to_owned(),
            });
        }
        let file = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_owned());
        let root = self.inner.store.root();
        if file.starts_with(root) {
            Ok(relink::relative(&root.join(folder_of(note)), &file))
        } else {
            Ok(file)
        }
    }

    /// Reads one note.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] when `path` is not a `.md` file inside the notes folder.
    /// - [`Error::Read`] or [`Error::NotUtf8`] when the file can't be read as text.
    pub fn read(&self, path: impl AsRef<Path>) -> Result<Note> {
        self.inner.store.read(path.as_ref()).map(|(note, _)| note)
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
    /// - [`Error::BadPath`] as in [`Core::read`].
    /// - [`Error::Read`] or [`Error::NotUtf8`] when the file on disk can't be read.
    /// - [`Error::Write`] when the new file can't be written. The old file then stays.
    /// - [`Error::Index`] as in [`Core::search`]. The file is saved all the same.
    pub fn save(&self, path: impl AsRef<Path>, body: &str) -> Written<Note> {
        self.inner.write(|draft| {
            let (note, stamp) = self.inner.store.save(path.as_ref(), body)?;
            self.inner.update(draft, &note, stamp)
        })
    }

    /// Writes `note` again at its path after another tool deleted its file, and returns it.
    /// The note keeps its id, so links to it still work. It also keeps its tags and its pin.
    /// When a file is at the path again, it saves the body over it like [`Core::save`].
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] as in [`Core::read`].
    /// - [`Error::Write`] when the file can't be written, for example when the folder is missing.
    /// - [`Error::Index`] as in [`Core::search`]. The file is saved all the same.
    pub fn restore(&self, note: &Note) -> Written<Note> {
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
    /// - [`Error::BadPath`] when `folder` is not a plain folder path inside the notes folder.
    /// - [`Error::Write`] when the file can't be written, for example when the folder is missing.
    /// - [`Error::Index`] as in [`Core::save`].
    pub fn create(&self, folder: impl AsRef<Path>, title: &str) -> Written<Note> {
        self.inner.write(|draft| {
            let (note, stamp) = self.inner.store.create(folder.as_ref(), title)?;
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
    /// - [`Error::BadPath`] as in [`Core::read`].
    /// - [`Error::Read`] or [`Error::NotUtf8`] when the file can't be read as text.
    /// - [`Error::Write`] when the file can't be moved. The note then stays where it was.
    /// - [`Error::Index`] as in [`Core::save`].
    pub fn rename_note(&self, path: impl AsRef<Path>, title: &str) -> Written<NoteInfo> {
        let path = path.as_ref();
        self.inner.write(|draft| {
            Store::check_note(path)?;
            let folder = folder_of(path);
            let name = store::check_title(title)?;
            let new = self
                .inner
                .store
                .move_note(path, folder, OsStr::new(&name))?;
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
    /// - [`Error::BadPath`] when `path` is not a note path, or `folder` is not a folder path.
    /// - Else as in [`Core::rename_note`].
    pub fn move_note(&self, path: impl AsRef<Path>, folder: impl AsRef<Path>) -> Written<NoteInfo> {
        let path = path.as_ref();
        self.inner.write(|draft| {
            Store::check_note(path)?;
            let name = path.file_name().unwrap_or_default();
            let new = self.inner.store.move_note(path, folder.as_ref(), name)?;
            self.inner.settle(draft, path, &new)
        })
    }

    /// Creates the folder `name` in `parent`, and returns its path.
    ///
    /// Holm uses `name` as it is and never adds a number.
    ///
    /// # Errors
    ///
    /// - [`Error::BadName`] when `name` is empty, holds `/`, starts with `.`, is too long,
    ///   or is `attachments` at the root.
    /// - [`Error::NameTaken`] when something in `parent` has the name, also with other
    ///   capital letters.
    /// - [`Error::BadPath`] when `parent` is not a folder path.
    /// - [`Error::Write`] when the folder can't be made, for example when `parent` is missing.
    /// - [`Error::Read`] when the new folder can't be read.
    pub fn create_folder(&self, parent: impl AsRef<Path>, name: &str) -> Written<PathBuf> {
        self.inner.write(|draft| {
            let new = self.inner.store.create_folder(parent.as_ref(), name)?;
            draft.folders(&new)?;
            Ok(new)
        })
    }

    /// Renames the folder to `name`, and returns its new path.
    ///
    /// Names work as in [`Core::create_folder`]. A change of only the capital letters works.
    /// Relative file links in its notes change, so they point to the same files.
    /// A note whose links can't change keeps them, with a log line.
    ///
    /// # Errors
    ///
    /// - [`Error::BadName`] and [`Error::NameTaken`] as in [`Core::create_folder`].
    ///   Nothing moves.
    /// - [`Error::BadPath`] when `folder` is the root or `attachments/`,
    ///   or is not a folder path.
    /// - [`Error::Read`] when `folder` is missing or is not a folder.
    /// - [`Error::Write`] when the folder can't be moved. It then stays where it was.
    /// - [`Error::Index`] as in [`Core::save`].
    pub fn rename_folder(&self, folder: impl AsRef<Path>, name: &str) -> Written<PathBuf> {
        let folder = folder.as_ref();
        self.move_folder_to(folder, folder_of(folder), name)
    }

    /// Moves the folder into `parent` with its name, and returns its new path.
    ///
    /// It works as [`Core::rename_folder`].
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] also when `parent` is `folder` or inside it.
    /// - Else as in [`Core::rename_folder`].
    pub fn move_folder(
        &self,
        folder: impl AsRef<Path>,
        parent: impl AsRef<Path>,
    ) -> Written<PathBuf> {
        let folder = folder.as_ref();
        match folder.file_name() {
            Some(name) => self.move_folder_to(folder, parent.as_ref(), &name.to_string_lossy()),
            None => self.inner.write(|_| {
                Err(Error::BadPath {
                    path: folder.to_owned(),
                })
            }),
        }
    }

    /// What [`Core::delete_note`] removes: the note and its attachment files.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] as in [`Core::read`].
    /// - [`Error::Index`] as in [`Core::search`].
    pub fn note_delete_count(&self, path: impl AsRef<Path>) -> Result<DeleteCount> {
        let path = path.as_ref();
        Store::check_note(path)?;
        self.inner.removal(path).map(|(count, _)| count)
    }

    /// What [`Core::delete_folder`] removes: the notes in the folder and below it,
    /// and their attachment files.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] when `folder` is not a plain folder path inside the notes folder.
    /// - [`Error::Index`] as in [`Core::search`].
    pub fn folder_delete_count(&self, folder: impl AsRef<Path>) -> Result<DeleteCount> {
        let folder = folder.as_ref();
        Store::check_folder(folder)?;
        self.inner.removal(folder).map(|(count, _)| count)
    }

    /// Deletes the note forever. Holm has no trash.
    ///
    /// It also deletes each file in `attachments/` that the note links and no other note links.
    /// It never deletes a linked file outside `attachments/`.
    /// A note that is already gone is no error.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] as in [`Core::read`].
    /// - [`Error::Write`] when the note can't be deleted.
    ///   An attachment that can't be deleted stays, with a log line.
    /// - [`Error::Index`] as in [`Core::save`].
    pub fn delete_note(&self, path: impl AsRef<Path>) -> Written<()> {
        let path = path.as_ref();
        self.inner.write(|draft| {
            Store::check_note(path)?;
            self.inner
                .delete(draft, path, |store| store.delete_note(path))
        })
    }

    /// Deletes the folder forever, with every note and file in it.
    ///
    /// Attachments go as in [`Core::delete_note`]. A folder that is already gone is no error.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] when `folder` is the root or `attachments/`, or is not a folder path.
    /// - [`Error::Read`] when `folder` is not a folder.
    /// - [`Error::Write`] when the folder can't be deleted. Some files in it may be gone.
    /// - [`Error::Index`] as in [`Core::save`].
    pub fn delete_folder(&self, folder: impl AsRef<Path>) -> Written<()> {
        let folder = folder.as_ref();
        self.inner.write(|draft| {
            Store::check_folder(folder)?;
            self.inner
                .delete(draft, folder, |store| store.delete_folder(folder))
        })
    }

    /// The id of the note, for a link to it.
    ///
    /// A note with no id gets one. Only the `id:` line is added, and the file keeps
    /// its modification time.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] as in [`Core::read`].
    /// - [`Error::Read`] or [`Error::NotUtf8`] when the file can't be read as text.
    /// - [`Error::Write`] when the id can't be written.
    /// - [`Error::Index`] as in [`Core::save`].
    pub fn link_id(&self, path: impl AsRef<Path>) -> Written<NoteId> {
        self.inner.write(|draft| {
            let note = self.inner.edit_fields(draft, path.as_ref(), |text| {
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
    /// - [`Error::BadPath`] as in [`Core::read`].
    /// - [`Error::Read`] or [`Error::NotUtf8`] when the file can't be read as text.
    /// - [`Error::Write`] when the file can't be written. The old file then stays.
    /// - [`Error::Index`] as in [`Core::save`].
    pub fn add_tag(&self, path: impl AsRef<Path>, tag: &str) -> Written<NoteInfo> {
        self.inner.write(|draft| {
            let tag = retag::clean(tag)?;
            self.inner
                .edit_fields(draft, path.as_ref(), |text| {
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
    pub fn remove_tag(&self, path: impl AsRef<Path>, tag: &str) -> Written<NoteInfo> {
        self.inner.write(|draft| {
            let tag = retag::check(tag)?;
            self.inner
                .edit_fields(draft, path.as_ref(), |text| {
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
    pub fn set_pinned(&self, path: impl AsRef<Path>, pinned: bool) -> Written<NoteInfo> {
        self.inner.write(|draft| {
            self.inner
                .edit_fields(draft, path.as_ref(), |text| {
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
    ///
    /// # Errors
    ///
    /// - [`Error::BadTag`] when `from` is not a tag name, or nothing is left of `to`.
    /// - The first error of a note, as in [`Core::add_tag`].
    pub fn rename_tag(&self, from: &str, to: &str) -> Written<usize> {
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
    /// It works as [`Core::rename_tag`].
    ///
    /// # Errors
    ///
    /// - [`Error::BadTag`] when `tag` is not a tag name.
    /// - The first error of a note, as in [`Core::add_tag`].
    pub fn delete_tag(&self, tag: &str) -> Written<usize> {
        self.inner
            .write(|draft| self.inner.edit_tag(draft, &TagEdit::delete(tag)?))
    }

    /// Copies the file into `attachments/` at the root of the notes folder,
    /// and returns its path from the folder of `note`, for a link.
    ///
    /// The copy is `<stem>-<hash>.<ext>`, with a short hash of the content: `photo-3fa9c2e1.png`.
    /// So the same file twice is one copy, and two files never clash.
    /// A leading `.` becomes `-`. Holm makes `attachments/` when it is missing.
    /// The note itself does not change, and neither does the snapshot.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] when `note` is not a `.md` path inside the notes folder.
    /// - [`Error::Read`] when `file` can't be read or is not a file.
    /// - [`Error::Write`] when the copy can't be written.
    pub fn attach_file(&self, note: impl AsRef<Path>, file: impl AsRef<Path>) -> Result<PathBuf> {
        let file = file.as_ref();
        let read_error = |source| Error::Read {
            path: file.to_owned(),
            source,
        };
        let source = File::open(file).map_err(read_error)?;
        if !source.metadata().map_err(read_error)?.is_file() {
            return Err(read_error(std::io::ErrorKind::IsADirectory.into()));
        }
        let stem =
            store::file_stem(&file.file_stem().unwrap_or_default().to_string_lossy());
        let ext = file
            .extension()
            .map(|ext| format!(".{}", ext.to_string_lossy()))
            .unwrap_or_default();
        self.attach(note.as_ref(), &OsString::from(stem), &ext, source)
    }

    /// Saves a pasted image into `attachments/`, and returns its path from the folder
    /// of `note`, for a link. `extension` names the image format, like `png`.
    ///
    /// The name is `Pasted image-<hash>.<extension>`. Else it works as [`Core::attach_file`].
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] as in [`Core::attach_file`].
    /// - [`Error::Write`] when the image can't be written.
    pub fn paste_image(
        &self,
        note: impl AsRef<Path>,
        extension: &str,
        bytes: &[u8],
    ) -> Result<PathBuf> {
        let ext = format!(".{}", store::file_stem(extension));
        self.attach(note.as_ref(), OsStr::new(PASTED_IMAGE), &ext, bytes)
    }

    fn attach(
        &self,
        note: &Path,
        stem: &OsStr,
        ext: &str,
        content: impl std::io::Read,
    ) -> Result<PathBuf> {
        Store::check_note(note)?;
        let file = self.inner.store.add_attachment(stem, ext, content)?;
        let root = self.inner.store.root();
        Ok(relink::relative(
            &root.join(folder_of(note)),
            &root.join(file),
        ))
    }

    fn tag_edit_count(&self, edit: &TagEdit<'_>) -> usize {
        self.snapshot()
            .notes_with_tag(edit.tag(), Order::Title)
            .iter()
            .filter(|note| edit.apply(&note.tags) != note.tags)
            .count()
    }

    fn move_folder_to(&self, folder: &Path, parent: &Path, name: &str) -> Written<PathBuf> {
        self.inner.write(|draft| {
            let new = self.inner.store.move_folder(folder, parent, name)?;
            if new != folder {
                self.inner
                    .refresh(draft, vec![folder.to_owned(), new.clone()])?;
                draft.folders(folder)?;
                draft.folders(&new)?;
            }
            Ok(new)
        })
    }

    fn build(store: Store, index_dir: &Path, watcher: Option<watch::Watcher>) -> Result<Self> {
        let index = Index::open(index_dir, &store)?;
        let snapshot = Snapshot::new(index.entries(&store)?, store.folders(Path::new(""))?);
        let inner = Inner {
            store,
            index,
            writing: Mutex::new(()),
            snapshot: Mutex::new(snapshot),
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

/// What a write gives back: its result, and the snapshot right after it.
///
/// The snapshot comes also after a failed write. A part of that write may be done,
/// like the first notes of [`Core::rename_tag`].
#[derive(Debug)]
#[must_use]
pub struct Written<T> {
    /// What the write gives, or why it failed.
    pub result: Result<T>,
    /// The facts of the notes folder right after the write.
    pub snapshot: Snapshot,
}

/// The next snapshot, while one write runs.
struct Draft<'a> {
    store: &'a Store,
    next: Snapshot,
    changed: bool,
}

impl Draft<'_> {
    fn notes(&mut self, delta: Delta) {
        self.changed |= self.next.apply(delta);
    }

    /// Reads the folders at or below `scope` from the disk again. It returns whether they changed.
    fn folders(&mut self, scope: &Path) -> Result<bool> {
        let found = self.store.folders(scope)?;
        let changed = self.next.set_folders(scope, found);
        self.changed |= changed;
        Ok(changed)
    }
}

impl Inner {
    fn snapshot(&self) -> Snapshot {
        lock(&self.snapshot).clone()
    }

    /// Runs `op` under the write lock, with a draft of the next snapshot.
    /// When a fact changed, the draft gets the next revision and replaces the snapshot.
    fn write<T>(&self, op: impl FnOnce(&mut Draft<'_>) -> Result<T>) -> Written<T> {
        let _writing = lock(&self.writing);
        let mut draft = Draft {
            store: &self.store,
            next: self.snapshot(),
            changed: false,
        };
        let result = op(&mut draft);
        let mut snapshot = draft.next;
        if draft.changed {
            snapshot = snapshot.next_revision();
            *lock(&self.snapshot) = snapshot.clone();
        }
        Written { result, snapshot }
    }

    /// Puts a note that Holm just wrote into the index and the draft, and returns it.
    fn update(&self, draft: &mut Draft<'_>, note: &Note, stamp: store::Stamp) -> Result<Note> {
        let (note, delta) = self.index.update(&self.store, note, stamp)?;
        draft.notes(delta);
        Ok(note)
    }

    /// Changes the frontmatter of the note with `edit`, keeps its modification time,
    /// and updates the index and the draft.
    fn edit_fields(
        &self,
        draft: &mut Draft<'_>,
        path: &Path,
        edit: impl FnOnce(&mut holm_markdown::Note),
    ) -> Result<Note> {
        let (note, stamp) = self.store.edit_fields(path, edit)?;
        self.update(draft, &note, stamp)
    }

    fn edit_tag(&self, draft: &mut Draft<'_>, edit: &TagEdit<'_>) -> Result<usize> {
        let mut changed = 0;
        let mut first_error = None;
        for info in draft.next.notes_with_tag(edit.tag(), Order::Title) {
            let mut hit = false;
            let result = self.edit_fields(draft, &info.path, |text| {
                let old = text.tags();
                let new = edit.apply(&old);
                if new != old {
                    text.set_tags(&new);
                    hit = true;
                }
            });
            match result {
                Ok(_) if hit => changed += 1,
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(%error, "kept the tags of a note");
                    first_error.get_or_insert(error);
                }
            }
        }
        first_error.map_or(Ok(changed), Err)
    }

    /// The count of a delete of `scope`, and the attachment files it removes.
    fn removal(&self, scope: &Path) -> Result<(DeleteCount, Vec<PathBuf>)> {
        let removal = self.index.removal(&self.store, scope)?;
        let files: Vec<PathBuf> = removal
            .files
            .into_iter()
            .filter(|file| self.store.is_attachment(file))
            .collect();
        let count = DeleteCount {
            notes: removal.notes,
            attachments: files.len(),
        };
        Ok((count, files))
    }

    /// Runs `delete` on `scope`, then deletes its attachment files.
    /// It updates the index and the draft also when `delete` fails, since a part may be gone.
    fn delete(
        &self,
        draft: &mut Draft<'_>,
        scope: &Path,
        delete: impl FnOnce(&Store) -> Result<()>,
    ) -> Result<()> {
        let (_, files) = self.removal(scope)?;
        let deleted = delete(&self.store);
        if deleted.is_ok() {
            for file in files {
                if let Err(error) = self.store.delete_attachment(&file) {
                    tracing::warn!(%error, "kept an attachment");
                }
            }
        }
        self.refresh(draft, vec![scope.to_owned()])?;
        draft.folders(scope)?;
        deleted
    }

    /// Updates the index and the draft after a note moved from `from` to `to`, and returns the note.
    fn settle(&self, draft: &mut Draft<'_>, from: &Path, to: &Path) -> Result<NoteInfo> {
        if from != to {
            self.refresh(draft, vec![from.to_owned(), to.to_owned()])?;
        }
        self.store.read(to).map(|(note, _)| note.info)
    }

    /// Updates the index and the draft at each path after Holm's own change.
    /// No change goes to [`Changes`].
    fn refresh(&self, draft: &mut Draft<'_>, mut paths: Vec<PathBuf>) -> Result<()> {
        paths.sort();
        let refresh = self.index.refresh(&self.store, &outer(paths))?;
        draft.notes(refresh.delta);
        Ok(())
    }

    /// Checks the disk at each path of `batch`, updates the index and the snapshot,
    /// and sends the changes.
    fn apply(&self, batch: Batch) -> Result<()> {
        // The batch is sorted.
        let scopes = outer(
            batch
                .into_iter()
                .filter(|path| Store::check_folder(path).is_ok()),
        );
        if scopes.is_empty() {
            return Ok(());
        }
        self.write(|draft| {
            let refresh = self.index.refresh(&self.store, &scopes)?;
            draft.notes(refresh.delta);
            let mut folders = false;
            for scope in &scopes {
                folders |= draft.folders(scope)?;
            }
            let mut changes = refresh.notes;
            if folders {
                changes.push(Change::Folders);
            }
            if refresh.tags {
                changes.push(Change::Tags);
            }
            if !changes.is_empty() {
                lock(&self.listeners).retain(|listener| {
                    changes
                        .iter()
                        .all(|change| listener.send(change.clone()).is_ok())
                });
            }
            Ok(())
        })
        .result
    }
}

/// A stream of the changes that come from outside Holm. [`Core::changes`] makes it.
///
/// As an [`Iterator`], it waits for each change. It ends when every [`Core`] handle is gone.
#[derive(Debug)]
pub struct Changes {
    receive: Receiver<Change>,
}

impl Changes {
    /// Waits up to `timeout` for the next change.
    ///
    /// It returns `None` when no change came in time, or when every [`Core`] handle is gone.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Change> {
        self.receive.recv_timeout(timeout).ok()
    }
}

impl Iterator for Changes {
    type Item = Change;

    fn next(&mut self) -> Option<Change> {
        self.receive.recv().ok()
    }
}

/// The paths that are not inside another of them. `paths` must be sorted,
/// so a folder comes before the paths inside it.
fn outer(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut outer: Vec<PathBuf> = Vec::new();
    for path in paths {
        if !outer.iter().any(|o| path.starts_with(o)) {
            outer.push(path);
        }
    }
    outer
}

/// A lock that still works after a panic in another thread. The data it guards stays valid.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
