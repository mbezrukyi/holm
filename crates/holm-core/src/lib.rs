//! The core of Holm: notes, files, links, the index and file watching.
//!
//! It has no UI. The app and the later manager use it through one [`Core`] handle.

mod diff;
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
mod tag;
mod watch;
mod write;

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};

use futures_channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use futures_core::Stream;

pub use error::{Error, NameProblem, Result};
pub use paths::{FolderPath, NotePath};
pub use snapshot::Snapshot;
pub use tag::TagName;
pub use watch::Feed;

use facts::Facts;
use index::{Index, Row, Rows};
use model::{
    ChangeList, DeleteCount, LinkTarget, Note, NoteId, NoteInfo, Order, SearchResults, Source,
};
use nodes::{Node, Nodes};
use paths::Name;
use reconcile::Settled;
use retag::TagEdit;
use store::Store;
use watch::{Apply, Batch};
use write::Write;

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
    /// Each write and each watch batch holds it to the end, so no reader sees half of one.
    locked: Mutex<Locked>,
    listeners: Mutex<Vec<UnboundedSender<ChangeList>>>,
    _watcher: Option<watch::Watcher>,
}

/// What the lock of [`Inner`] holds.
#[derive(Debug)]
struct Locked {
    disk: Disk,
    facts: Facts,
}

