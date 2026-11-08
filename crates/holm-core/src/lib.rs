//! The core of Holm: notes, files, links, the index and file watching.
//!
//! It has no UI. The app and the later manager use it through one [`Core`] handle.

mod error;
mod index;
pub mod model;
mod store;
mod watch;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

pub use error::{Error, Result};
pub use watch::Feed;

use index::{Filter, Index};
use model::{Change, Link, Note, NoteInfo, Order, Tag};
use store::{Entry, Store};
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
        let mut scopes: Vec<PathBuf> = Vec::new();
        // The batch is sorted, so a folder comes before the paths inside it.
        for path in batch {
            if Store::check_folder(&path).is_ok() && !scopes.iter().any(|s| path.starts_with(s)) {
                scopes.push(path);
            }
        }
        if scopes.is_empty() {
            return Ok(());
        }
        let _writing = lock(&self.writing);
        let refresh = self.index.refresh(&self.store, &scopes)?;
        // A missing path may have been a folder. The index knows only notes, so it can't tell.
        // A missing `.md` path is taken as a note.
        let folders = scopes.iter().any(|scope| match self.store.entry(scope) {
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

/// A lock that still works after a panic in another thread. The data it guards stays valid.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
