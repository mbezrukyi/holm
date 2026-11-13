//! The app's only door to the notes folder.
//!
//! [`NotesFolder`] holds the latest [`Snapshot`] and runs every read and write of the core.
//! Writes, and reads that must see them, wait in one line and run one at a time, in the order
//! the UI asked for them. A write ends with the snapshot after it: the entity sets it and tells
//! the views, and only then does the task of the write give its result. A failed write does
//! not stop the line.
//!
//! Every 100 ms, it looks for changes from outside Holm. At quit, it runs every write that
//! waits in the line.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_channel::oneshot;
use gpui_kit::{App, AppContext as _, Context, EventEmitter, Task};
use holm_core::model::{Change, DeleteCount, LinkTarget, Note, NoteInfo, Order};
use holm_core::{Changes, Core, Snapshot};
use holm_editor::PastedFile;

use crate::error::AppError;

/// How often it looks for changes from outside Holm. The core already waits up to 500 ms to
/// batch them.
pub(crate) const POLL: Duration = Duration::from_millis(100);

/// What the notes folder tells the views.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FolderEvent {
    /// [`NotesFolder::snapshot`] has a new snapshot.
    Changed,
    /// A change from outside Holm. The snapshot already shows it.
    Outside(Change),
}

/// The editor text of a note, for a write.
#[derive(Clone, Debug)]
pub(crate) enum NoteText {
    /// The new body of the note at `path`. The note keeps its other facts.
    Save { path: PathBuf, body: String },
    /// The note after another tool deleted its file: its last facts and its body.
    Restore { info: NoteInfo, body: String },
}

/// One entry of the line. It runs off the UI thread.
type Job = Box<dyn FnOnce(&Core) -> Done + Send>;

/// The end of a job.
struct Done {
    /// The error of a failed job, for the log at quit.
    failure: Option<String>,
    /// Gives the result to the task of the write.
    deliver: Box<dyn FnOnce() + Send>,
}

pub(crate) struct NotesFolder {
    core: Core,
    snapshot: Snapshot,
    line: VecDeque<Job>,
    /// The job that left the line. Its thread holds the lock while it runs, so the quit drain
    /// waits for it, or runs it when the thread did not start yet.
    running: Arc<Mutex<Option<Job>>>,
    turn: Option<Task<()>>,
    /// After the quit drain, no job runs.
    closed: bool,
    _poll: Task<()>,
}

