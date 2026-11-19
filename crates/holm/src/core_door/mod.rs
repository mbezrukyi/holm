//! The app's only door to the core.
//!
//! [`CoreDoor`] holds the latest [`Snapshot`] and runs every read and write of the core.
//! Writes, and reads that must see them, are jobs. They wait in the write line and run one at
//! a time, in the order the UI asked for them. A failed job does not stop the line.
//!
//! Each job ends in one update. The entity sets the snapshot after it and sends one
//! [`FolderEvent`] with the changes up to that snapshot. Then, in the same update, the caller
//! gets the result. So no frame falls between them.
//!
//! A call waits on the UI thread for the jobs before it and its own, up to a budget. A job that
//! ends in time ends in the update of the call, so no frame falls between the key and the end
//! of its job. A late job ends in a later update, and [`CoreDoor::writing`] is true until then.
//!
//! It holds one [`Draft`]: the unsaved editor text of the open note, and of the notes the editor
//! left. Before each job, the write line writes the draft. So no job sees a note file older than
//! the editor text. When the editor takes text of another note, the old text goes in the line at
//! once. The draft keeps each text until a write of it succeeds, and each write of it ends in at
//! most one [`SaveEvent`]. While a disk question of a note is open, no job writes its text.
//!
//! A change list from outside Holm wakes it when no job runs, and it sends one event. During a
//! job, the lists wait for the event of the job. When its window closes or the app quits, it runs
//! every job that waits in the write line, and then writes the draft.
//!
//! Each job of the line, the search and the file links are work of the user. When one fails, it
//! sends one [`JobFailed`] with the answer for the message line. So a caller gets only the
//! value, unless it owns the error. The draft is Holm's own work, and its fail is a
//! [`SaveEvent`]. The other reads give their error, for the log.

use std::path::{Path, PathBuf};
use std::pin::Pin;
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
use draft::{Draft, DraftSave, Written};
use write_line::{End, Work, WriteLine};

mod draft;
mod write_line;

/// What the core door tells the views: a job of Holm ended, or changes from outside came in.
/// Each job sends one, also a failed job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FolderEvent {
    /// The changes of every revision up to [`CoreDoor::snapshot`], in order, each with
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
pub(crate) type Then<R> = Box<dyn FnOnce(R, &mut App)>;