/// The note files and the index rows.
#[derive(Debug)]
struct Disk {
    store: Store,
    index: Index,
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
    /// revision, with the snapshot of that revision. When a list comes, [`Core::snapshot`]
    /// shows its revision or a newer one.
    /// Each list gives the moves of a write first, then the note changes in path order, then
    /// [`Change::Folders`](model::Change::Folders) and [`Change::Tags`](model::Change::Tags).
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
        self.inner.with(|locked| {
            let paths = locked.disk.read_index(|index| index.search(text))?;
            let snapshot = locked.facts.snapshot().clone();
            Ok(SearchResults::new(snapshot, paths, order))
        })
    }

    /// The text of each heading in the note at `path`, in order.
    ///
    /// # Errors
    ///
    /// As in [`Core::read`].
    pub fn headings(&self, path: &NotePath) -> Result<Vec<String>> {
        self.read(path).map(|note| relink::headings(&note))
    }

    /// The note links in the note at `path` that point to a missing note or heading,
    /// each once. A heading link needs a heading with the same text, also in capital letters.
    ///
    /// # Errors
    ///
    /// As in [`Core::read`], when a linked note can't be read.
    pub fn broken_links(&self, path: &NotePath) -> Result<Vec<LinkTarget>> {
        self.inner.with(|locked| {
            relink::broken(locked.facts.snapshot(), path, |note| {
                locked.disk.store.read(note).map(|(note, _)| note)
            })
        })
    }

    /// The path for a link from the note at `note` to `file`, with no copy. A file inside the
    /// notes folder gets a path from the folder of the note. A file outside gets its absolute
    /// path.
    ///
    /// # Errors
    ///
    /// [`Error::BadPath`] when `file` is not absolute.
    pub fn file_link(&self, note: &NotePath, file: impl AsRef<Path>) -> Result<PathBuf> {
        self.inner
            .with(|locked| locked.disk.store.file_link(note, file.as_ref()))
    }

    /// Reads one note.
    ///
    /// # Errors
    ///
    /// [`Error::Read`] or [`Error::NotUtf8`] when the file can't be read as text.
    pub fn read(&self, path: &NotePath) -> Result<Note> {
        self.inner
            .with(|locked| locked.disk.store.read(path))
            .map(|(note, _)| note)
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
        self.inner.write(|write| write.save(path, body))
    }

    /// Writes `note` again at its path after another tool deleted its file, and returns it.
    /// The note keeps its id, so links to it still work. It also keeps its tags and its pin.
    /// When a file is at the path again, it saves the body over it like [`Core::save`].
    ///
    /// # Errors
    ///
    /// [`Error::Write`] when the file can't be written, for example when the folder is missing.
    pub fn restore(&self, note: &Note) -> Result<Note> {
        self.inner.write(|write| write.restore(note))
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
        self.inner
            .write(|write| write.create(&folder.note(&Name::title(title)?)))
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
        self.inner
            .write(|write| write.move_note(path, path.folder().note(&Name::title(title)?)))
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
        self.inner
            .write(|write| write.move_note(path, path.in_folder(folder)))
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
        self.inner
            .write(|write| write.create_folder(parent, &Name::typed(name)?))
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
        self.inner
            .write(|write| write.rename_folder(folder, &Name::typed(name)?))
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
        self.inner.write(|write| write.move_folder(folder, parent))
    }

    /// What [`Core::delete_note`] removes: the note and its attachment files.
    pub fn note_delete_count(&self, path: &NotePath) -> DeleteCount {
        self.inner.with(|locked| {
            locked
                .disk
                .removal(locked.facts.snapshot(), &Node::Note(path.clone()))
                .0
        })
    }

    /// What [`Core::delete_folder`] removes: the notes in the folder and below it,
    /// and their attachment files.
    pub fn folder_delete_count(&self, folder: &FolderPath) -> DeleteCount {
        self.inner.with(|locked| {
            locked
                .disk
                .removal(locked.facts.snapshot(), &Node::Folder(folder.clone()))
                .0
        })
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
        self.inner.write(|write| write.delete_note(path))
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
        self.inner.write(|write| write.delete_folder(folder))
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
        self.inner.write(|write| write.link_id(path))
    }

    /// Adds the tag to the note and returns it. Its body, other tags and modification time stay.
    ///
    /// A note that has it already does not change.
    ///
    /// # Errors
    ///
    /// - [`Error::Read`] or [`Error::NotUtf8`] when the file can't be read as text.
    /// - [`Error::Write`] when the file can't be written. The old file then stays.
    pub fn add_tag(&self, path: &NotePath, tag: &TagName) -> Result<NoteInfo> {
        self.inner.write(|write| write.add_tag(path, tag))
    }

    /// Removes the tag from the note and returns it. Its child tags, its body, other tags and
    /// modification time stay. A note without the tag does not change.
    ///
    /// # Errors
    ///
    /// The errors of [`Core::add_tag`] for the file.
    pub fn remove_tag(&self, path: &NotePath, tag: &TagName) -> Result<NoteInfo> {
        self.inner.write(|write| write.remove_tag(path, tag))
    }

    /// Pins or unpins the note and returns it. Its body and modification time stay.
    ///
    /// # Errors
    ///
    /// As in [`Core::add_tag`].
    pub fn set_pinned(&self, path: &NotePath, pinned: bool) -> Result<NoteInfo> {
        self.inner.write(|write| write.set_pinned(path, pinned))
    }

    /// How many notes [`Core::rename_tag`] changes.
    pub fn tag_rename_count(&self, from: &TagName, to: &TagName) -> usize {
        self.tag_edit_count(&TagEdit::Rename { from, to })
    }

    /// Renames the tag `from` to `to` in every note, and returns how many notes changed.
    ///
    /// Child tags follow: `work` to `job` makes `work/holm` into `job/holm`.
    /// When a note already has the new name, the two tags merge.
    /// The notes keep their modification times.
    ///
    /// A note that can't be changed keeps its tags, with a log line. The other notes still change.
    /// Each note changes in one step, but the whole job does not. After a crash, some notes can
    /// have the new tag and some the old one. A second rename then finishes the job.
    ///
    /// # Errors
    ///
    /// The first error of a note, as in [`Core::add_tag`].
    pub fn rename_tag(&self, from: &TagName, to: &TagName) -> Result<usize> {
        self.inner
            .write(|write| write.edit_tag(&TagEdit::Rename { from, to }))
    }

    /// How many notes [`Core::delete_tag`] changes.
    pub fn tag_delete_count(&self, tag: &TagName) -> usize {
        self.tag_edit_count(&TagEdit::Delete(tag))
    }

    /// Removes the tag and its child tags from every note, and returns how many notes changed.
    ///
    /// It works as [`Core::rename_tag`], also after a crash.
    ///
    /// # Errors
    ///
    /// The first error of a note, as in [`Core::add_tag`].
    pub fn delete_tag(&self, tag: &TagName) -> Result<usize> {
        self.inner
            .write(|write| write.edit_tag(&TagEdit::Delete(tag)))
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
        self.inner
            .with(|locked| locked.disk.store.attach_file(note, file.as_ref()))
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
        self.inner
            .with(|locked| locked.disk.store.paste_image(note, extension, bytes))
    }

    fn tag_edit_count(&self, edit: &TagEdit<'_>) -> usize {
        self.inner
            .with(|locked| edit.count(locked.facts.snapshot()))
    }

    fn build(store: Store, index_dir: &Path, watcher: Option<watch::Watcher>) -> Result<Self> {
        let index = Index::open(index_dir)?;
        let mut disk = Disk { store, index };
        let facts = disk.start()?;
        let inner = Inner {
            locked: Mutex::new(Locked { disk, facts }),
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

impl Disk {
    /// Builds the facts from the index rows and the files, and brings the rows up to date.
    /// When the rows can't be read, it starts from no facts and rebuilds the index.
    fn start(&mut self) -> Result<Facts> {
        let (entries, readable) = match self.index.entries() {
            Ok(entries) => (entries, true),
            Err(error) => {
                tracing::warn!(%error, "cannot read the index, so Holm rebuilds it");
                (Vec::new(), false)
            }
        };
        let mut facts = Facts::new(entries, self.store.folders(&FolderPath::root())?);
        let settled = scan::files(&self.store, &facts, &Nodes::root())?;
        if readable {
            self.write_rows(&settled);
        } else {
            self.rebuild_from(&settled);
        }
        facts.settle(settled);
        Ok(facts)
    }

    /// Writes the rows of `settled` into the index. When SQLite fails, it rebuilds the index,
    /// so it never fails.
    fn write_rows(&mut self, settled: &Settled) {
        let rows = Rows {
            removed: &settled.removed,
            moved: settled
                .moved
                .iter()
                .map(|moved| (&moved.from, &moved.info.path))
                .collect(),
            put: put_rows(settled),
        };
        if let Err(error) = self.index.write(&rows) {
            tracing::warn!(%error, "the index failed, so Holm rebuilds it");
            self.rebuild();
        }
    }

    /// Runs `op` on the index. When SQLite fails, it rebuilds the index and runs `op` again.
    fn read_index<T>(&mut self, op: impl Fn(&mut Index) -> Result<T>) -> Result<T> {
        op(&mut self.index).or_else(|error| {
            tracing::warn!(%error, "the index failed, so Holm rebuilds it");
            self.rebuild();
            op(&mut self.index)
        })
    }

    /// Rebuilds the index from every note file, read with no facts. A failure leaves the
    /// index as it is, with a log line.
    fn rebuild(&mut self) {
        match scan::files(&self.store, &Facts::default(), &Nodes::root()) {
            Ok(settled) => self.rebuild_from(&settled),
            Err(error) => tracing::warn!(%error, "cannot rebuild the index"),
        }
    }

    /// Rebuilds the index with the rows of `settled`, which holds every note file.
    fn rebuild_from(&mut self, settled: &Settled) {
        if let Err(error) = self.index.rebuild(&put_rows(settled)) {
            tracing::warn!(%error, "cannot rebuild the index");
        }
    }

    /// The count of a delete of `node` in `snapshot`, and the attachment files it removes.
    fn removal(&self, snapshot: &Snapshot, node: &Node) -> (DeleteCount, Vec<PathBuf>) {
        let removal = snapshot.removal(node);
        let files = self.store.attachments(removal.files);
        let count = DeleteCount {
            notes: removal.notes,
            attachments: files.len(),
        };
        (count, files)
    }
}

/// The index rows of each put note of `settled`.
fn put_rows(settled: &Settled) -> Vec<Row<'_>> {
    settled
        .put
        .iter()
        .map(|put| Row {
            note: &put.note,
            links: &put.links,
            stamp: put.stamp,
            source: put.source,
        })
        .collect()
}

impl Inner {
    /// Runs `op` under the lock.
    fn with<T>(&self, op: impl FnOnce(&mut Locked) -> T) -> T {
        op(&mut lock(&self.locked))
    }

    fn snapshot(&self) -> Snapshot {
        self.with(|locked| locked.facts.snapshot().clone())
    }

    /// Runs `op` under the lock, with a copy of the facts as the next facts.
    /// The next facts replace the facts, so they keep each new stamp. When a fact changed, they
    /// get the next revision. The change list carries the snapshot of its revision.
    fn write<T>(&self, op: impl FnOnce(&mut Write<'_>) -> Result<T>) -> Result<T> {
        self.run(Source::Holm, op)
    }

    /// Runs `op` as [`Inner::write`] does, for changes from `source`.
    fn run<T>(&self, source: Source, op: impl FnOnce(&mut Write<'_>) -> Result<T>) -> Result<T> {
        self.with(|Locked { disk, facts }| {
            let (result, list) = write::run(disk, facts, source, op);
            if let Some(list) = list {
                send(&mut lock(&self.listeners), list);
            }
            result
        })
    }

    /// Checks the disk at each path of `batch`, and updates the index, the snapshot and the
    /// change list.
    fn apply(&self, batch: Batch) -> Result<()> {
        self.run(Source::Outside, |write| write.outside(batch))
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

/// Sends `list` to each listener, and drops the listeners whose stream is gone. The last
/// listener gets `list` itself, so a long list is cloned only for the others.
fn send(listeners: &mut Vec<UnboundedSender<ChangeList>>, list: ChangeList) {
    listeners.retain(|listener| !listener.is_closed());
    if let Some((last, others)) = listeners.split_last() {
        for listener in others {
            // A stream that ends now is dropped at the next list.
            let _ = listener.unbounded_send(list.clone());
        }
        let _ = last.unbounded_send(list);
    }
}

/// A lock that still works after a panic in another thread. The data it guards stays valid.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