impl NotesFolder {
    pub(crate) fn new(core: Core, cx: &mut Context<Self>) -> Self {
        let changes = core.changes();
        let poll = cx.spawn(async move |folder, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                if folder
                    .update(cx, |folder, cx| folder.poll(&changes, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        cx.on_release(|folder, _| folder.drain(None)).detach();
        Self {
            snapshot: core.snapshot(),
            core,
            line: VecDeque::new(),
            running: Arc::default(),
            turn: None,
            closed: false,
            _poll: poll,
        }
    }

    /// The facts of the notes folder. It is never older than a write that ended.
    pub(crate) fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// Writes the editor text to its note.
    pub(crate) fn save_text(
        &mut self,
        text: NoteText,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<()>> {
        self.write(move |core| write_text(core, text), cx)
    }

    /// At quit: runs every write in the line, then writes `last`. Later writes do not run.
    /// It works on the UI thread, because no task runs after the window is gone.
    pub(crate) fn quit(&mut self, last: Option<NoteText>) {
        let last: Option<Job> = last.map(|text| {
            Box::new(move |core: &Core| {
                let failure = write_text(core, text)
                    .err()
                    .map(|error| crate::chain(&error));
                Done {
                    failure,
                    deliver: Box::new(|| {}),
                }
            }) as Job
        });
        self.drain(last);
    }

    /// Reads the note at `path`, after the writes before it.
    pub(crate) fn read(
        &mut self,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<Note>> {
        self.write(move |core| core.read(&path), cx)
    }

    /// Renames the note file to match its title.
    pub(crate) fn rename_to_title(
        &mut self,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<NoteInfo>> {
        self.write(move |core| core.rename_to_title(&path).result, cx)
    }

    /// Sets the tags of the note at `path`.
    pub(crate) fn set_tags(
        &mut self,
        path: PathBuf,
        tags: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<()>> {
        self.write(move |core| core.set_tags(&path, &tags).result.map(drop), cx)
    }

    /// Pins or unpins the note at `path`.
    pub(crate) fn set_pinned(
        &mut self,
        path: PathBuf,
        pinned: bool,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<()>> {
        self.write(
            move |core| core.set_pinned(&path, pinned).result.map(drop),
            cx,
        )
    }

    /// Creates a note in `folder`, with `tag` when it has one, and gives its path.
    pub(crate) fn create_note(
        &mut self,
        folder: PathBuf,
        tag: Option<String>,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<PathBuf>> {
        self.write(
            move |core| {
                let note = core.create(&folder).result?;
                match tag {
                    Some(tag) => core
                        .set_tags(&note.info.path, &[tag])
                        .result
                        .map(|info| info.path),
                    None => Ok(note.info.path),
                }
            },
            cx,
        )
    }

    /// Creates a note in `folder` with `title` as its heading, and gives its id for a link.
    pub(crate) fn create_titled_note(
        &mut self,
        folder: PathBuf,
        title: String,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<String>> {
        self.write(
            move |core| {
                let mut note = core.create(&folder).result?;
                note.body = format!("# {title}\n");
                let note = core.save(&note).result?;
                let info = core.rename_to_title(&note.info.path).result?;
                Ok(core.link_id(&info.path).result?.to_string())
            },
            cx,
        )
    }

    /// The id of the note at `path`, for a link. A note with no id gets one.
    pub(crate) fn link_id(
        &mut self,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<String>> {
        self.write(
            move |core| core.link_id(&path).result.map(|id| id.to_string()),
            cx,
        )
    }

    pub(crate) fn move_note(
        &mut self,
        path: PathBuf,
        folder: PathBuf,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<PathBuf>> {
        self.write(
            move |core| core.move_note(&path, &folder).result.map(|info| info.path),
            cx,
        )
    }

    pub(crate) fn move_folder(
        &mut self,
        path: PathBuf,
        folder: PathBuf,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<PathBuf>> {
        self.write(move |core| core.move_folder(&path, &folder).result, cx)
    }

    pub(crate) fn rename_folder(
        &mut self,
        folder: PathBuf,
        name: String,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<PathBuf>> {
        self.write(move |core| core.rename_folder(&folder, &name).result, cx)
    }

    pub(crate) fn create_folder(
        &mut self,
        parent: PathBuf,
        name: &'static str,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<PathBuf>> {
        self.write(move |core| core.create_folder(&parent, name).result, cx)
    }

    pub(crate) fn delete_note(
        &mut self,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<()>> {
        self.write(move |core| core.delete_note(&path).result, cx)
    }

    pub(crate) fn delete_folder(
        &mut self,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<()>> {
        self.write(move |core| core.delete_folder(&path).result, cx)
    }

    /// What a delete of the note at `path` removes, after the writes before it.
    pub(crate) fn note_delete_count(
        &mut self,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<DeleteCount>> {
        self.write(move |core| core.note_delete_count(&path), cx)
    }

    /// What a delete of the folder at `path` removes, after the writes before it.
    pub(crate) fn folder_delete_count(
        &mut self,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<DeleteCount>> {
        self.write(move |core| core.folder_delete_count(&path), cx)
    }

    /// Renames a tag in all notes, and gives the number of changed notes.
    pub(crate) fn rename_tag(
        &mut self,
        from: String,
        to: String,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<usize>> {
        self.write(move |core| core.rename_tag(&from, &to).result, cx)
    }

    /// The number of notes that a tag rename changes, after the writes before it.
    pub(crate) fn tag_rename_count(
        &mut self,
        from: String,
        to: String,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<usize>> {
        self.write(move |core| core.tag_rename_count(&from, &to), cx)
    }

    /// Removes a tag from all notes, and gives the number of changed notes.
    pub(crate) fn delete_tag(
        &mut self,
        tag: String,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<usize>> {
        self.write(move |core| core.delete_tag(&tag).result, cx)
    }

    /// The number of notes that a tag delete changes, after the writes before it.
    pub(crate) fn tag_delete_count(
        &mut self,
        tag: String,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<usize>> {
        self.write(move |core| core.tag_delete_count(&tag), cx)
    }

    /// Copies pasted or dropped files into `attachments/`, and gives the link to each copy
    /// from the note at `note`.
    pub(crate) fn attach(
        &mut self,
        note: PathBuf,
        files: Vec<PastedFile>,
        cx: &mut Context<Self>,
    ) -> Task<Vec<holm_core::Result<PathBuf>>> {
        let copies = self.write(
            move |core| {
                Ok(files
                    .iter()
                    .map(|file| match file {
                        PastedFile::Path(path) => core.attach_file(&note, path),
                        PastedFile::Image { extension, bytes } => {
                            core.paste_image(&note, extension, bytes)
                        }
                    })
                    .collect::<Vec<_>>())
            },
            cx,
        );
        cx.background_spawn(async move { copies.await.unwrap_or_default() })
    }

    /// The link to each file from the note at `note`, with no copy.
    pub(crate) fn file_links(
        &self,
        note: PathBuf,
        files: Vec<PathBuf>,
        cx: &App,
    ) -> Task<Vec<holm_core::Result<PathBuf>>> {
        self.look(
            move |core| {
                files
                    .iter()
                    .map(|file| core.file_link(&note, file))
                    .collect()
            },
            cx,
        )
    }

    /// The notes that match `query`. It does not wait for the line.
    pub(crate) fn search(
        &self,
        query: String,
        order: Order,
        cx: &App,
    ) -> Task<holm_core::Result<Vec<NoteInfo>>> {
        self.look(move |core| core.search(&query, order), cx)
    }

    /// The headings of the note at `path`, without its title. It does not wait for the line.
    pub(crate) fn headings(&self, path: PathBuf, cx: &App) -> Task<holm_core::Result<Vec<String>>> {
        self.look(move |core| core.headings(&path), cx)
    }

    /// The note links in the note at `path` whose note or heading is gone. It does not wait
    /// for the line.
    pub(crate) fn broken_links(
        &self,
        path: PathBuf,
        cx: &App,
    ) -> Task<holm_core::Result<Vec<LinkTarget>>> {
        self.look(move |core| core.broken_links(&path), cx)
    }

    /// Puts `job` in the line. The task gives its result after the entity has the snapshot
    /// after it. When the entity is gone first, the task never ends.
    fn write<T: Send + 'static>(
        &mut self,
        job: impl FnOnce(&Core) -> holm_core::Result<T> + Send + 'static,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<T>> {
        let (send, receive) = oneshot::channel();
        self.line.push_back(Box::new(move |core: &Core| {
            let result = job(core);
            let failure = result.as_ref().err().map(|error| crate::chain(error));
            Done {
                failure,
                deliver: Box::new(move || {
                    send.send(result).ok();
                }),
            }
        }));
        self.next(cx);
        cx.background_spawn(async move {
            match receive.await {
                Ok(result) => result,
                Err(oneshot::Canceled) => std::future::pending().await,
            }
        })
    }

    /// Runs a read that needs no order, off the UI thread.
    fn look<T: Send + 'static>(
        &self,
        job: impl FnOnce(&Core) -> T + Send + 'static,
        cx: &App,
    ) -> Task<T> {
        let core = self.core.clone();
        cx.background_spawn(async move { job(&core) })
    }

    /// Starts the first job of the line, when no job runs.
    fn next(&mut self, cx: &mut Context<Self>) {
        if self.turn.is_some() || self.closed {
            return;
        }
        let Some(job) = self.line.pop_front() else {
            return;
        };
        *lock(&self.running) = Some(job);
        let core = self.core.clone();
        let running = self.running.clone();
        let work = cx.background_spawn(async move {
            let mut slot = lock(&running);
            let job = slot.take()?;
            let done = job(&core);
            Some((core.snapshot(), done))
        });
        self.turn = Some(cx.spawn(async move |folder, cx| {
            let finished = work.await;
            folder
                .update(cx, |folder, cx| {
                    folder.turn = None;
                    if let Some((snapshot, done)) = finished {
                        folder.set(snapshot, cx);
                        (done.deliver)();
                    }
                    folder.next(cx);
                })
                .ok();
        }));
    }

    /// Runs every job that waits, and then `last`, on this thread.
    fn drain(&mut self, last: Option<Job>) {
        self.closed = true;
        let mut slot = lock(&self.running);
        let jobs = slot
            .take()
            .into_iter()
            .chain(self.line.drain(..))
            .chain(last);
        for job in jobs {
            if let Some(failure) = job(&self.core).failure {
                tracing::warn!(error = %failure, "a write at quit failed");
            }
        }
    }

    fn poll(&mut self, changes: &Changes, cx: &mut Context<Self>) {
        let snapshot = self.core.snapshot();
        self.set(snapshot, cx);
        while let Some(change) = changes.recv_timeout(Duration::ZERO) {
            cx.emit(FolderEvent::Outside(change));
        }
    }

    fn set(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        if snapshot.revision() > self.snapshot.revision() {
            self.snapshot = snapshot;
            cx.emit(FolderEvent::Changed);
        }
    }
}

impl EventEmitter<FolderEvent> for NotesFolder {}

/// Opens the core and gives the note to show first, if any. `create` makes the folder first.
pub(crate) fn open(
    open: &dyn Fn(&Path, &Path) -> holm_core::Result<Core>,
    folder: &Path,
    index_dir: &Path,
    create: bool,
    last: Option<&Path>,
) -> Result<(Core, Option<Note>), AppError> {
    if create && let Err(error) = std::fs::create_dir_all(folder) {
        tracing::warn!(path = %folder.display(), %error, "cannot make the notes folder");
    }
    let core = open(folder, index_dir)?;
    let note = first_note(&core, last)?;
    Ok((core, note))
}

/// The note to open first: the last open note when it still exists, else the last changed
/// note. With no notes, it is `None`.
fn first_note(core: &Core, last: Option<&Path>) -> holm_core::Result<Option<Note>> {
    if let Some(note) = last.and_then(|last| core.read(last).ok()) {
        return Ok(Some(note));
    }
    match core.snapshot().notes(Order::LastChange).first() {
        Some(info) => core.read(&info.path).map(Some),
        None => Ok(None),
    }
}

/// Saves the body of `text`. A save keeps the other facts that the core has for the note.
fn write_text(core: &Core, text: NoteText) -> holm_core::Result<()> {
    match text {
        NoteText::Save { path, body } => {
            let info = match core.snapshot().note(&path) {
                Some(info) => info.clone(),
                None => core.read(&path)?.info,
            };
            core.save(&Note { info, body }).result.map(drop)
        }
        NoteText::Restore { info, body } => core.restore(&Note { info, body }).result.map(drop),
    }
}

/// A lock that still works after a panic in another thread. A job leaves no half state.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use std::cell::RefCell;
    use std::fs;
    use std::rc::Rc;

    use gpui_kit::{Entity, TestAppContext};
    use tempfile::TempDir;

    use super::*;

    fn notes_folder(cx: &mut TestAppContext) -> (TempDir, Entity<NotesFolder>) {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes");
        fs::create_dir(&notes).unwrap();
        fs::write(notes.join("A.md"), "# A\n").unwrap();
        let (core, _) = Core::open_with_feed(&notes, dir.path().join("index")).unwrap();
        let notes_folder = cx.new(|cx| NotesFolder::new(core, cx));
        (dir, notes_folder)
    }

    #[gpui_kit::test]
    fn a_write_after_a_failed_write_still_runs(cx: &mut TestAppContext) {
        let (dir, notes_folder) = notes_folder(cx);
        let steps = Rc::new(RefCell::new(Vec::new()));
        let (failed, made) = notes_folder.update(cx, |notes_folder, cx| {
            (
                notes_folder.rename_folder("gone".into(), "x".into(), cx),
                notes_folder.create_folder(PathBuf::new(), "B", cx),
            )
        });
        cx.update(|cx| {
            for (name, write) in [("after the fail", failed), ("after the folder", made)] {
                let steps = steps.clone();
                cx.spawn(async move |_| {
                    if write.await.is_ok() {
                        steps.borrow_mut().push(name);
                    }
                })
                .detach();
            }
        });
        cx.run_until_parked();

        assert_eq!(*steps.borrow(), ["after the folder"]);
        assert!(dir.path().join("notes/B").is_dir());
        let folders: Vec<PathBuf> = notes_folder.read_with(cx, |notes_folder, _| {
            notes_folder
                .snapshot()
                .folders()
                .map(Path::to_owned)
                .collect()
        });
        assert_eq!(folders, [PathBuf::from("B")]);
    }

    #[gpui_kit::test]
    fn the_writes_in_the_line_reach_the_files_at_quit_with_the_text_last(cx: &mut TestAppContext) {
        let (dir, notes_folder) = notes_folder(cx);
        let path = PathBuf::from("A.md");
        let text = |body: &str| NoteText::Save {
            path: path.clone(),
            body: body.to_owned(),
        };
        notes_folder.update(cx, |notes_folder, cx| {
            notes_folder.save_text(text("# A\n\nfirst\n"), cx).detach();
            notes_folder
                .set_tags(path.clone(), vec!["work".into()], cx)
                .detach();
            notes_folder.create_folder(PathBuf::new(), "B", cx).detach();
            notes_folder.quit(Some(text("# A\n\nlast\n")));
        });

        let file = fs::read_to_string(dir.path().join("notes/A.md")).unwrap();
        assert!(file.contains("  - work\n"), "{file}");
        assert!(file.ends_with("# A\n\nlast\n"), "{file}");
        assert!(dir.path().join("notes/B").is_dir());
    }
}
