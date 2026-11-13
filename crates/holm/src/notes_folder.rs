//! The app's only door to the notes folder.
//!
//! [`NotesFolder`] holds the latest [`Snapshot`] and runs every read and write of the core.
//! Writes, and reads that must see them, wait in one line and run one at a time, in the order
//! the UI asked for them. A failed write does not stop the line.
//!
//! Each write ends in one update. The entity sets the snapshot after it and sends one
//! [`FolderEvent::Changed`] with the moves of the write. Then, in the same update, the caller
//! gets the result. So no frame shows a state between them.
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
use crate::tags::TagChange;

/// How often it looks for changes from outside Holm. The core already waits up to 500 ms to
/// batch them.
pub(crate) const POLL: Duration = Duration::from_millis(100);

/// What the notes folder tells the views.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FolderEvent {
    /// A write of Holm ended, or changes from outside came in. [`NotesFolder::snapshot`] has
    /// the snapshot after them, which can be the one before. It names the moves of the write.
    /// Each write sends one, also a failed write.
    Changed(Vec<Move>),
    /// A change from outside Holm. The snapshot already shows it.
    Outside(Change),
}

/// A move that a write made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Move {
    /// A note or a folder. A folder takes everything below it.
    Path { from: PathBuf, to: PathBuf },
    /// A tag. It takes its child tags.
    Tag { from: String, to: String },
}

impl Move {
    /// `path` after the move, when it is the moved path or below it.
    pub(crate) fn path(&self, path: &Path) -> Option<PathBuf> {
        let Self::Path { from, to } = self else {
            return None;
        };
        let rest = path.strip_prefix(from).ok()?;
        Some(if rest.as_os_str().is_empty() {
            to.clone()
        } else {
            to.join(rest)
        })
    }

    /// `tag` after the move, when it is the moved tag or one of its child tags.
    pub(crate) fn tag(&self, tag: &str) -> Option<String> {
        let Self::Tag { from, to } = self else {
            return None;
        };
        if tag == from {
            return Some(to.clone());
        }
        let rest = tag.strip_prefix(from.as_str())?.strip_prefix('/')?;
        Some(format!("{to}/{rest}"))
    }
}

/// A step that gets the end of a write, on the UI thread.
pub(crate) type Step<R> = Box<dyn FnOnce(R, &mut App)>;

/// The editor text of a note, for a write.
#[derive(Clone, Debug)]
pub(crate) enum NoteText {
    /// The new body of the note at `path`. The note keeps its other facts.
    Save { path: PathBuf, body: String },
    /// The note after another tool deleted its file: its last facts and its body.
    Restore { info: NoteInfo, body: String },
}

/// The work of one entry of the line. It runs off the UI thread, and gives the error of a
/// failed job for the log at quit.
type Job = Box<dyn FnOnce(&Core) -> Option<String> + Send>;

/// The end of one entry of the line, on the UI thread after its job: it sends the event and
/// gives the result to the caller.
type End = Box<dyn FnOnce(&mut Context<NotesFolder>)>;

pub(crate) struct NotesFolder {
    core: Core,
    snapshot: Snapshot,
    line: VecDeque<(Job, End)>,
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
        done: impl FnOnce(holm_core::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.write(move |core| write_text(core, text), done, cx);
    }

    /// At quit: runs every write in the line, then writes `last`. Later writes do not run.
    /// It works on the UI thread, because no task runs after the window is gone.
    pub(crate) fn quit(&mut self, last: Option<NoteText>) {
        let last: Option<Job> = last.map(|text| {
            Box::new(move |core: &Core| {
                write_text(core, text)
                    .err()
                    .map(|error| crate::chain(&error))
            }) as Job
        });
        self.drain(last);
    }

