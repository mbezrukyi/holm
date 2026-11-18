//! The app's only door to the notes folder.
//!
//! [`NotesFolder`] holds the latest [`Snapshot`] and runs every read and write of the core.
//! Writes, and reads that must see them, wait in one line and run one at a time, in the order
//! the UI asked for them. A failed write does not stop the line.
//!
//! Each write ends in one update. The entity sets the snapshot after it and sends one
//! [`FolderEvent`] with the changes up to that snapshot. Then, in the same update, the caller
//! gets the result. So no frame shows a state between them.
//!
//! A call waits on the UI thread for the jobs before it and its own, up to a budget. A job that
//! ends in time ends in the update of the call, so no frame shows a state between the key and
//! the end of its write. A late job ends in a later update, and [`NotesFolder::writing`] is true
//! until then.
//!
//! It holds one [`Draft`]: the unsaved editor text of the open note. Before each job, the line
//! writes the draft. So no job sees a note file older than the editor text. The draft keeps the
//! text until a write of it succeeds, and each write of it ends in one [`SaveEvent`].
//!
//! A change list from outside Holm wakes it when no job runs, and it sends one event. During a
//! job, the lists wait for the event of the job. When its window closes or the app quits, it runs
//! every write that waits in the line, and then writes the draft.
//!
//! Each job of the line, the search and the file links are work of the user. When one fails, it
//! sends one [`JobFailed`] with the answer for the message line. So a caller gets only the
//! value, unless it owns the error. The draft is Holm's own work, and its fail is a
//! [`SaveEvent`]. The other reads give their error, for the log.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{self, Poll};
use std::time::{Duration, Instant};

use futures_channel::oneshot;
use futures_core::Stream;
use gpui_kit::{App, AppContext as _, Context, EventEmitter, Subscription, Task, Window};
use holm_core::model::{
    Change, ChangeList, DeleteCount, LinkTarget, Note, NoteSet, Order, SearchResults, Source,
};
use holm_core::{ChangeLists, Core, FolderPath, NotePath, Snapshot, TagName};
use holm_editor::PastedFile;

use crate::error::AppError;
use crate::tags::TagChange;
use draft::{Draft, Written};

mod draft;

/// What the notes folder tells the views: a write of Holm ended, or changes from outside came
/// in. Each write sends one, also a failed write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FolderEvent {
    /// The changes of every revision up to [`NotesFolder::snapshot`], in order, each with
    /// its source. It is empty when the snapshot did not change.
    changes: Vec<(Source, Change)>,
}

impl FolderEvent {
    /// Every change, in order.
    pub(crate) fn changes(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().map(|(_, change)| change)
    }

    /// The changes that other tools made, in order.
    pub(crate) fn outside(&self) -> impl Iterator<Item = &Change> {
        self.changes
            .iter()
            .filter(|(source, _)| *source == Source::Outside)
            .map(|(_, change)| change)
    }

    /// Whether the snapshot stayed the same.
    pub(crate) fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// A step that gets the end of a write, on the UI thread.
pub(crate) type Step<R> = Box<dyn FnOnce(R, &mut App)>;

/// What the caller of a job gets at its end. A failure shows as an answer in both cases.
pub(crate) struct Done<T>(Step<Result<T, AppError>>);

impl<T: 'static> Done<T> {
    /// `step` gets the value. After a failure, it does not run.
    pub(crate) fn value(step: impl FnOnce(T, &mut App) + 'static) -> Self {
        Self(Box::new(move |result, cx| {
            if let Ok(value) = result {
                step(value, cx);
            }
        }))
    }

    /// `step` gets the result, for a caller that owns the error.
    pub(crate) fn result(step: impl FnOnce(Result<T, AppError>, &mut App) + 'static) -> Self {
        Self(Box::new(step))
    }
}

/// A job of the user failed. It holds the answer for the message line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct JobFailed(pub(crate) &'static str);

/// A step that runs `step` on the entity of `cx` with its window, while both still exist.
pub(crate) fn window_step<V: 'static, R: 'static>(
    window: &Window,
    cx: &Context<V>,
    step: impl FnOnce(&mut V, R, &mut Window, &mut Context<V>) + 'static,
) -> Step<R> {
    let entity = cx.weak_entity();
    let window = window.window_handle();
    Box::new(move |result, cx| {
        window
            .update(cx, |_, window, cx| {
                entity.update(cx, |entity, cx| step(entity, result, window, cx))
            })
            .ok();
    })
}

/// The end of a write of the draft. It comes after the [`FolderEvent`] of the write.
#[derive(Debug)]
pub(crate) enum SaveEvent {
    /// The text of the write is on disk.
    Saved,
    /// The write failed. The draft keeps the text, and the next job writes it again.
    Failed(AppError),
}

