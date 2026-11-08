//! The core of Holm: notes, files, links, the index and file watching.
//!
//! It has no UI. The app and the later manager use it through one [`Core`] handle.

mod error;
mod index;
pub mod model;
mod relink;
mod retag;
mod store;
mod watch;

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

pub use error::{Error, Result};
pub use watch::Feed;

use index::{Filter, Index};
use model::{Change, DeleteCount, Link, Note, NoteId, NoteInfo, Order, Tag};
use retag::TagEdit;
use store::{ATTACHMENTS, Entry, Store, folder_of};
use watch::{Apply, Batch};

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
    /// Held while a write and its index update run, so a watch batch never sees half of it.
    writing: Mutex<()>,
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

    /// Every note in the notes folder and its subfolders, in `order`. Pinned notes are on top.
    ///
    /// Only `*.md` files are notes. Hidden files and folders, and symlinks, are skipped.
    /// A note that can't be read is left out, with a log line.
    ///
    /// # Errors
    ///
    /// [`Error::Index`] when the index can't be read, also after a rebuild.
    pub fn notes(&self, order: Order) -> Result<Vec<NoteInfo>> {
        self.list(&Filter::All, order)
    }

    /// The notes right in `folder`, without the notes of its subfolders.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] when `folder` is not a plain folder path inside the notes folder.
    /// - [`Error::Index`] as in [`Core::notes`].
    pub fn notes_in(&self, folder: impl AsRef<Path>, order: Order) -> Result<Vec<NoteInfo>> {
        let folder = folder.as_ref();
        Store::check_folder(folder)?;
        self.list(&Filter::Folder(folder), order)
    }

    /// Every tag as a tree. `work` holds `work/holm`.
    ///
    /// A parent tag shows even when no note has it, only its children.
    ///
    /// # Errors
    ///
    /// [`Error::Index`] as in [`Core::notes`].
    pub fn tags(&self) -> Result<Vec<Tag>> {
        self.inner.index.tags(&self.inner.store)
    }

    /// The notes with `tag` or one of its child tags. Tags match exactly, also in capital letters.
    ///
    /// # Errors
    ///
    /// [`Error::Index`] as in [`Core::notes`].
    pub fn notes_with_tag(&self, tag: &str, order: Order) -> Result<Vec<NoteInfo>> {
        self.list(&Filter::Tag(tag), order)
    }

    /// The notes whose title, text or tags hold every word of `text`.
    ///
    /// A word also matches longer words that start with it, so `pla` finds "plans".
    /// Capital letters and accents make no difference. With no words, the list is empty.
    ///
    /// # Errors
    ///
    /// [`Error::Index`] as in [`Core::notes`].
    pub fn search(&self, text: &str, order: Order) -> Result<Vec<NoteInfo>> {
        self.list(&Filter::Search(text), order)
    }

    /// The links and embedded files in the note at `path`, in the order of its text.
    ///
    /// A note that the index does not know has no links.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] when `path` is not a `.md` file inside the notes folder.
    /// - [`Error::Index`] as in [`Core::notes`].
    pub fn links(&self, path: impl AsRef<Path>) -> Result<Vec<Link>> {
        let path = path.as_ref();
        Store::check_note(path)?;
        self.inner.index.links(&self.inner.store, path)
    }

    /// The other notes that link to the note at `path`, by its id or by its file path.
    ///
    /// # Errors
    ///
    /// As in [`Core::links`].
    pub fn backlinks(&self, path: impl AsRef<Path>, order: Order) -> Result<Vec<NoteInfo>> {
        let path = path.as_ref();
        Store::check_note(path)?;
        self.list(&Filter::Backlinks(path), order)
    }

    /// Every subfolder of the notes folder, sorted by path.
    ///
    /// Hidden folders, symlinks and the `attachments/` folder at the root are skipped.
    ///
    /// # Errors
    ///
    /// [`Error::Read`] when the notes folder itself can't be read.
    pub fn folders(&self) -> Result<Vec<PathBuf>> {
        self.inner.store.folders()
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

    /// Saves the body, tags and pin of `note` to the file at `note.info.path`,
    /// and returns the note as it is now on disk.
    ///
    /// The id, the title and the modification time come from the file, not from `note`.
    /// Every frontmatter field that Holm does not know stays as it is.
    ///
    /// - A note with no `id:` gets one.
    /// - A body with no final line break gets one.
    /// - When the body stays the same, the file keeps its old modification time.
    ///   So a change of only the tags or the pin does not move the note in the "last change" order.
    /// - With no change at all, Holm does not write the file.
    ///
    /// The write goes to a temp file first, which then replaces the note in one step.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] as in [`Core::read`].
    /// - [`Error::Read`] or [`Error::NotUtf8`] when the file on disk can't be read.
    /// - [`Error::Write`] when the new file can't be written. The old file then stays.
    /// - [`Error::Index`] as in [`Core::notes`]. The file is saved all the same.
    pub fn save(&self, note: &Note) -> Result<Note> {
        let _writing = lock(&self.inner.writing);
        let (note, stamp) = self.inner.store.save(note)?;
        self.inner.index.update(&self.inner.store, &note, stamp)
    }

    /// Creates an empty note with a new id in `folder`, and returns it.
    ///
    /// The file is `Untitled.md`, or `Untitled 2.md`, `Untitled 3.md` and so on when the name is taken.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] when `folder` is not a plain folder path inside the notes folder.
    /// - [`Error::Write`] when the file can't be written, for example when the folder is missing.
    /// - [`Error::Index`] as in [`Core::save`].
    pub fn create(&self, folder: impl AsRef<Path>) -> Result<Note> {
        let _writing = lock(&self.inner.writing);
        let (note, stamp) = self.inner.store.create(folder.as_ref())?;
        self.inner.index.update(&self.inner.store, &note, stamp)
    }

    /// Renames the note file to match its title, and returns the note at its new path.
    ///
    /// The name keeps the title as it is. Only `/` and a leading `.` become `-`.
    /// When the name is taken, it gets a number: `Plans 2.md`.
    /// A note whose name already matches stays as it is.
    ///
    /// No other note changes, because note links use the id.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] as in [`Core::read`].
    /// - [`Error::Read`] or [`Error::NotUtf8`] when the file can't be read as text.
    /// - [`Error::Write`] when the file can't be moved. The note then stays where it was.
    /// - [`Error::Index`] as in [`Core::save`].
    pub fn rename_to_title(&self, path: impl AsRef<Path>) -> Result<NoteInfo> {
        let path = path.as_ref();
        let _writing = lock(&self.inner.writing);
        let (note, _) = self.inner.store.read(path)?;
        let stem = holm_markdown::file_stem(&note.info.title);
        let new = self
            .inner
            .store
            .move_note(path, folder_of(path), OsStr::new(&stem))?;
        self.settle(path, &new)
    }

    /// Moves the note to `folder` and returns it at its new path.
    ///
    /// It keeps its file name. When the name is taken, it gets a number: `Plans 2.md`.
    /// Relative file links in the note change, so they point to the same files.
    /// No other note changes, because note links use the id.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] when `path` is not a note path, or `folder` is not a folder path.
    /// - Else as in [`Core::rename_to_title`].
    pub fn move_note(&self, path: impl AsRef<Path>, folder: impl AsRef<Path>) -> Result<NoteInfo> {
        let path = path.as_ref();
        let _writing = lock(&self.inner.writing);
        Store::check_note(path)?;
        let stem = path.file_stem().unwrap_or_default();
        let new = self.inner.store.move_note(path, folder.as_ref(), stem)?;
        self.settle(path, &new)
    }

    /// Creates the folder `name` in `parent`, and returns its path.
    ///
    /// `name` is cleaned like a note file name. When it is taken, it gets a number: `Work 2`.
    /// The name `attachments` at the root is always taken.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] when `parent` is not a folder path, or `name` is empty.
    /// - [`Error::Write`] when the folder can't be made, for example when `parent` is missing.
    pub fn create_folder(&self, parent: impl AsRef<Path>, name: &str) -> Result<PathBuf> {
        let _writing = lock(&self.inner.writing);
        self.inner.store.create_folder(parent.as_ref(), name)
    }

    /// Renames the folder to `name`, and returns its new path.
    ///
    /// Names work as in [`Core::create_folder`]. Relative file links in its notes change,
    /// so they point to the same files. A note whose links can't change keeps them,
    /// with a log line.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] when `folder` is the root or `attachments/`,
    ///   or is not a folder path, or `name` is empty.
    /// - [`Error::Read`] when `folder` is missing or is not a folder.
    /// - [`Error::Write`] when the folder can't be moved. It then stays where it was.
    /// - [`Error::Index`] as in [`Core::save`].
    pub fn rename_folder(&self, folder: impl AsRef<Path>, name: &str) -> Result<PathBuf> {
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
    ) -> Result<PathBuf> {
        let folder = folder.as_ref();
        let name = folder
            .file_name()
            .ok_or_else(|| Error::BadPath {
                path: folder.to_owned(),
            })?
            .to_string_lossy();
        self.move_folder_to(folder, parent.as_ref(), &name)
    }

    /// What [`Core::delete_note`] removes: the note and its attachment files.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] as in [`Core::read`].
    /// - [`Error::Index`] as in [`Core::notes`].
    pub fn note_delete_count(&self, path: impl AsRef<Path>) -> Result<DeleteCount> {
        let path = path.as_ref();
        Store::check_note(path)?;
        self.removal(path).map(|(count, _)| count)
    }

    /// What [`Core::delete_folder`] removes: the notes in the folder and below it,
    /// and their attachment files.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] as in [`Core::notes_in`].
    /// - [`Error::Index`] as in [`Core::notes`].
    pub fn folder_delete_count(&self, folder: impl AsRef<Path>) -> Result<DeleteCount> {
        let folder = folder.as_ref();
        Store::check_folder(folder)?;
        self.removal(folder).map(|(count, _)| count)
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
    pub fn delete_note(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        Store::check_note(path)?;
        self.delete(path, |store| store.delete_note(path))
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
    pub fn delete_folder(&self, folder: impl AsRef<Path>) -> Result<()> {
        let folder = folder.as_ref();
        Store::check_folder(folder)?;
        self.delete(folder, |store| store.delete_folder(folder))
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
    pub fn link_id(&self, path: impl AsRef<Path>) -> Result<NoteId> {
        let note = self.edit_fields(path.as_ref(), |text| {
            if text.id().is_none() {
                text.set_id(Some(NoteId::generate().as_str()));
            }
        })?;
        note.info.id.ok_or_else(|| Error::Write {
            path: self.inner.store.root().join(&note.info.path),
            source: std::io::ErrorKind::InvalidData.into(),
        })
    }

    /// Sets the tags of the note and returns it. Its body and modification time stay.
    ///
    /// Each tag is cleaned like a typed tag, and repeats go. With no tags, the `tags:` field goes.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] as in [`Core::read`].
    /// - [`Error::Read`] or [`Error::NotUtf8`] when the file can't be read as text.
    /// - [`Error::Write`] when the file can't be written. The old file then stays.
    /// - [`Error::Index`] as in [`Core::save`].
    pub fn set_tags<S: AsRef<str>>(&self, path: impl AsRef<Path>, tags: &[S]) -> Result<NoteInfo> {
        let tags = retag::unique(
            tags.iter()
                .filter_map(|tag| holm_markdown::normalize_tag(tag.as_ref())),
        );
        self.edit_fields(path.as_ref(), |text| {
            if text.tags() != tags {
                text.set_tags(&tags);
            }
        })
        .map(|note| note.info)
    }

    /// Pins or unpins the note and returns it. Its body and modification time stay.
    ///
    /// # Errors
    ///
    /// As in [`Core::set_tags`].
    pub fn set_pinned(&self, path: impl AsRef<Path>, pinned: bool) -> Result<NoteInfo> {
        self.edit_fields(path.as_ref(), |text| {
            if text.pinned() != pinned {
                text.set_pinned(pinned);
            }
        })
        .map(|note| note.info)
    }

    /// How many notes [`Core::rename_tag`] changes.
    ///
    /// # Errors
    ///
    /// - [`Error::BadTag`] as in [`Core::rename_tag`].
    /// - [`Error::Index`] as in [`Core::notes`].
    pub fn tag_rename_count(&self, from: &str, to: &str) -> Result<usize> {
        self.tag_edit_count(&TagEdit::rename(from, to)?)
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
    /// - The first error of a note, as in [`Core::set_tags`].
    pub fn rename_tag(&self, from: &str, to: &str) -> Result<usize> {
        self.edit_tag(&TagEdit::rename(from, to)?)
    }

    /// How many notes [`Core::delete_tag`] changes.
    ///
    /// # Errors
    ///
    /// - [`Error::BadTag`] when `tag` is not a tag name.
    /// - [`Error::Index`] as in [`Core::notes`].
    pub fn tag_delete_count(&self, tag: &str) -> Result<usize> {
        self.tag_edit_count(&TagEdit::delete(tag)?)
    }

    /// Removes the tag and its child tags from every note, and returns how many notes changed.
    ///
    /// It works as [`Core::rename_tag`].
    ///
    /// # Errors
    ///
    /// - [`Error::BadTag`] when `tag` is not a tag name.
    /// - The first error of a note, as in [`Core::set_tags`].
    pub fn delete_tag(&self, tag: &str) -> Result<usize> {
        self.edit_tag(&TagEdit::delete(tag)?)
    }

    /// Copies the file into `attachments/` at the root of the notes folder,
    /// and returns its path from the folder of `note`, for a link.
    ///
    /// The copy keeps the file name. When it is taken, it gets a number: `photo 2.png`.
    /// A leading `.` becomes `-`. Holm makes `attachments/` when it is missing.
    /// The note itself does not change.
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
            holm_markdown::file_stem(&file.file_stem().unwrap_or_default().to_string_lossy());
        let ext = file
            .extension()
            .map(|ext| format!(".{}", ext.to_string_lossy()))
            .unwrap_or_default();
        self.attach(note.as_ref(), &OsString::from(stem), &ext, source)
    }

    /// Saves a pasted PNG image into `attachments/`, and returns its path from the folder
    /// of `note`, for a link.
    ///
    /// The name holds the local time: `Pasted image 2026-11-08 15-30-12.png`.
    /// Else it works as [`Core::attach_file`].
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] as in [`Core::attach_file`].
    /// - [`Error::Write`] when the image can't be written.
    pub fn paste_image(&self, note: impl AsRef<Path>, png: &[u8]) -> Result<PathBuf> {
        let stem = jiff::Zoned::now()
            .strftime("Pasted image %Y-%m-%d %H-%M-%S")
            .to_string();
        self.attach(note.as_ref(), &OsString::from(stem), ".png", png)
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

    /// Changes the frontmatter of the note with `edit`, keeps its modification time,
    /// and updates the index.
    fn edit_fields(
        &self,
        path: &Path,
        edit: impl FnOnce(&mut holm_markdown::Note),
    ) -> Result<Note> {
        let _writing = lock(&self.inner.writing);
        let (note, stamp) = self.inner.store.edit_fields(path, edit)?;
        self.inner.index.update(&self.inner.store, &note, stamp)
    }

    fn tag_edit_count(&self, edit: &TagEdit<'_>) -> Result<usize> {
        let notes = self.list(&Filter::Tag(edit.tag()), Order::Title)?;
        Ok(notes
            .iter()
            .filter(|note| edit.apply(&note.tags) != note.tags)
            .count())
    }

    fn edit_tag(&self, edit: &TagEdit<'_>) -> Result<usize> {
        let mut changed = 0;
        let mut first_error = None;
        for info in self.list(&Filter::Tag(edit.tag()), Order::Title)? {
            let mut hit = false;
            let result = self.edit_fields(&info.path, |text| {
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

    fn move_folder_to(&self, folder: &Path, parent: &Path, name: &str) -> Result<PathBuf> {
        let _writing = lock(&self.inner.writing);
        let new = self.inner.store.move_folder(folder, parent, name)?;
        if new != folder {
            self.refresh(vec![folder.to_owned(), new.clone()])?;
        }
        Ok(new)
    }

    /// The count of a delete of `scope`, and the attachment files it removes.
    fn removal(&self, scope: &Path) -> Result<(DeleteCount, Vec<PathBuf>)> {
        let removal = self.inner.index.removal(&self.inner.store, scope)?;
        let files: Vec<PathBuf> = removal
            .files
            .into_iter()
            .filter(|file| self.inner.store.is_attachment(file))
            .collect();
        let count = DeleteCount {
            notes: removal.notes,
            attachments: files.len(),
        };
        Ok((count, files))
    }

    /// Runs `delete` on `scope`, then deletes its attachment files and updates the index.
    fn delete(&self, scope: &Path, delete: impl FnOnce(&Store) -> Result<()>) -> Result<()> {
        let _writing = lock(&self.inner.writing);
        let (_, files) = self.removal(scope)?;
        delete(&self.inner.store)?;
        for file in files {
            if let Err(error) = self.inner.store.delete_attachment(&file) {
                tracing::warn!(%error, "kept an attachment");
            }
        }
        self.refresh(vec![scope.to_owned()])
    }

    /// Updates the index after a note moved from `from` to `to`, and returns the note.
    fn settle(&self, from: &Path, to: &Path) -> Result<NoteInfo> {
        if from != to {
            self.refresh(vec![from.to_owned(), to.to_owned()])?;
        }
        self.inner.store.read(to).map(|(note, _)| note.info)
    }

    /// Updates the index at each path after Holm's own change. No change goes to [`Changes`].
    fn refresh(&self, mut paths: Vec<PathBuf>) -> Result<()> {
        paths.sort();
        self.inner
            .index
            .refresh(&self.inner.store, &outer(paths))
            .map(drop)
    }

    fn list(&self, filter: &Filter<'_>, order: Order) -> Result<Vec<NoteInfo>> {
        self.inner.index.notes(&self.inner.store, filter, order)
    }

    fn build(store: Store, index_dir: &Path, watcher: Option<watch::Watcher>) -> Result<Self> {
        let index = Index::open(index_dir, &store)?;
        let inner = Inner {
            store,
            index,
            writing: Mutex::new(()),
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

impl Inner {
    /// Checks the disk at each path of `batch`, updates the index and sends the changes.
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
        let _writing = lock(&self.writing);
        let refresh = self.index.refresh(&self.store, &scopes)?;
        // A missing path may have been a folder. The index knows only notes, so it can't tell.
        // A missing `.md` path is taken as a note.
        let folders = scopes.iter().any(|scope| match self.store.entry(scope) {
            // The folder list never holds `attachments/`.
            _ if scope == Path::new(ATTACHMENTS) => false,
            Entry::Folder => true,
            Entry::Missing => Store::check_note(scope).is_err(),
            Entry::Note(_) | Entry::Other => false,
        });
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