    /// Reads the note at `path`, after the writes before it.
    pub(crate) fn read(
        &mut self,
        path: PathBuf,
        done: impl FnOnce(holm_core::Result<Note>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.write(move |core| core.read(&path), done, cx);
    }

    /// Ends after the writes before it, so `done` sees their snapshot.
    pub(crate) fn wait(
        &mut self,
        done: impl FnOnce(holm_core::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.write(|_| Ok(()), done, cx);
    }

    /// Renames the note file to match its title.
    pub(crate) fn rename_to_title(
        &mut self,
        path: PathBuf,
        done: impl FnOnce(holm_core::Result<NoteInfo>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let from = path.clone();
        self.queue(
            move |core| core.rename_to_title(&path).result,
            move |info: &NoteInfo| moved(from, &info.path),
            done,
            cx,
        );
    }

    /// Adds or removes one tag of the note at `path`.
    pub(crate) fn change_tag(
        &mut self,
        path: PathBuf,
        change: TagChange,
        done: impl FnOnce(holm_core::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.write(
            move |core| {
                let written = match change {
                    TagChange::Add(tag) => core.add_tag(&path, &tag),
                    TagChange::Remove(tag) => core.remove_tag(&path, &tag),
                };
                written.result.map(drop)
            },
            done,
            cx,
        );
    }

    /// Pins or unpins the note at `path`.
    pub(crate) fn set_pinned(
        &mut self,
        path: PathBuf,
        pinned: bool,
        done: impl FnOnce(holm_core::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.write(
            move |core| core.set_pinned(&path, pinned).result.map(drop),
            done,
            cx,
        );
    }

    /// Creates a note in `folder`, with `tag` when it has one, and gives its path.
    pub(crate) fn create_note(
        &mut self,
        folder: PathBuf,
        tag: Option<String>,
        done: impl FnOnce(holm_core::Result<PathBuf>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.write(
            move |core| {
                let note = core.create(&folder).result?;
                match tag {
                    Some(tag) => core
                        .add_tag(&note.info.path, &tag)
                        .result
                        .map(|info| info.path),
                    None => Ok(note.info.path),
                }
            },
            done,
            cx,
        );
    }

    /// Creates a note in `folder` with `title` as its heading, and gives its id for a link.
    pub(crate) fn create_titled_note(
        &mut self,
        folder: PathBuf,
        title: String,
        done: impl FnOnce(holm_core::Result<String>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.write(
            move |core| {
                let note = core.create(&folder).result?;
                let note = core.save(&note.info.path, &format!("# {title}\n")).result?;
                let info = core.rename_to_title(&note.info.path).result?;
                Ok(core.link_id(&info.path).result?.to_string())
            },
            done,
            cx,
        );
    }

    /// The id of the note at `path`, for a link. A note with no id gets one.
    pub(crate) fn link_id(
        &mut self,
        path: PathBuf,
        done: impl FnOnce(holm_core::Result<String>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.write(
            move |core| core.link_id(&path).result.map(|id| id.to_string()),
            done,
            cx,
        );
    }

    pub(crate) fn move_note(
        &mut self,
        path: PathBuf,
        folder: PathBuf,
        done: impl FnOnce(holm_core::Result<PathBuf>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let from = path.clone();
        self.queue(
            move |core| core.move_note(&path, &folder).result.map(|info| info.path),
            move |to: &PathBuf| moved(from, to),
            done,
            cx,
        );
    }

    pub(crate) fn move_folder(
        &mut self,
        path: PathBuf,
        folder: PathBuf,
        done: impl FnOnce(holm_core::Result<PathBuf>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let from = path.clone();
        self.queue(
            move |core| core.move_folder(&path, &folder).result,
            move |to: &PathBuf| moved(from, to),
            done,
            cx,
        );
    }

    pub(crate) fn rename_folder(
        &mut self,
        folder: PathBuf,
        name: String,
        done: impl FnOnce(holm_core::Result<PathBuf>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let from = folder.clone();
        self.queue(
            move |core| core.rename_folder(&folder, &name).result,
            move |to: &PathBuf| moved(from, to),
            done,
            cx,
        );
    }

    pub(crate) fn create_folder(
        &mut self,
        parent: PathBuf,
        name: &'static str,
        done: impl FnOnce(holm_core::Result<PathBuf>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.write(
            move |core| core.create_folder(&parent, name).result,
            done,
            cx,
        );
    }

    pub(crate) fn delete_note(
        &mut self,
        path: PathBuf,
        done: impl FnOnce(holm_core::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.write(move |core| core.delete_note(&path).result, done, cx);
    }

    pub(crate) fn delete_folder(
        &mut self,
        path: PathBuf,
        done: impl FnOnce(holm_core::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.write(move |core| core.delete_folder(&path).result, done, cx);
    }

    /// What a delete of the note at `path` removes, after the writes before it.
    pub(crate) fn note_delete_count(
        &mut self,
        path: PathBuf,
        done: impl FnOnce(holm_core::Result<DeleteCount>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.write(move |core| core.note_delete_count(&path), done, cx);
    }

    /// What a delete of the folder at `path` removes, after the writes before it.
    pub(crate) fn folder_delete_count(
        &mut self,
        path: PathBuf,
        done: impl FnOnce(holm_core::Result<DeleteCount>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.write(move |core| core.folder_delete_count(&path), done, cx);
    }

    /// Renames a tag in all notes, and gives the number of changed notes.
    pub(crate) fn rename_tag(
        &mut self,
        from: String,
        to: String,
        done: impl FnOnce(holm_core::Result<usize>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let moves = holm_markdown::normalize_tag(&to)
            .map(|to| Move::Tag {
                from: from.clone(),
                to,
            })
            .into_iter()
            .collect();
        self.queue(
            move |core| core.rename_tag(&from, &to).result,
            move |_: &usize| moves,
            done,
            cx,
        );
    }

    /// The number of notes that a tag rename changes, after the writes before it.
    pub(crate) fn tag_rename_count(
        &mut self,
        from: String,
        to: String,
        done: impl FnOnce(holm_core::Result<usize>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.write(move |core| core.tag_rename_count(&from, &to), done, cx);
    }

    /// Removes a tag from all notes, and gives the number of changed notes.
    pub(crate) fn delete_tag(
        &mut self,
        tag: String,
        done: impl FnOnce(holm_core::Result<usize>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.write(move |core| core.delete_tag(&tag).result, done, cx);
    }

    /// The number of notes that a tag delete changes, after the writes before it.
    pub(crate) fn tag_delete_count(
        &mut self,
        tag: String,
        done: impl FnOnce(holm_core::Result<usize>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.write(move |core| core.tag_delete_count(&tag), done, cx);
    }

    /// Copies pasted or dropped files into `attachments/`, and gives the link to each copy
    /// from the note at `note`.
    pub(crate) fn attach(
        &mut self,
        note: PathBuf,
        files: Vec<PastedFile>,
        done: impl FnOnce(Vec<holm_core::Result<PathBuf>>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.write(
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
            |copies, cx| done(copies.unwrap_or_default(), cx),
            cx,
        );
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

    /// Like [`Self::queue`], for a job that moves nothing.
    fn write<T: Send + 'static>(
        &mut self,
        job: impl FnOnce(&Core) -> holm_core::Result<T> + Send + 'static,
        done: impl FnOnce(holm_core::Result<T>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.queue(job, |_: &T| Vec::new(), done, cx);
    }

    /// Puts `job` in the line. When it ends, the entity sets the snapshot after it and sends
    /// the event with `moves` of its value. Then `done` gets its result, in the same update.
    /// When the entity is gone first, `done` never runs.
    fn queue<T: Send + 'static>(
        &mut self,
        job: impl FnOnce(&Core) -> holm_core::Result<T> + Send + 'static,
        moves: impl FnOnce(&T) -> Vec<Move> + 'static,
        done: impl FnOnce(holm_core::Result<T>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let (send, mut receive) = oneshot::channel();
        let job: Job = Box::new(move |core: &Core| {
            let result = job(core);
            let failure = result.as_ref().err().map(|error| crate::chain(error));
            send.send(result).ok();
            failure
        });
        let end: End = Box::new(move |cx: &mut Context<Self>| {
            let Ok(Some(result)) = receive.try_recv() else {
                return;
            };
            let moves = result.as_ref().map(moves).unwrap_or_default();
            cx.emit(FolderEvent::Changed(moves));
            // A deferred step runs after the views took the event, in the same update.
            cx.defer(move |cx| done(result, cx));
        });
        self.line.push_back((job, end));
        self.next(cx);
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
        let Some((job, end)) = self.line.pop_front() else {
            return;
        };
        *lock(&self.running) = Some(job);
        let core = self.core.clone();
        let running = self.running.clone();
        let work = cx.background_spawn(async move {
            let mut slot = lock(&running);
            let job = slot.take()?;
            job(&core);
            Some(core.snapshot())
        });
        self.turn = Some(cx.spawn(async move |folder, cx| {
            let snapshot = work.await;
            folder
                .update(cx, |folder, cx| {
                    folder.turn = None;
                    if let Some(snapshot) = snapshot {
                        folder.set(snapshot);
                        end(cx);
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
            .chain(self.line.drain(..).map(|(job, _)| job))
            .chain(last);
        for job in jobs {
            if let Some(failure) = job(&self.core) {
                tracing::warn!(error = %failure, "a write at quit failed");
            }
        }
    }

    /// Takes the changes from outside Holm. While a job runs, it waits: the core can have the
    /// snapshot of the job before its event.
    fn poll(&mut self, changes: &Changes, cx: &mut Context<Self>) {
        if self.turn.is_some() {
            return;
        }
        if self.set(self.core.snapshot()) {
            cx.emit(FolderEvent::Changed(Vec::new()));
        }
        while let Some(change) = changes.recv_timeout(Duration::ZERO) {
            cx.emit(FolderEvent::Outside(change));
        }
    }

    /// Keeps `snapshot` when it is newer, and gives whether it did.
    fn set(&mut self, snapshot: Snapshot) -> bool {
        let newer = snapshot.revision() > self.snapshot.revision();
        if newer {
            self.snapshot = snapshot;
        }
        newer
    }
}

/// The move from `from` to `to`, when they differ.
fn moved(from: PathBuf, to: &Path) -> Vec<Move> {
    if from == to {
        return Vec::new();
    }
    vec![Move::Path {
        from,
        to: to.to_owned(),
    }]
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
        NoteText::Save { path, body } => core.save(&path, &body).result.map(drop),
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
        notes_folder.update(cx, |notes_folder, cx| {
            let step = |name: &'static str| {
                let steps = steps.clone();
                move |result: holm_core::Result<PathBuf>, _: &mut App| {
                    if result.is_ok() {
                        steps.borrow_mut().push(name);
                    }
                }
            };
            notes_folder.rename_folder("gone".into(), "x".into(), step("after the fail"), cx);
            notes_folder.create_folder(PathBuf::new(), "B", step("after the folder"), cx);
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
    fn each_write_sends_its_moves_and_then_gives_its_result(cx: &mut TestAppContext) {
        let (_dir, notes_folder) = notes_folder(cx);
        let steps = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let steps = steps.clone();
            cx.subscribe(&notes_folder, move |_, event, _| {
                if let FolderEvent::Changed(moves) = event {
                    steps.borrow_mut().push(format!("event {moves:?}"));
                }
            })
            .detach();
        });
        notes_folder.update(cx, |notes_folder, cx| {
            let step = |name: &'static str| {
                let steps = steps.clone();
                move |result: holm_core::Result<PathBuf>, _: &mut App| {
                    let end = if result.is_ok() { "ok" } else { "failed" };
                    steps.borrow_mut().push(format!("{name} {end}"));
                }
            };
            notes_folder.create_folder(PathBuf::new(), "work", step("create"), cx);
            notes_folder.rename_folder("work".into(), "job".into(), step("rename"), cx);
            notes_folder.rename_folder("gone".into(), "x".into(), step("fail"), cx);
        });
        cx.run_until_parked();

        assert_eq!(
            *steps.borrow(),
            [
                "event []",
                "create ok",
                r#"event [Path { from: "work", to: "job" }]"#,
                "rename ok",
                "event []",
                "fail failed",
            ]
        );
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
            notes_folder.save_text(text("# A\n\nfirst\n"), |_, _| {}, cx);
            notes_folder.change_tag(path.clone(), TagChange::Add("work".into()), |_, _| {}, cx);
            notes_folder.create_folder(PathBuf::new(), "B", |_, _| {}, cx);
            notes_folder.quit(Some(text("# A\n\nlast\n")));
        });

        let file = fs::read_to_string(dir.path().join("notes/A.md")).unwrap();
        assert!(file.contains("  - work\n"), "{file}");
        assert!(file.ends_with("# A\n\nlast\n"), "{file}");
        assert!(dir.path().join("notes/B").is_dir());
    }

    #[gpui_kit::test]
    fn a_save_keeps_a_tag_from_another_tool(cx: &mut TestAppContext) {
        let (dir, notes_folder) = notes_folder(cx);
        let file = dir.path().join("notes/A.md");
        fs::write(&file, "---\ntags: [theirs]\n---\n# A\n").unwrap();

        notes_folder.update(cx, |notes_folder, cx| {
            let text = NoteText::Save {
                path: "A.md".into(),
                body: "# A\n\nmine\n".into(),
            };
            notes_folder.save_text(text, |_, _| {}, cx);
        });
        cx.run_until_parked();

        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains("tags: [theirs]\n"), "{text}");
        assert!(text.ends_with("# A\n\nmine\n"), "{text}");
    }

    #[gpui_kit::test]
    fn a_tag_removal_keeps_a_tag_from_another_tool(cx: &mut TestAppContext) {
        let (dir, notes_folder) = notes_folder(cx);
        let file = dir.path().join("notes/A.md");
        let change = |change: TagChange, cx: &mut TestAppContext| {
            notes_folder.update(cx, |notes_folder, cx| {
                notes_folder.change_tag("A.md".into(), change, |_, _| {}, cx);
            });
            cx.run_until_parked();
        };

        change(TagChange::Add("a".into()), cx);
        fs::write(&file, "---\ntags: [a, b]\n---\n# A\n").unwrap();
        change(TagChange::Remove("a".into()), cx);

        let tags = holm_markdown::Note::parse(&fs::read_to_string(&file).unwrap()).tags();
        assert_eq!(tags, ["b"]);
    }
}