/// The editor text is not on disk, because the last write of the draft failed. The
/// [`SaveEvent`] of that write has the error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NotSaved;

/// The work of one entry of the line. It runs off the UI thread, and gives the error of a
/// failed job for the log at quit.
type Job = Box<dyn FnOnce(&Core) -> Option<String> + Send>;

/// The end of one entry of the line, on the UI thread after its job and its event: it gives
/// the result to the caller.
type End = Box<dyn FnOnce(&mut NotesFolder, &mut Context<NotesFolder>)>;

/// One entry of the line. Its number grows with each entry.
struct Entry {
    number: u64,
    job: Job,
    end: End,
}

/// The entry whose job runs.
struct Turn {
    number: u64,
    end: End,
    /// It comes when the job ends. It is gone when the quit drain ran the job.
    done: oneshot::Receiver<()>,
    /// It ends the turn when the job ends after its call.
    _wake: Task<()>,
}

pub(crate) struct NotesFolder {
    core: Core,
    root: PathBuf,
    snapshot: Snapshot,
    changes: ChangeLists,
    /// How long a call waits for its job.
    budget: Duration,
    line: VecDeque<Entry>,
    next_number: u64,
    /// The number of the last entry that ended.
    ended: u64,
    /// The last entry that is past its budget, until it ends.
    late: Option<u64>,
    /// The job that left the line. Its thread holds the lock while it runs, so the quit drain
    /// waits for it, or runs it when the thread did not start yet.
    running: Arc<Mutex<Option<Job>>>,
    turn: Option<Turn>,
    draft: Draft,
    /// After the quit drain, no job runs.
    closed: bool,
    watch: Task<()>,
    _ends: [Subscription; 2],
}

impl NotesFolder {
    /// The notes folder at `root`, which `core` opened. A call waits up to `budget` for its job.
    /// The line drains when `window` closes, when the app quits, or when the entity goes.
    pub(crate) fn new(
        core: Core,
        root: PathBuf,
        budget: Duration,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.on_release(|folder, _| folder.drain()).detach();
        // The platform window can keep views alive after a close, and with them this entity.
        let window = window.window_handle().window_id();
        let folder = cx.weak_entity();
        let ends = [
            cx.on_window_closed(move |cx, closed| {
                if closed == window {
                    folder.update(cx, |folder, _| folder.drain()).ok();
                }
            }),
            cx.on_app_quit(|folder, _| {
                folder.drain();
                std::future::ready(())
            }),
        ];
        let (snapshot, changes) = core.follow();
        Self {
            changes,
            snapshot,
            core,
            root,
            budget,
            line: VecDeque::new(),
            next_number: 1,
            ended: 0,
            late: None,
            running: Arc::default(),
            turn: None,
            draft: Draft::default(),
            closed: false,
            watch: Self::watch(cx),
            _ends: ends,
        }
    }

    /// The path of the notes folder, as the user chose it.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// The facts of the notes folder. It is never older than a write that ended.
    pub(crate) fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// Whether a job is past its budget.
    pub(crate) fn writing(&self) -> bool {
        self.late.is_some()
    }

    /// The editor text of the note at `path` is now `body`. It writes nothing yet.
    pub(crate) fn set_text(&mut self, path: NotePath, body: String) {
        self.draft.set(path, body);
    }

    /// Drops the editor text with no write.
    pub(crate) fn drop_text(&mut self) {
        self.draft.drop_text();
    }

    /// Whether the draft has text that is not on disk.
    pub(crate) fn unsaved(&self) -> bool {
        self.draft.unsaved()
    }

    /// Puts the draft in the line now. It does not wait for the job.
    pub(crate) fn save_text(&mut self, cx: &mut Context<Self>) {
        self.push_draft();
        self.run(None, cx);
    }

