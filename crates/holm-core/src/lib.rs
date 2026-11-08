//! The core of Holm: notes, files, links, the index and file watching.
//!
//! It has no UI. The app and the later manager use it through one [`Core`] handle.

mod error;
pub mod model;
mod store;

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use error::{Error, Result};

use model::{Note, NoteInfo};
use store::Store;

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
    #[expect(dead_code, reason = "the index will live here")]
    index_dir: PathBuf,
}

impl Core {
    /// Opens the notes folder. `index_dir` is where Holm keeps the index it can rebuild.
    ///
    /// # Errors
    ///
    /// [`Error::NotesFolder`] when `notes_folder` is missing or is not a folder.
    pub fn open(notes_folder: impl AsRef<Path>, index_dir: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(Inner {
                store: Store::open(notes_folder.as_ref())?,
                index_dir: index_dir.into(),
            }),
        })
    }

    /// Every note in the notes folder and its subfolders, sorted by path.
    ///
    /// Only `*.md` files are notes. Hidden files and folders, and symlinks, are skipped.
    /// A note that can't be read is left out, with a log line.
    ///
    /// # Errors
    ///
    /// [`Error::Read`] when the notes folder itself can't be read.
    pub fn notes(&self) -> Result<Vec<NoteInfo>> {
        self.inner.store.notes()
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
        self.inner.store.read(path.as_ref())
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
    pub fn save(&self, note: &Note) -> Result<Note> {
        self.inner.store.save(note)
    }

    /// Creates an empty note with a new id in `folder`, and returns it.
    ///
    /// The file is `Untitled.md`, or `Untitled 2.md`, `Untitled 3.md` and so on when the name is taken.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] when `folder` is not a plain folder path inside the notes folder.
    /// - [`Error::Write`] when the file can't be written, for example when the folder is missing.
    pub fn create(&self, folder: impl AsRef<Path>) -> Result<Note> {
        self.inner.store.create(folder.as_ref())
    }
}