/// What the caller of a job gets at its end. A failure shows as an answer in both cases.
pub(crate) struct Done<T>(Then<Result<T, AppError>>);

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
) -> Then<R> {
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

/// The editor text is not on disk after the writes: the last write of the draft failed, or a
/// disk question holds the text. The [`SaveEvent`] of a failed write has the error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NotSaved;

pub(crate) struct CoreDoor {
    core: Core,
    root: PathBuf,
    snapshot: Snapshot,
    changes: ChangeLists,
    /// How long a call waits for its job.
    budget: Duration,
    write_line: WriteLine,
    draft: Draft,
    watch: Task<()>,
    _ends: [Subscription; 2],
}

impl CoreDoor {
    /// The door to the notes folder at `root`, which `core` opened. A call waits up to `budget`
    /// for its job. The write line drains when `window` closes, when the app quits, or when the
    /// entity goes.
    pub(crate) fn new(
        core: Core,
        root: PathBuf,
        budget: Duration,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.on_release(|door, _| door.drain()).detach();
        // The platform window can keep views alive after a close, and with them this entity.
        let window = window.window_handle().window_id();
        let door = cx.weak_entity();
        let ends = [
            cx.on_window_closed(move |cx, closed| {
                if closed == window {
                    door.update(cx, |door, _| door.drain()).ok();
                }
            }),
            cx.on_app_quit(|door, _| {
                door.drain();
                std::future::ready(())
            }),
        ];
        let (snapshot, changes) = core.follow();
        Self {
            changes,
            snapshot,
            write_line: WriteLine::new(core.clone()),
            core,
            root,
            budget,
            draft: Draft::default(),
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
        self.write_line.is_late()
    }

    /// The editor text of the note at `path` is now `body`. It writes nothing yet. Text of
    /// another note before it goes in the write line now.
    pub(crate) fn set_text(&mut self, path: NotePath, body: String, cx: &mut Context<Self>) {
        if let Some(old) = self.draft.set(path, body) {
            self.push_save(old);
            self.run(None, cx);
        }
    }

    /// Drops the editor text of the note at `path` with no write.
    pub(crate) fn drop_text(&mut self, path: &NotePath) {
        self.draft.drop_text(path);
    }

    /// A disk question of the note at `path` opened. No job writes its editor text until
    /// [`Self::keep_text`] or [`Self::drop_text`].
    pub(crate) fn hold_text(&mut self, path: &NotePath) {
        self.draft.hold(path);
    }

    /// The user keeps the editor text of the note at `path` over the file. The hold ends, and
    /// the draft goes in the write line now.
    pub(crate) fn keep_text(&mut self, path: &NotePath, cx: &mut Context<Self>) {
        self.draft.release(path);
        self.save_text(cx);
    }

    /// Whether the draft has text that is not on disk.
    pub(crate) fn unsaved(&self) -> bool {
        self.draft.unsaved()
    }

    /// Whether the draft has text of the note at `path` that is not on disk.
    pub(crate) fn unsaved_note(&self, path: &NotePath) -> bool {
        self.draft.unsaved_note(path)
    }

    /// Puts the draft in the write line now. It does not wait for the job.
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
        let end: End = Box::new(move |door, cx| {
            let saved = if door.draft.kept_back() {
                Err(NotSaved)
            } else {
                Ok(())
            };
            cx.defer(move |cx| done(saved, cx));
        });
        let number = self.write_line.push(Box::new(|_| None), end);
        self.run(Some(number), cx);
    }

    /// Reads the note at `path`, after the writes before it.
    pub(crate) fn read(&mut self, path: NotePath, done: Done<Note>, cx: &mut Context<Self>) {
        self.write(move |core| core.read(&path), done, cx);
    }

    /// Reads the note at `path` to replace its editor text. The text drops now with no write,
    /// and comes back with its hold when the read fails.
    pub(crate) fn read_and_drop_text(
        &mut self,
        path: NotePath,
        done: Done<Note>,
        cx: &mut Context<Self>,
    ) {
        let text = self.draft.drop_for_read(&path);
        self.queue(
            move |core| core.read(&path),
            move |door, note, cx| {
                if let Some(id) = text {
                    door.draft.read_ended(id, note.is_ok());
                }
                give(note, done, cx);
            },
            cx,
        );
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
            |_, copies, cx| {
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
        cx.spawn(async move |door, cx| {
            let links = job.await;
            door.update(cx, |_, cx| each_answered(links, cx))
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
        cx.spawn(async move |door, cx| {
            let found = job.await;
            door.update(cx, |_, cx| answered(found, cx).ok())
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

    /// Puts `job` in the write line, and waits for it up to the budget. When it ends, the entity
    /// sets the snapshot after it and sends the event. Then `done` gets its result, in the same
    /// update. When the entity is gone first, `done` never runs.
    fn write<T: Send + 'static>(
        &mut self,
        job: impl FnOnce(&Core) -> holm_core::Result<T> + Send + 'static,
        done: Done<T>,
        cx: &mut Context<Self>,
    ) {
        self.queue(job, |_, result, cx| give(result, done, cx), cx);
    }

    /// Like [`Self::write`], but `end` gets the result on the entity, before the deferred steps.
    fn queue<T: Send + 'static>(
        &mut self,
        job: impl FnOnce(&Core) -> holm_core::Result<T> + Send + 'static,
        end: impl FnOnce(&mut Self, holm_core::Result<T>, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.push_draft();
        let number = self.push(job, end);
        self.run(Some(number), cx);
    }

    fn push_draft(&mut self) {
        for save in self.draft.writes() {
            self.push_save(save);
        }
    }

    fn push_save(&mut self, save: DraftSave) {
        let (send, mut receive) = oneshot::channel();
        let work: Work = Box::new(move |core: &Core| {
            let failure = save.text.write(core).err().map(AppError::from);
            let chain = failure.as_ref().map(|error| crate::chain(error));
            send.send(failure).ok();
            chain
        });
        let (id, version) = (save.id, save.version);
        let end: End = Box::new(move |door, cx| {
            let Ok(Some(failure)) = receive.try_recv() else {
                return;
            };
            match (door.draft.written(id, version, failure.is_none()), failure) {
                (Written::Saved, _) => cx.emit(SaveEvent::Saved),
                (Written::Failed, Some(error)) => cx.emit(SaveEvent::Failed(error)),
                _ => {}
            }
        });
        self.write_line.push(work, end);
    }

    /// Puts `job` at the end of the write line, and gives its number.
    fn push<T: Send + 'static>(
        &mut self,
        job: impl FnOnce(&Core) -> holm_core::Result<T> + Send + 'static,
        end: impl FnOnce(&mut Self, holm_core::Result<T>, &mut Context<Self>) + 'static,
    ) -> u64 {
        let (send, mut receive) = oneshot::channel();
        let work: Work = Box::new(move |core: &Core| {
            let result = job(core);
            let failure = result.as_ref().err().map(|error| crate::chain(error));
            send.send(result).ok();
            failure
        });
        let end: End = Box::new(move |door, cx: &mut Context<Self>| {
            if let Ok(Some(result)) = receive.try_recv() {
                end(door, result, cx);
            }
        });
        self.write_line.push(work, end)
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

    /// Runs the jobs of the write line. With `until`, it waits on this thread up to the budget,
    /// until the job `until` ends. A job that does not end in time ends later, in its own update.
    fn run(&mut self, until: Option<u64>, cx: &mut Context<Self>) {
        let writing = self.writing();
        self.run_line(until, cx);
        self.tell_writing(writing, cx);
    }

    /// Ends the running job, whose work ended after the call that started it, and runs the
    /// jobs that wait.
    fn job_woke(&mut self, ran: bool, cx: &mut Context<Self>) {
        let writing = self.writing();
        self.end_job(ran, cx);
        self.run_line(None, cx);
        self.tell_writing(writing, cx);
    }

    /// Notifies the observers when [`Self::writing`] changed since `before`. It comes after the
    /// ends of the jobs, so the observers run after the deferred steps of the ends. A step can
    /// ask for a new late job, and the observers never see the short state between the jobs.
    fn tell_writing(&self, before: bool, cx: &mut Context<Self>) {
        if self.writing() != before {
            cx.notify();
        }
    }

    fn run_line(&mut self, until: Option<u64>, cx: &mut Context<Self>) {
        let deadline = Instant::now() + self.budget;
        loop {
            if !self.write_line.is_running() && !self.write_line.start(cx) {
                return;
            }
            let Some(until) = until.filter(|until| !self.write_line.has_ended(*until)) else {
                return;
            };
            let left = deadline.saturating_duration_since(Instant::now());
            let Some(ran) = self.write_line.wait(until, left, cx) else {
                return;
            };
            self.end_job(ran, cx);
        }
    }

    /// Ends the running job. When its work `ran` on its thread, it takes every list that waits
    /// and sends one event. Then the watch starts again.
    fn end_job(&mut self, ran: bool, cx: &mut Context<Self>) {
        let Some(end) = self.write_line.end() else {
            return;
        };
        if ran {
            let lists: Vec<ChangeList> = std::iter::from_fn(|| self.changes.try_next()).collect();
            let event = self.keep(lists);
            cx.emit(event);
            end(self, cx);
        }
        self.watch = Self::watch(cx);
    }

    /// Runs every job that waits, and then writes the draft. Later jobs do not run.
    fn drain(&mut self) {
        let texts = self.draft.take();
        let draft: Work = Box::new(move |core: &Core| {
            let failures: Vec<String> = texts
                .into_iter()
                .filter_map(|text| text.write(core).err())
                .map(|error| crate::chain(&error))
                .collect();
            (!failures.is_empty()).then(|| failures.join("; "))
        });
        self.write_line.drain(draft);
    }

    /// Wakes on each change list while no job runs. A job ends it, and [`Self::end_job`]
    /// starts a new one.
    fn watch(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |door, cx| {
            std::future::poll_fn(|waker| {
                door.update(cx, |door, cx| door.wake(waker, cx))
                    .unwrap_or(Poll::Ready(()))
            })
            .await;
        })
    }

    /// Takes every list that waits and sends one event. During a job, it takes nothing.
    fn wake(&mut self, waker: &mut task::Context<'_>, cx: &mut Context<Self>) -> Poll<()> {
        if self.write_line.is_running() {
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

impl EventEmitter<FolderEvent> for CoreDoor {}

impl EventEmitter<SaveEvent> for CoreDoor {}

impl EventEmitter<JobFailed> for CoreDoor {}

/// Gives the result with the app error. A failure goes to the log, and its answer to
/// [`JobFailed`].
fn answered<T>(result: holm_core::Result<T>, cx: &mut Context<CoreDoor>) -> Result<T, AppError> {
    result.map_err(|error| {
        let error = AppError::from(error);
        tracing::warn!(error = %crate::chain(&error), "a job of the user failed");
        cx.emit(JobFailed(error.message()));
        error
    })
}

/// Gives the end of a job to its caller, after [`answered`].
fn give<T: 'static>(result: holm_core::Result<T>, done: Done<T>, cx: &mut Context<CoreDoor>) {
    let result = answered(result, cx);
    // A deferred step runs after the views took the event, in the same update.
    cx.defer(move |cx| (done.0)(result, cx));
}

/// The value of each result, or `None` after its failure went to [`answered`].
fn each_answered<T>(
    results: Vec<holm_core::Result<T>>,
    cx: &mut Context<CoreDoor>,
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

    fn core_door(cx: &mut TestAppContext) -> (TempDir, Entity<CoreDoor>) {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes");
        fs::create_dir(&notes).unwrap();
        fs::write(notes.join("A.md"), "# A\n").unwrap();
        let (core, _) = Core::open_with_feed(&notes, dir.path().join("index")).unwrap();
        let window = cx.add_window(|_, _| gpui_kit::Empty);
        let core_door = window
            .update(cx, |_, window, cx| {
                cx.new(|cx| CoreDoor::new(core, notes, Duration::ZERO, window, cx))
            })
            .unwrap();
        (dir, core_door)
    }

    #[gpui_kit::test]
    fn a_write_after_a_failed_write_still_runs(cx: &mut TestAppContext) {
        let (dir, core_door) = core_door(cx);
        let steps = Rc::new(RefCell::new(Vec::new()));
        core_door.update(cx, |core_door, cx| {
            let step = |name: &'static str| {
                let steps = steps.clone();
                Done::result(move |result: Result<FolderPath, AppError>, _: &mut App| {
                    if result.is_ok() {
                        steps.borrow_mut().push(name);
                    }
                })
            };
            core_door.rename_folder(folder_path("gone"), "x".into(), step("after the fail"), cx);
            core_door.create_folder(FolderPath::root(), "B".into(), step("after the folder"), cx);
        });
        cx.run_until_parked();

        assert_eq!(*steps.borrow(), ["after the folder"]);
        assert!(dir.path().join("notes/B").is_dir());
        let folders: Vec<FolderPath> = core_door.read_with(cx, |core_door, _| {
            core_door.snapshot().folders().cloned().collect()
        });
        assert_eq!(folders, [folder_path("B")]);
    }

    #[gpui_kit::test]
    fn a_failed_job_sends_its_answer_and_skips_its_value_step(cx: &mut TestAppContext) {
        let (_dir, core_door) = core_door(cx);
        let steps = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let steps = steps.clone();
            cx.subscribe(&core_door, move |_, JobFailed(answer), _| {
                steps.borrow_mut().push(format!("answer {answer}"));
            })
            .detach();
        });
        core_door.update(cx, |core_door, cx| {
            let value = {
                let steps = steps.clone();
                Done::value(move |_, _| steps.borrow_mut().push("value".to_owned()))
            };
            core_door.rename_folder(folder_path("gone"), "x".into(), value, cx);
            let result = {
                let steps = steps.clone();
                Done::result(move |result: Result<FolderPath, AppError>, _| {
                    let message = result.err().map(|error| error.message());
                    steps.borrow_mut().push(format!("result {message:?}"));
                })
            };
            core_door.rename_folder(folder_path("gone"), "x".into(), result, cx);
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
        let (_dir, core_door) = core_door(cx);
        let steps = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let steps = steps.clone();
            cx.subscribe(&core_door, move |_, event: &FolderEvent, _| {
                steps
                    .borrow_mut()
                    .push(format!("event {:?}", event.changes().collect::<Vec<_>>()));
            })
            .detach();
        });
        core_door.update(cx, |core_door, cx| {
            let step = |name: &'static str| {
                let steps = steps.clone();
                Done::result(move |result: Result<FolderPath, AppError>, _: &mut App| {
                    let end = if result.is_ok() { "ok" } else { "failed" };
                    steps.borrow_mut().push(format!("{name} {end}"));
                })
            };
            core_door.create_folder(FolderPath::root(), "work".into(), step("create"), cx);
            core_door.rename_folder(folder_path("work"), "job".into(), step("rename"), cx);
            core_door.rename_folder(folder_path("gone"), "x".into(), step("fail"), cx);
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
        let (dir, core_door) = core_door(cx);
        core_door.update(cx, |core_door, cx| {
            core_door.set_text(note_path("A.md"), "# A\n\nfirst\n".into(), cx);
            core_door.change_tag(
                note_path("A.md"),
                TagChange::Add(tag_name("work")),
                Done::value(|(), _| {}),
                cx,
            );
            core_door.create_folder(FolderPath::root(), "B".into(), Done::value(|_, _| {}), cx);
            core_door.set_text(note_path("A.md"), "# A\n\nlast\n".into(), cx);
        });
        cx.quit();

        let file = fs::read_to_string(dir.path().join("notes/A.md")).unwrap();
        assert!(file.contains("  - work\n"), "{file}");
        assert!(file.ends_with("# A\n\nlast\n"), "{file}");
        assert!(dir.path().join("notes/B").is_dir());
    }

    #[gpui_kit::test]
    fn a_job_sees_the_draft_and_a_newer_draft_waits_for_the_next_job(cx: &mut TestAppContext) {
        let (_dir, core_door) = core_door(cx);
        let bodies = Rc::new(RefCell::new(Vec::new()));
        let read = |core_door: &mut CoreDoor, cx: &mut Context<CoreDoor>| {
            let bodies = bodies.clone();
            core_door.read(
                note_path("A.md"),
                Done::value(move |note: Note, _| bodies.borrow_mut().push(note.body)),
                cx,
            );
        };
        core_door.update(cx, |core_door, cx| {
            core_door.set_text(note_path("A.md"), "# A\n\nold\n".into(), cx);
            core_door.set_text(note_path("A.md"), "# A\n\nnew\n".into(), cx);
            read(core_door, cx);
            read(core_door, cx);
            core_door.set_text(note_path("A.md"), "# A\n\nlater\n".into(), cx);
        });
        cx.run_until_parked();
        core_door.update(cx, |core_door, cx| read(core_door, cx));
        cx.run_until_parked();

        assert_eq!(
            *bodies.borrow(),
            ["# A\n\nnew\n", "# A\n\nnew\n", "# A\n\nlater\n"]
        );
    }

    #[gpui_kit::test]
    fn a_failed_save_keeps_the_text_for_the_next_save(cx: &mut TestAppContext) {
        let (dir, core_door) = core_door(cx);
        let notes = dir.path().join("notes");
        let read_only = |read_only: bool| {
            let mut permissions = fs::metadata(&notes).unwrap().permissions();
            permissions.set_readonly(read_only);
            fs::set_permissions(&notes, permissions).unwrap();
        };
        let steps = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let steps = steps.clone();
            cx.subscribe(&core_door, move |_, event: &SaveEvent, _| {
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
            core_door.update(cx, |core_door, cx| {
                core_door.wait_saved(
                    move |saved, _| steps.borrow_mut().push(format!("wait {saved:?}")),
                    cx,
                );
            });
            cx.run_until_parked();
        };

        read_only(true);
        core_door.update(cx, |core_door, cx| {
            core_door.set_text(note_path("A.md"), "# A\n\nmine\n".into(), cx);
            core_door.save_text(cx);
        });
        cx.run_until_parked();
        wait(cx);
        assert!(core_door.read_with(cx, |core_door, _| core_door.unsaved()));

        read_only(false);
        wait(cx);
        assert!(!core_door.read_with(cx, |core_door, _| core_door.unsaved()));
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
        let (dir, core_door) = core_door(cx);
        let file = dir.path().join("notes/A.md");
        fs::write(&file, "---\ntags: [theirs]\n---\n# A\n").unwrap();

        core_door.update(cx, |core_door, cx| {
            core_door.set_text(note_path("A.md"), "# A\n\nmine\n".into(), cx);
            core_door.save_text(cx);
        });
        cx.run_until_parked();

        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains("tags: [theirs]\n"), "{text}");
        assert!(text.ends_with("# A\n\nmine\n"), "{text}");
    }

    #[gpui_kit::test]
    fn a_tag_removal_keeps_a_tag_from_another_tool(cx: &mut TestAppContext) {
        let (dir, core_door) = core_door(cx);
        let file = dir.path().join("notes/A.md");
        let change = |change: TagChange, cx: &mut TestAppContext| {
            core_door.update(cx, |core_door, cx| {
                core_door.change_tag(note_path("A.md"), change, Done::value(|(), _| {}), cx);
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