    /// Writes the draft, and ends after the writes before it. `done` gets [`NotSaved`] when
    /// the write of the draft failed.
    pub(crate) fn wait_saved(
        &mut self,
        done: impl FnOnce(Result<(), NotSaved>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.push_draft();
        let end: End = Box::new(move |folder, cx| {
            let saved = if folder.draft.failed() {
                Err(NotSaved)
            } else {
                Ok(())
            };
            cx.defer(move |cx| done(saved, cx));
        });
        let number = self.push_entry(Box::new(|_| None), end);
        self.run(Some(number), cx);
    }

    /// Reads the note at `path`, after the writes before it.
    pub(crate) fn read(&mut self, path: NotePath, done: Done<Note>, cx: &mut Context<Self>) {
        self.write(move |core| core.read(&path), done, cx);
    }

    /// Ends after the writes before it, so `done` sees their snapshot.
    pub(crate) fn wait(&mut self, done: Done<()>, cx: &mut Context<Self>) {
        self.write(|_| Ok(()), done, cx);
    }

    /// Gives the note at `path` a new title, and gives its new path.
    pub(crate) fn rename_note(
        &mut self,
        path: NotePath,
        title: String,
        done: Done<NotePath>,
        cx: &mut Context<Self>,
    ) {
        self.write(
            move |core| core.rename_note(&path, &title).map(|info| info.path),
            done,
            cx,
        );
    }

    /// Adds or removes one tag of the note at `path`.
    pub(crate) fn change_tag(
        &mut self,
        path: NotePath,
        change: TagChange,
        done: Done<()>,
        cx: &mut Context<Self>,
    ) {
        self.write(
            move |core| {
                match change {
                    TagChange::Add(tag) => core.add_tag(&path, &tag),
                    TagChange::Remove(tag) => core.remove_tag(&path, &tag),
                }
                .map(drop)
            },
            done,
            cx,
        );
    }

    /// Pins or unpins the note at `path`.
    pub(crate) fn set_pinned(
        &mut self,
        path: NotePath,
        pinned: bool,
        done: Done<()>,
        cx: &mut Context<Self>,
    ) {
        self.write(
            move |core| core.set_pinned(&path, pinned).map(drop),
            done,
            cx,
        );
    }

    /// Creates the note `title` in `folder`, with `tag` when it has one, and gives the note.
    pub(crate) fn create_note(
        &mut self,
        folder: FolderPath,
        title: String,
        tag: Option<TagName>,
        done: Done<Note>,
        cx: &mut Context<Self>,
    ) {
        self.write(
            move |core| {
                let note = core.create(&folder, &title)?;
                match tag {
                    Some(tag) => {
                        let info = core.add_tag(&note.info.path, &tag)?;
                        core.read(&info.path)
                    }
                    None => Ok(note),
                }
            },
            done,
            cx,
        );
    }

    /// Creates the note `title` in `folder`, and gives its id for a link.
    pub(crate) fn create_linked_note(
        &mut self,
        folder: FolderPath,
        title: String,
        done: Done<String>,
        cx: &mut Context<Self>,
    ) {
        self.write(
            move |core| {
                let note = core.create(&folder, &title)?;
                Ok(core.link_id(&note.info.path)?.to_string())
            },
            done,
            cx,
        );
    }

    /// The id of the note at `path`, for a link. A note with no id gets one.
    pub(crate) fn link_id(&mut self, path: NotePath, done: Done<String>, cx: &mut Context<Self>) {
        self.write(
            move |core| core.link_id(&path).map(|id| id.to_string()),
            done,
            cx,
        );
    }

    pub(crate) fn move_note(
        &mut self,
        path: NotePath,
        folder: FolderPath,
        done: Done<NotePath>,
        cx: &mut Context<Self>,
    ) {
        self.write(
            move |core| core.move_note(&path, &folder).map(|info| info.path),
            done,
            cx,
        );
    }

    pub(crate) fn move_folder(
        &mut self,
        path: FolderPath,
        folder: FolderPath,
        done: Done<FolderPath>,
        cx: &mut Context<Self>,
    ) {
        self.write(move |core| core.move_folder(&path, &folder), done, cx);
    }

    pub(crate) fn rename_folder(
        &mut self,
        folder: FolderPath,
        name: String,
        done: Done<FolderPath>,
        cx: &mut Context<Self>,
    ) {
        self.write(move |core| core.rename_folder(&folder, &name), done, cx);
    }

    pub(crate) fn create_folder(
        &mut self,
        parent: FolderPath,
        name: String,
        done: Done<FolderPath>,
        cx: &mut Context<Self>,
    ) {
        self.write(move |core| core.create_folder(&parent, &name), done, cx);
    }

    pub(crate) fn delete_note(&mut self, path: NotePath, done: Done<()>, cx: &mut Context<Self>) {
        self.write(move |core| core.delete_note(&path), done, cx);
    }

    pub(crate) fn delete_folder(
        &mut self,
        path: FolderPath,
        done: Done<()>,
        cx: &mut Context<Self>,
    ) {
        self.write(move |core| core.delete_folder(&path), done, cx);
    }

    /// What a delete of the note at `path` removes, after the writes before it.
    pub(crate) fn note_delete_count(
        &mut self,
        path: NotePath,
        done: Done<DeleteCount>,
        cx: &mut Context<Self>,
    ) {
        self.write(move |core| Ok(core.note_delete_count(&path)), done, cx);
    }

    /// What a delete of the folder at `path` removes, after the writes before it.
    pub(crate) fn folder_delete_count(
        &mut self,
        path: FolderPath,
        done: Done<DeleteCount>,
        cx: &mut Context<Self>,
    ) {
        self.write(move |core| Ok(core.folder_delete_count(&path)), done, cx);
    }

    /// Renames a tag in all notes, and gives the number of changed notes.
    pub(crate) fn rename_tag(
        &mut self,
        from: TagName,
        to: TagName,
        done: Done<usize>,
        cx: &mut Context<Self>,
    ) {
        self.write(move |core| core.rename_tag(&from, &to), done, cx);
    }

    /// The typed name `to` as a tag, and the number of notes that a rename to it changes,
    /// after the writes before it.
    pub(crate) fn tag_rename_count(
        &mut self,
        from: TagName,
        to: String,
        done: Done<(TagName, usize)>,
        cx: &mut Context<Self>,
    ) {
        self.write(
            move |core| {
                let to = TagName::typed(&to)?;
                let count = core.tag_rename_count(&from, &to);
                Ok((to, count))
            },
            done,
            cx,
        );
    }

    /// Removes a tag from all notes, and gives the number of changed notes.
    pub(crate) fn delete_tag(&mut self, tag: TagName, done: Done<usize>, cx: &mut Context<Self>) {
        self.write(move |core| core.delete_tag(&tag), done, cx);
    }

    /// The number of notes that a tag delete changes, after the writes before it.
    pub(crate) fn tag_delete_count(
        &mut self,
        tag: TagName,
        done: Done<usize>,
        cx: &mut Context<Self>,
    ) {
        self.write(move |core| Ok(core.tag_delete_count(&tag)), done, cx);
    }

    /// Copies pasted or dropped files into `attachments/`, and gives the link to each copy
    /// from the note at `note`. A file with no copy gives `None`.
    pub(crate) fn attach(
        &mut self,
        note: NotePath,
        files: Vec<PastedFile>,
        done: impl FnOnce(Vec<Option<PathBuf>>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.queue(
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
            |copies, cx| {
                let copies = each_answered(copies.unwrap_or_default(), cx);
                cx.defer(move |cx| done(copies, cx));
            },
            cx,
        );
    }

    /// The link to each file from the note at `note`, with no copy. A file with no link gives
    /// `None`.
    pub(crate) fn file_links(
        &self,
        note: NotePath,
        files: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) -> Task<Vec<Option<PathBuf>>> {
        let job = self.look(
            move |core| {
                files
                    .iter()
                    .map(|file| core.file_link(&note, file))
                    .collect()
            },
            cx,
        );
        cx.spawn(async move |folder, cx| {
            let links = job.await;
            folder
                .update(cx, |_, cx| each_answered(links, cx))
                .unwrap_or_default()
        })
    }

    /// The notes that match `query`, in `order`. It does not wait for the line. A failed
    /// search gives `None`.
    pub(crate) fn search(
        &self,
        query: String,
        order: Order,
        cx: &mut Context<Self>,
    ) -> Task<Option<SearchResults>> {
        let job = self.look(move |core| core.search(&query, order), cx);
        cx.spawn(async move |folder, cx| {
            let found = job.await;
            folder
                .update(cx, |_, cx| answered(found, cx).ok())
                .ok()
                .flatten()
        })
    }

    /// The headings of the note at `path`. It does not wait for the line.
    pub(crate) fn headings(
        &self,
        path: NotePath,
        cx: &App,
    ) -> Task<holm_core::Result<Vec<String>>> {
        self.look(move |core| core.headings(&path), cx)
    }

    /// The note links in the note at `path` whose note or heading is gone. It does not wait
    /// for the line.
    pub(crate) fn broken_links(
        &self,
        path: NotePath,
        cx: &App,
    ) -> Task<holm_core::Result<Vec<LinkTarget>>> {
        self.look(move |core| core.broken_links(&path), cx)
    }

    /// Puts `job` in the line, and waits for it up to the budget. When it ends, the entity sets
    /// the snapshot after it and sends the event. Then `done` gets its result, in the same
    /// update. When the entity is gone first, `done` never runs.
    fn write<T: Send + 'static>(
        &mut self,
        job: impl FnOnce(&Core) -> holm_core::Result<T> + Send + 'static,
        done: Done<T>,
        cx: &mut Context<Self>,
    ) {
        self.queue(
            job,
            |result, cx| {
                let result = answered(result, cx);
                // A deferred step runs after the views took the event, in the same update.
                cx.defer(move |cx| (done.0)(result, cx));
            },
            cx,
        );
    }

    /// Like [`Self::write`], but `end` gets the result on the entity, before the deferred steps.
    fn queue<T: Send + 'static>(
        &mut self,
        job: impl FnOnce(&Core) -> holm_core::Result<T> + Send + 'static,
        end: impl FnOnce(holm_core::Result<T>, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.push_draft();
        let number = self.push(job, end);
        self.run(Some(number), cx);
    }

    fn push_draft(&mut self) {
        let Some(write) = self.draft.write() else {
            return;
        };
        let (send, mut receive) = oneshot::channel();
        let job: Job = Box::new(move |core: &Core| {
            let failure = write.text.write(core).err().map(AppError::from);
            let chain = failure.as_ref().map(|error| crate::chain(error));
            send.send(failure).ok();
            chain
        });
        let version = write.version;
        let end: End = Box::new(move |folder, cx| {
            let Ok(Some(failure)) = receive.try_recv() else {
                return;
            };
            match (folder.draft.written(version, failure.is_none()), failure) {
                (Written::Saved, _) => cx.emit(SaveEvent::Saved),
                (Written::Failed, Some(error)) => cx.emit(SaveEvent::Failed(error)),
                _ => {}
            }
        });
        self.push_entry(job, end);
    }

    /// Puts `job` at the end of the line, and gives the number of its entry.
    fn push<T: Send + 'static>(
        &mut self,
        job: impl FnOnce(&Core) -> holm_core::Result<T> + Send + 'static,
        end: impl FnOnce(holm_core::Result<T>, &mut Context<Self>) + 'static,
    ) -> u64 {
        let (send, mut receive) = oneshot::channel();
        let job: Job = Box::new(move |core: &Core| {
            let result = job(core);
            let failure = result.as_ref().err().map(|error| crate::chain(error));
            send.send(result).ok();
            failure
        });
        let end: End = Box::new(move |_, cx: &mut Context<Self>| {
            if let Ok(Some(result)) = receive.try_recv() {
                end(result, cx);
            }
        });
        self.push_entry(job, end)
    }

    /// Puts an entry at the end of the line, and gives its number.
    fn push_entry(&mut self, job: Job, end: End) -> u64 {
        let number = self.next_number;
        self.next_number += 1;
        self.line.push_back(Entry { number, job, end });
        number
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

    /// Runs the jobs of the line. With `until`, it waits on this thread up to the budget, until
    /// the entry `until` ends. A job that does not end in time ends later, in its own update.
    fn run(&mut self, until: Option<u64>, cx: &mut Context<Self>) {
        let deadline = Instant::now() + self.budget;
        loop {
            if self.turn.is_none() && !self.start(cx) {
                return;
            }
            let Some(until) = until.filter(|until| self.ended < *until) else {
                return;
            };
            let left = deadline.saturating_duration_since(Instant::now());
            let done = match &mut self.turn {
                Some(turn) if !left.is_zero() => cx
                    .foreground_executor()
                    .block_with_timeout(left, &mut turn.done)
                    .ok(),
                _ => None,
            };
            let Some(done) = done else {
                if self.late < Some(until) {
                    self.late = Some(until);
                    cx.notify();
                }
                return;
            };
            self.end_turn(done.is_ok(), cx);
        }
    }

    /// Starts the first job of the line, and gives whether it did.
    fn start(&mut self, cx: &mut Context<Self>) -> bool {
        if self.closed {
            return false;
        }
        let Some(Entry { number, job, end }) = self.line.pop_front() else {
            return false;
        };
        *lock(&self.running) = Some(job);
        let core = self.core.clone();
        let running = self.running.clone();
        let (send, done) = oneshot::channel();
        let work = cx.background_spawn(async move {
            let mut slot = lock(&running);
            if let Some(job) = slot.take() {
                job(&core);
                send.send(()).ok();
            }
        });
        let wake = cx.spawn(async move |folder, cx| {
            work.await;
            folder
                .update(cx, |folder, cx| {
                    let ran = folder
                        .turn
                        .as_mut()
                        .is_some_and(|turn| matches!(turn.done.try_recv(), Ok(Some(()))));
                    folder.end_turn(ran, cx);
                    folder.run(None, cx);
                })
                .ok();
        });
        self.turn = Some(Turn {
            number,
            end,
            done,
            _wake: wake,
        });
        true
    }

    /// Ends the entry whose job ends. When the job `ran` on its thread, it takes every list
    /// that waits and sends one event. Then the watch starts again.
    fn end_turn(&mut self, ran: bool, cx: &mut Context<Self>) {
        let Some(turn) = self.turn.take() else {
            return;
        };
        self.ended = turn.number;
        if ran {
            let lists: Vec<ChangeList> = std::iter::from_fn(|| self.changes.try_next()).collect();
            let event = self.keep(lists);
            cx.emit(event);
            (turn.end)(self, cx);
        }
        if self.late.is_some_and(|late| late <= self.ended) {
            self.late = None;
            cx.notify();
        }
        self.watch = Self::watch(cx);
    }

    /// Runs every job that waits, and then writes the draft. Later jobs do not run. It works on
    /// the UI thread, because no task runs after the window is gone.
    fn drain(&mut self) {
        self.closed = true;
        let draft: Option<Job> = self.draft.take().map(|text| {
            Box::new(move |core: &Core| text.write(core).err().map(|error| crate::chain(&error)))
                as Job
        });
        let mut slot = lock(&self.running);
        let jobs = slot
            .take()
            .into_iter()
            .chain(self.line.drain(..).map(|entry| entry.job))
            .chain(draft);
        for job in jobs {
            if let Some(failure) = job(&self.core) {
                tracing::warn!(error = %failure, "a write at quit failed");
            }
        }
    }

    /// Wakes on each change list while no job runs. A job ends it, and [`Self::end_turn`]
    /// starts a new one.
    fn watch(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |folder, cx| {
            std::future::poll_fn(|waker| {
                folder
                    .update(cx, |folder, cx| folder.wake(waker, cx))
                    .unwrap_or(Poll::Ready(()))
            })
            .await;
        })
    }

    /// Takes every list that waits and sends one event. During a job, it takes nothing.
    fn wake(&mut self, waker: &mut task::Context<'_>, cx: &mut Context<Self>) -> Poll<()> {
        if self.turn.is_some() {
            return Poll::Ready(());
        }
        let mut lists = Vec::new();
        while let Poll::Ready(Some(list)) = Pin::new(&mut self.changes).poll_next(waker) {
            lists.push(list);
        }
        if !lists.is_empty() {
            let event = self.keep(lists);
            cx.emit(event);
        }
        Poll::Pending
    }

    /// Keeps the snapshot of the last list, and gives the event with the changes of all lists.
    fn keep(&mut self, lists: Vec<ChangeList>) -> FolderEvent {
        let mut changes = Vec::new();
        for list in lists {
            for change in &list.changes {
                self.draft.follow(list.source, change, &self.snapshot);
            }
            self.snapshot = list.snapshot;
            changes.extend(list.changes.into_iter().map(|change| (list.source, change)));
        }
        FolderEvent { changes }
    }
}

impl EventEmitter<FolderEvent> for NotesFolder {}

impl EventEmitter<SaveEvent> for NotesFolder {}

impl EventEmitter<JobFailed> for NotesFolder {}

/// Gives the result with the app error. A failure goes to the log, and its answer to
/// [`JobFailed`].
fn answered<T>(result: holm_core::Result<T>, cx: &mut Context<NotesFolder>) -> Result<T, AppError> {
    result.map_err(|error| {
        let error = AppError::from(error);
        tracing::warn!(error = %crate::chain(&error), "a job of the user failed");
        cx.emit(JobFailed(error.message()));
        error
    })
}

/// The value of each result, or `None` after its failure went to [`answered`].
fn each_answered<T>(
    results: Vec<holm_core::Result<T>>,
    cx: &mut Context<NotesFolder>,
) -> Vec<Option<T>> {
    results
        .into_iter()
        .map(|result| answered(result, cx).ok())
        .collect()
}

/// Opens the core and gives the note to show first, if any. `create` makes the folder first.
pub(crate) fn open(
    open: &dyn Fn(&Path, &Path) -> holm_core::Result<Core>,
    folder: &Path,
    index_dir: &Path,
    create: bool,
    last: Option<&NotePath>,
    order: Order,
) -> Result<(Core, Option<Note>), AppError> {
    if create && let Err(error) = std::fs::create_dir_all(folder) {
        tracing::warn!(path = %folder.display(), %error, "cannot make the notes folder");
    }
    let core = open(folder, index_dir)?;
    let note = first_note(&core, last, order)?;
    Ok((core, note))
}

/// The note to open first: the last open note when it still exists, else the first note in
/// `order`. With no notes, it is `None`.
fn first_note(
    core: &Core,
    last: Option<&NotePath>,
    order: Order,
) -> holm_core::Result<Option<Note>> {
    if let Some(note) = last.and_then(|last| core.read(last).ok()) {
        return Ok(Some(note));
    }
    match core.snapshot().first(NoteSet::All, order) {
        Some(info) => core.read(&info.path).map(Some),
        None => Ok(None),
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
    use crate::test_paths::{folder_path, note_path, tag_name};

    fn notes_folder(cx: &mut TestAppContext) -> (TempDir, Entity<NotesFolder>) {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes");
        fs::create_dir(&notes).unwrap();
        fs::write(notes.join("A.md"), "# A\n").unwrap();
        let (core, _) = Core::open_with_feed(&notes, dir.path().join("index")).unwrap();
        let window = cx.add_window(|_, _| gpui_kit::Empty);
        let notes_folder = window
            .update(cx, |_, window, cx| {
                cx.new(|cx| NotesFolder::new(core, notes, Duration::ZERO, window, cx))
            })
            .unwrap();
        (dir, notes_folder)
    }

    #[gpui_kit::test]
    fn a_write_after_a_failed_write_still_runs(cx: &mut TestAppContext) {
        let (dir, notes_folder) = notes_folder(cx);
        let steps = Rc::new(RefCell::new(Vec::new()));
        notes_folder.update(cx, |notes_folder, cx| {
            let step = |name: &'static str| {
                let steps = steps.clone();
                Done::result(move |result: Result<FolderPath, AppError>, _: &mut App| {
                    if result.is_ok() {
                        steps.borrow_mut().push(name);
                    }
                })
            };
            notes_folder.rename_folder(folder_path("gone"), "x".into(), step("after the fail"), cx);
            notes_folder.create_folder(
                FolderPath::root(),
                "B".into(),
                step("after the folder"),
                cx,
            );
        });
        cx.run_until_parked();

        assert_eq!(*steps.borrow(), ["after the folder"]);
        assert!(dir.path().join("notes/B").is_dir());
        let folders: Vec<FolderPath> = notes_folder.read_with(cx, |notes_folder, _| {
            notes_folder.snapshot().folders().cloned().collect()
        });
        assert_eq!(folders, [folder_path("B")]);
    }

    #[gpui_kit::test]
    fn a_failed_job_sends_its_answer_and_skips_its_value_step(cx: &mut TestAppContext) {
        let (_dir, notes_folder) = notes_folder(cx);
        let steps = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let steps = steps.clone();
            cx.subscribe(&notes_folder, move |_, JobFailed(answer), _| {
                steps.borrow_mut().push(format!("answer {answer}"));
            })
            .detach();
        });
        notes_folder.update(cx, |notes_folder, cx| {
            let value = {
                let steps = steps.clone();
                Done::value(move |_, _| steps.borrow_mut().push("value".to_owned()))
            };
            notes_folder.rename_folder(folder_path("gone"), "x".into(), value, cx);
            let result = {
                let steps = steps.clone();
                Done::result(move |result: Result<FolderPath, AppError>, _| {
                    let message = result.err().map(|error| error.message());
                    steps.borrow_mut().push(format!("result {message:?}"));
                })
            };
            notes_folder.rename_folder(folder_path("gone"), "x".into(), result, cx);
        });
        cx.run_until_parked();

        let answer = crate::text::READ;
        assert_eq!(
            *steps.borrow(),
            [
                format!("answer {answer}"),
                format!("answer {answer}"),
                format!("result Some({answer:?})")
            ]
        );
    }

    #[gpui_kit::test]
    fn each_write_sends_its_moves_and_then_gives_its_result(cx: &mut TestAppContext) {
        let (_dir, notes_folder) = notes_folder(cx);
        let steps = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let steps = steps.clone();
            cx.subscribe(&notes_folder, move |_, event: &FolderEvent, _| {
                steps
                    .borrow_mut()
                    .push(format!("event {:?}", event.changes().collect::<Vec<_>>()));
            })
            .detach();
        });
        notes_folder.update(cx, |notes_folder, cx| {
            let step = |name: &'static str| {
                let steps = steps.clone();
                Done::result(move |result: Result<FolderPath, AppError>, _: &mut App| {
                    let end = if result.is_ok() { "ok" } else { "failed" };
                    steps.borrow_mut().push(format!("{name} {end}"));
                })
            };
            notes_folder.create_folder(FolderPath::root(), "work".into(), step("create"), cx);
            notes_folder.rename_folder(folder_path("work"), "job".into(), step("rename"), cx);
            notes_folder.rename_folder(folder_path("gone"), "x".into(), step("fail"), cx);
        });
        cx.run_until_parked();

        assert_eq!(
            *steps.borrow(),
            [
                "event [Folders]",
                "create ok",
                r#"event [FolderMoved { from: "work", to: "job" }, Folders]"#,
                "rename ok",
                "event []",
                "fail failed",
            ]
        );
    }

    #[gpui_kit::test]
    fn the_writes_in_the_line_reach_the_files_at_quit_with_the_draft_last(cx: &mut TestAppContext) {
        let (dir, notes_folder) = notes_folder(cx);
        notes_folder.update(cx, |notes_folder, cx| {
            notes_folder.set_text(note_path("A.md"), "# A\n\nfirst\n".into());
            notes_folder.change_tag(
                note_path("A.md"),
                TagChange::Add(tag_name("work")),
                Done::value(|(), _| {}),
                cx,
            );
            notes_folder.create_folder(FolderPath::root(), "B".into(), Done::value(|_, _| {}), cx);
            notes_folder.set_text(note_path("A.md"), "# A\n\nlast\n".into());
        });
        cx.quit();

        let file = fs::read_to_string(dir.path().join("notes/A.md")).unwrap();
        assert!(file.contains("  - work\n"), "{file}");
        assert!(file.ends_with("# A\n\nlast\n"), "{file}");
        assert!(dir.path().join("notes/B").is_dir());
    }

    #[gpui_kit::test]
    fn a_job_sees_the_draft_and_a_newer_draft_waits_for_the_next_job(cx: &mut TestAppContext) {
        let (_dir, notes_folder) = notes_folder(cx);
        let bodies = Rc::new(RefCell::new(Vec::new()));
        let read = |notes_folder: &mut NotesFolder, cx: &mut Context<NotesFolder>| {
            let bodies = bodies.clone();
            notes_folder.read(
                note_path("A.md"),
                Done::value(move |note: Note, _| bodies.borrow_mut().push(note.body)),
                cx,
            );
        };
        notes_folder.update(cx, |notes_folder, cx| {
            notes_folder.set_text(note_path("A.md"), "# A\n\nold\n".into());
            notes_folder.set_text(note_path("A.md"), "# A\n\nnew\n".into());
            read(notes_folder, cx);
            read(notes_folder, cx);
            notes_folder.set_text(note_path("A.md"), "# A\n\nlater\n".into());
        });
        cx.run_until_parked();
        notes_folder.update(cx, |notes_folder, cx| read(notes_folder, cx));
        cx.run_until_parked();

        assert_eq!(
            *bodies.borrow(),
            ["# A\n\nnew\n", "# A\n\nnew\n", "# A\n\nlater\n"]
        );
    }

    #[gpui_kit::test]
    fn a_failed_save_keeps_the_text_for_the_next_save(cx: &mut TestAppContext) {
        let (dir, notes_folder) = notes_folder(cx);
        let notes = dir.path().join("notes");
        let read_only = |read_only: bool| {
            let mut permissions = fs::metadata(&notes).unwrap().permissions();
            permissions.set_readonly(read_only);
            fs::set_permissions(&notes, permissions).unwrap();
        };
        let steps = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let steps = steps.clone();
            cx.subscribe(&notes_folder, move |_, event: &SaveEvent, _| {
                let end = match event {
                    SaveEvent::Saved => "saved",
                    SaveEvent::Failed(_) => "failed",
                };
                steps.borrow_mut().push(end.to_owned());
            })
            .detach();
        });
        let wait = |cx: &mut TestAppContext| {
            let steps = steps.clone();
            notes_folder.update(cx, |notes_folder, cx| {
                notes_folder.wait_saved(
                    move |saved, _| steps.borrow_mut().push(format!("wait {saved:?}")),
                    cx,
                );
            });
            cx.run_until_parked();
        };

        read_only(true);
        notes_folder.update(cx, |notes_folder, cx| {
            notes_folder.set_text(note_path("A.md"), "# A\n\nmine\n".into());
            notes_folder.save_text(cx);
        });
        cx.run_until_parked();
        wait(cx);
        assert!(notes_folder.read_with(cx, |notes_folder, _| notes_folder.unsaved()));

        read_only(false);
        wait(cx);
        assert!(!notes_folder.read_with(cx, |notes_folder, _| notes_folder.unsaved()));
        assert_eq!(
            *steps.borrow(),
            [
                "failed",
                "failed",
                "wait Err(NotSaved)",
                "saved",
                "wait Ok(())"
            ]
        );
        let text = fs::read_to_string(notes.join("A.md")).unwrap();
        assert!(text.ends_with("\n---\n# A\n\nmine\n"), "{text}");
    }

    #[gpui_kit::test]
    fn a_save_keeps_a_tag_from_another_tool(cx: &mut TestAppContext) {
        let (dir, notes_folder) = notes_folder(cx);
        let file = dir.path().join("notes/A.md");
        fs::write(&file, "---\ntags: [theirs]\n---\n# A\n").unwrap();

        notes_folder.update(cx, |notes_folder, cx| {
            notes_folder.set_text(note_path("A.md"), "# A\n\nmine\n".into());
            notes_folder.save_text(cx);
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
                notes_folder.change_tag(note_path("A.md"), change, Done::value(|(), _| {}), cx);
            });
            cx.run_until_parked();
        };

        change(TagChange::Add(tag_name("a")), cx);
        fs::write(&file, "---\ntags: [a, b]\n---\n# A\n").unwrap();
        change(TagChange::Remove(tag_name("a")), cx);

        let tags = holm_markdown::NoteFile::parse(&fs::read_to_string(&file).unwrap()).tags();
        assert_eq!(tags, ["b"]);
    }
}
