//! The app's only door to the core.
//!
//! [`CoreDoor`] holds the latest [`Snapshot`] and runs every read and write of the core. Writes, and reads that must
//! see them, are jobs. They wait in the write line and run one at a time, in the order the UI asked for them. A failed
//! job does not stop the line.
//!
//! Each job ends in one update. The jobs that end in one update send one [`DoorEvent`]: the entity sets the snapshot
//! after the last of them and sends the changes up to it, so each view reads the snapshot of the changes it takes.
//! Then, in the same update, each caller gets its result, in the order of the jobs. So no frame falls between them.
//!
//! A call waits on the UI thread for the jobs before it and its own, up to a budget. A job that ends in time ends in
//! the update of the call, so no frame falls between the key and the end of its job. A late job ends in a later update,
//! and [`CoreDoor::writing`] is true until then.
//!
//! It holds one [`Draft`]: the unsaved editor text of the open note, and of the notes the editor left. Before each job,
//! the write line writes the draft. So no job sees a note file older than the editor text. When the editor takes text
//! of another note, the old text goes in the line at once. The draft keeps each text until a write of it succeeds, and
//! each write of it ends in at most one [`SaveEvent`]. While a disk question of a note is open, no job writes its text.
//! Each write of the draft carries the revision of the snapshot when it goes in the line. The core refuses it after an
//! outside change that this revision does not hold, and the draft then holds the text as a disk question does.
//!
//! A change list from outside Holm wakes it when no job runs, and it sends one event. During a job, the lists wait for
//! the event of the job. When its window closes or the app quits, it runs every job that waits in the write line, and
//! then writes the draft.
//!
//! Each job of the line, the search and the file links are work of the user. When one fails, it sends one [`JobFailed`]
//! with the answer for the message line. The draft is Holm's own work. Each failed write of it sends a [`SaveEvent`],
//! and the first fail of the draft also sends a [`JobFailed`]. The other reads log their error and give an empty value.

use std::path::{Path, PathBuf};
use std::task::{self, Poll};
use std::time::Duration;

use futures_channel::oneshot;
use gpui_kit::{App, AppContext as _, Context, EventEmitter, Subscription, Task, Window};
use holm_core::model::{Change, ChangeList, DeleteCount, LinkTarget, Note, Order, SearchResults, Source};
use holm_core::{Core, FolderPath, LinkPath, NotePath, Snapshot, TagName};
use holm_editor::PastedFile;

use crate::error::AppError;
use crate::tags::TagChange;
pub(crate) use draft::DraftVersion;
use draft::{Draft, DraftSave, Written};
use write_line::{JobNumber, Work, WriteLine};

mod draft;
mod write_line;

/// What the core door tells the views: jobs of Holm ended, or changes from outside came in. The jobs that end in one
/// update send one, also failed jobs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DoorEvent {
    /// The changes of every revision up to [`CoreDoor::snapshot`], in order. It is empty when the snapshot did not
    /// change.
    changes: Vec<(Source, Change)>,
}

impl DoorEvent {
    pub(crate) fn changes(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().map(|(_, change)| change)
    }

    pub(crate) fn sourced(&self) -> impl Iterator<Item = (Source, &Change)> {
        self.changes.iter().map(|(source, change)| (*source, change))
    }

    /// Whether the snapshot stayed the same.
    pub(crate) fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// A step that gets the end of a job, on the UI thread.
pub(crate) type Then<R> = Box<dyn FnOnce(R, &mut App)>;

/// What the caller of a job gets at its end. A failure shows as an answer in both cases.
pub(crate) struct Done<T>(Then<Result<T, AppError>>);

impl<T: 'static> Done<T> {
    /// After a failure, `step` does not run.
    pub(crate) fn value(step: impl FnOnce(T, &mut App) + 'static) -> Self {
        Self(Box::new(move |result, cx| {
            if let Ok(value) = result {
                step(value, cx);
            }
        }))
    }

    /// A failure also sends its [`JobFailed`], so `step` does not show it.
    pub(crate) fn result(step: impl FnOnce(Result<T, AppError>, &mut App) + 'static) -> Self {
        Self(Box::new(step))
    }
}

/// A job of the user, or the first write of the draft, failed. It holds the answer for the message line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct JobFailed(pub(crate) &'static str);

/// The end of a write of the draft. It comes after the [`DoorEvent`] of the write.
#[derive(Debug)]
pub(crate) enum SaveEvent {
    Saved,
    /// The draft keeps the text, and the next job writes it again.
    Failed,
    /// The core refused the write of the open note after an outside change. The draft holds the text, as a disk
    /// question does.
    Changed,
}

/// The editor text is not on disk after the writes: the last write of the draft failed, or a disk question holds the
/// text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NotSaved;

/// The end of one job, on the UI thread after its work and its event: it gives the result to the caller.
type End = Box<dyn FnOnce(&mut CoreDoor, &mut Context<CoreDoor>)>;

pub(crate) struct CoreDoor {
    core: Core,
    root: PathBuf,
    snapshot: Snapshot,
    /// How long a call waits for its job.
    budget: Duration,
    write_line: WriteLine<End>,
    draft: Draft,
    /// It ends the late job, when its work ends.
    wake: Option<Task<()>>,
    watch: Task<()>,
    _ends: [Subscription; 2],
}

impl CoreDoor {
    /// `core` opened the notes folder at `root`. The write line drains when `window` closes, when the app quits, or
    /// when the entity goes.
    pub(crate) fn new(core: Core, root: PathBuf, budget: Duration, window: &Window, cx: &mut Context<Self>) -> Self {
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
            snapshot,
            write_line: WriteLine::new(core.clone(), changes),
            core,
            root,
            budget,
            draft: Draft::default(),
            wake: None,
            watch: Self::watch(cx),
            _ends: ends,
        }
    }

    /// As the user chose it.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// It is never older than a write that ended.
    pub(crate) fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// Whether a job is past its budget.
    pub(crate) fn writing(&self) -> bool {
        self.write_line.is_late()
    }

    /// The editor text of the note at `path` is now `body`. It writes nothing yet. Text of another note before it goes
    /// in the write line now.
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

    /// A disk question of the note at `path` opened. No job writes its editor text until [`Self::keep_text`] or
    /// [`Self::drop_text`].
    pub(crate) fn hold_text(&mut self, path: &NotePath) {
        self.draft.hold(path);
    }

    /// The user keeps the editor text of the note at `path` over the file. The hold ends, and the draft goes in the
    /// write line now.
    pub(crate) fn keep_text(&mut self, path: &NotePath, cx: &mut Context<Self>) {
        self.draft.release(path);
        self.save_text(cx);
    }

    /// It grows with each new editor text.
    pub(crate) fn draft_version(&self) -> DraftVersion {
        self.draft.version()
    }

    /// Whether the draft has text that is not on disk.
    pub(crate) fn unsaved(&self) -> bool {
        self.draft.unsaved()
    }

    /// Whether the draft has text of the note at `path` that is not on disk.
    pub(crate) fn unsaved_note(&self, path: &NotePath) -> bool {
        self.draft.unsaved_note(path)
    }

    /// Why the draft has text that is not on disk: the reason of a failed write. The reason of the open note wins.
    pub(crate) fn not_saved(&self) -> Option<&'static str> {
        self.draft.not_saved()
    }

    /// Puts the draft in the write line now. It does not wait for the job.
    pub(crate) fn save_text(&mut self, cx: &mut Context<Self>) {
        self.push_draft();
        self.run(None, cx);
    }

    /// Writes the draft, and ends after the writes before it. `done` gets [`NotSaved`] when the write of the draft
    /// failed.
    pub(crate) fn wait_saved(
        &mut self,
        done: impl FnOnce(Result<(), NotSaved>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.push_draft();
        let end: End = Box::new(move |door, cx| {
            let saved = if door.draft.kept_back() { Err(NotSaved) } else { Ok(()) };
            cx.defer(move |cx| done(saved, cx));
        });
        let number = self.write_line.push(Box::new(|_| None), end);
        self.run(Some(number), cx);
    }

    /// After the writes before it.
    pub(crate) fn read(&mut self, path: NotePath, done: Done<Note>, cx: &mut Context<Self>) {
        self.job(move |core| core.read(&path), done, cx);
    }

    /// Reads the note at `path` to replace its editor text. The text drops now with no write, and comes back with its
    /// hold when the read fails.
    pub(crate) fn read_and_drop_text(&mut self, path: NotePath, done: Done<Note>, cx: &mut Context<Self>) {
        let text = self.draft.drop_for_read(&path);
        self.own_job(
            move |core| core.read(&path),
            move |door, note, cx| {
                if let Some(id) = text {
                    door.draft.read_ended(id, &note);
                }
                give(note, done, cx);
            },
            cx,
        );
    }

    /// Ends after the writes before it, so `done` sees their snapshot.
    pub(crate) fn wait(&mut self, done: Done<()>, cx: &mut Context<Self>) {
        self.job(|_| Ok(()), done, cx);
    }

    pub(crate) fn rename_note(&mut self, path: NotePath, title: String, done: Done<NotePath>, cx: &mut Context<Self>) {
        self.job(
            move |core| core.rename_note(&path, &title).map(|info| info.path),
            done,
            cx,
        );
    }

    pub(crate) fn change_tag(&mut self, path: NotePath, change: TagChange, done: Done<()>, cx: &mut Context<Self>) {
        self.job(
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

    pub(crate) fn set_pinned(&mut self, path: NotePath, pinned: bool, done: Done<()>, cx: &mut Context<Self>) {
        self.job(move |core| core.set_pinned(&path, pinned).map(drop), done, cx);
    }

    pub(crate) fn create_note(
        &mut self,
        folder: FolderPath,
        title: String,
        tag: Option<TagName>,
        done: Done<Note>,
        cx: &mut Context<Self>,
    ) {
        self.job(
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

    /// Gives the id of the new note, for a link.
    pub(crate) fn create_linked_note(
        &mut self,
        folder: FolderPath,
        title: String,
        done: Done<String>,
        cx: &mut Context<Self>,
    ) {
        self.job(
            move |core| {
                let note = core.create(&folder, &title)?;
                Ok(core.link_id(&note.info.path)?.to_string())
            },
            done,
            cx,
        );
    }

    /// A note with no id gets one.
    pub(crate) fn link_id(&mut self, path: NotePath, done: Done<String>, cx: &mut Context<Self>) {
        self.job(move |core| core.link_id(&path).map(|id| id.to_string()), done, cx);
    }

    pub(crate) fn move_note(
        &mut self,
        path: NotePath,
        folder: FolderPath,
        done: Done<NotePath>,
        cx: &mut Context<Self>,
    ) {
        self.job(
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
        self.job(move |core| core.move_folder(&path, &folder), done, cx);
    }

    pub(crate) fn rename_folder(
        &mut self,
        folder: FolderPath,
        name: String,
        done: Done<FolderPath>,
        cx: &mut Context<Self>,
    ) {
        self.job(move |core| core.rename_folder(&folder, &name), done, cx);
    }

    pub(crate) fn create_folder(
        &mut self,
        parent: FolderPath,
        name: String,
        done: Done<FolderPath>,
        cx: &mut Context<Self>,
    ) {
        self.job(move |core| core.create_folder(&parent, &name), done, cx);
    }

    pub(crate) fn delete_note(&mut self, path: NotePath, done: Done<()>, cx: &mut Context<Self>) {
        self.job(move |core| core.delete_note(&path), done, cx);
    }

    pub(crate) fn delete_folder(&mut self, path: FolderPath, done: Done<()>, cx: &mut Context<Self>) {
        self.job(move |core| core.delete_folder(&path), done, cx);
    }

    /// After the writes before it.
    pub(crate) fn note_delete_count(&mut self, path: NotePath, done: Done<DeleteCount>, cx: &mut Context<Self>) {
        self.job(move |core| Ok(core.note_delete_count(&path)), done, cx);
    }

    /// After the writes before it.
    pub(crate) fn folder_delete_count(&mut self, path: FolderPath, done: Done<DeleteCount>, cx: &mut Context<Self>) {
        self.job(move |core| Ok(core.folder_delete_count(&path)), done, cx);
    }

    /// Gives the number of changed notes.
    pub(crate) fn rename_tag(&mut self, from: TagName, to: TagName, done: Done<usize>, cx: &mut Context<Self>) {
        self.job(move |core| core.rename_tag(&from, &to), done, cx);
    }

    /// The typed name `to` as a tag, and the number of notes that a rename to it changes, after the writes before it.
    pub(crate) fn tag_rename_count(
        &mut self,
        from: TagName,
        to: String,
        done: Done<(TagName, usize)>,
        cx: &mut Context<Self>,
    ) {
        self.job(
            move |core| {
                let to = TagName::typed(&to)?;
                let count = core.tag_rename_count(&from, &to);
                Ok((to, count))
            },
            done,
            cx,
        );
    }

    /// Gives the number of changed notes.
    pub(crate) fn delete_tag(&mut self, tag: TagName, done: Done<usize>, cx: &mut Context<Self>) {
        self.job(move |core| core.delete_tag(&tag), done, cx);
    }

    /// The number of notes that a tag delete changes, after the writes before it.
    pub(crate) fn tag_delete_count(&mut self, tag: TagName, done: Done<usize>, cx: &mut Context<Self>) {
        self.job(move |core| Ok(core.tag_delete_count(&tag)), done, cx);
    }

    /// Copies pasted or dropped files into `attachments/`, and gives the link to each copy from the note at `note`. A
    /// file with no copy gives `None`.
    pub(crate) fn attach(
        &mut self,
        note: NotePath,
        files: Vec<PastedFile>,
        done: impl FnOnce(Vec<Option<LinkPath>>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.own_job(
            move |core| {
                Ok(files
                    .iter()
                    .map(|file| match file {
                        PastedFile::Path(path) => core.attach_file(&note, path),
                        PastedFile::Image { extension, bytes } => core.paste_image(&note, extension, bytes),
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

    /// The link to each file from the note at `note`, with no copy. A file with no link gives `None`.
    pub(crate) fn file_links(
        &self,
        note: NotePath,
        files: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) -> Task<Vec<Option<LinkPath>>> {
        let job = self.look(
            move |core| files.iter().map(|file| core.file_link(&note, file)).collect(),
            cx,
        );
        cx.spawn(async move |door, cx| {
            let links = job.await;
            door.update(cx, |_, cx| each_answered(links, cx)).unwrap_or_default()
        })
    }

    /// It does not wait for the line. A failed search gives `None`.
    pub(crate) fn search(&self, query: String, order: Order, cx: &mut Context<Self>) -> Task<Option<SearchResults>> {
        let job = self.look(move |core| core.search(&query, order), cx);
        cx.spawn(async move |door, cx| {
            let found = job.await;
            door.update(cx, |_, cx| answered(found, cx).ok()).ok().flatten()
        })
    }

    /// It does not wait for the line. After a failure, it gives none.
    pub(crate) fn headings(&self, path: NotePath, cx: &App) -> Task<Vec<String>> {
        self.look(move |core| logged(core.headings(&path), "cannot read the headings"), cx)
    }

    /// The note links in the note at `path` whose note or heading is gone. It does not wait for the line. After a
    /// failure, it gives none.
    pub(crate) fn broken_links(&self, path: NotePath, cx: &App) -> Task<Vec<LinkTarget>> {
        self.look(
            move |core| logged(core.broken_links(&path), "cannot read the broken links"),
            cx,
        )
    }

    /// Puts `job` in the write line, and waits for it up to the budget. When it ends, the entity sets the snapshot
    /// after it and sends the event. Then `done` gets its result, in the same update. When the entity is gone first,
    /// `done` never runs.
    fn job<T: Send + 'static>(
        &mut self,
        job: impl FnOnce(&Core) -> holm_core::Result<T> + Send + 'static,
        done: Done<T>,
        cx: &mut Context<Self>,
    ) {
        self.own_job(job, |_, result, cx| give(result, done, cx), cx);
    }

    /// Like [`Self::job`], but `end` gets the result on the entity, before the deferred steps.
    fn own_job<T: Send + 'static>(
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

    /// Puts `save` in the write line. It carries the revision of the snapshot now, so the core refuses it after an
    /// outside change that the door has not taken yet.
    fn push_save(&mut self, save: DraftSave) {
        let (send, mut receive) = oneshot::channel();
        let seen = self.snapshot.revision();
        let work: Work = Box::new(move |core: &Core| {
            let failure = save.text.write(core, Some(seen)).err().map(AppError::from);
            let chain = failure.as_ref().map(|error| crate::chain(error));
            send.send(failure).ok();
            chain
        });
        let (id, version) = (save.id, save.version);
        let end: End = Box::new(move |door, cx| {
            let Ok(Some(failure)) = receive.try_recv() else {
                return;
            };
            if failure.as_ref().is_some_and(AppError::is_outside_change) {
                tracing::info!("a save found an outside change");
                if door.draft.refused(id, version) == Written::Changed {
                    cx.emit(SaveEvent::Changed);
                }
                return;
            }
            let reason = failure.as_ref().map(AppError::message);
            match (door.draft.written(id, version, reason), failure) {
                (Written::Saved, _) => cx.emit(SaveEvent::Saved),
                (Written::Failed { first }, Some(error)) => {
                    if first {
                        answer(&error, cx);
                    } else {
                        tracing::warn!(error = %crate::chain(&error), "a save failed again");
                    }
                    cx.emit(SaveEvent::Failed);
                }
                _ => {}
            }
        });
        self.write_line.push(work, end);
    }

    fn push<T: Send + 'static>(
        &mut self,
        job: impl FnOnce(&Core) -> holm_core::Result<T> + Send + 'static,
        end: impl FnOnce(&mut Self, holm_core::Result<T>, &mut Context<Self>) + 'static,
    ) -> JobNumber {
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
    fn look<T: Send + 'static>(&self, job: impl FnOnce(&Core) -> T + Send + 'static, cx: &App) -> Task<T> {
        let core = self.core.clone();
        cx.background_spawn(async move { job(&core) })
    }

    /// Runs the jobs of the write line. With `until`, it waits on this thread up to the budget, until the job `until`
    /// ends. A job that does not end in time ends later, in its own update.
    fn run(&mut self, until: Option<JobNumber>, cx: &mut Context<Self>) {
        let writing = self.writing();
        let run = self.write_line.run(until, self.budget, cx);
        if let Some(wake) = run.wake {
            self.wake = Some(cx.spawn(async move |door, cx| {
                wake.await;
                door.update(cx, |door, cx| door.run(None, cx)).ok();
            }));
        }
        if !run.ended.is_empty() {
            let event = self.keep(run.lists);
            cx.emit(event);
            for end in run.ended {
                end(self, cx);
            }
            self.watch = Self::watch(cx);
        }
        // The observers hear of a change of `Self::writing` after the ends of the jobs, so they run after the
        // deferred steps of the ends. A step can ask for a new late job, and the observers never see the short state
        // between the jobs.
        if self.writing() != writing {
            cx.notify();
        }
    }

    /// Runs every job that waits, and then writes the draft. Later jobs do not run.
    fn drain(&mut self) {
        let texts = self.draft.take();
        let draft: Work = Box::new(move |core: &Core| {
            let failures: Vec<String> = texts
                .into_iter()
                .filter_map(|text| text.write(core, None).err())
                .map(|error| crate::chain(&error))
                .collect();
            (!failures.is_empty()).then(|| failures.join("; "))
        });
        self.write_line.drain(draft);
    }

    /// Wakes on each change list while no job runs. A job ends it, and [`Self::run`] starts a new one after the jobs
    /// that ended.
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
        let Some(lists) = self.write_line.outside(waker) else {
            return Poll::Ready(());
        };
        if !lists.is_empty() {
            let event = self.keep(lists);
            cx.emit(event);
        }
        Poll::Pending
    }

    /// Keeps the snapshot of the last list, and gives the event with the changes of all lists.
    fn keep(&mut self, lists: Vec<ChangeList>) -> DoorEvent {
        let mut changes = Vec::new();
        for list in lists {
            for change in &list.changes {
                self.draft.follow(list.source, change, &self.snapshot);
            }
            self.snapshot = list.snapshot;
            changes.extend(list.changes.into_iter().map(|change| (list.source, change)));
        }
        DoorEvent { changes }
    }
}

impl EventEmitter<DoorEvent> for CoreDoor {}

impl EventEmitter<SaveEvent> for CoreDoor {}

impl EventEmitter<JobFailed> for CoreDoor {}

/// Gives the result with the app error. A failure goes to the log, and its answer to [`JobFailed`].
fn answered<T>(result: holm_core::Result<T>, cx: &mut Context<CoreDoor>) -> Result<T, AppError> {
    result.map_err(|error| {
        let error = AppError::from(error);
        answer(&error, cx);
        error
    })
}

/// The value of a read of Holm's own work. A failure goes to the log, and gives no value.
fn logged<T: Default>(result: holm_core::Result<T>, what: &'static str) -> T {
    result.unwrap_or_else(|error| {
        tracing::warn!(error = %crate::chain(&AppError::from(error)), "{what}");
        T::default()
    })
}

fn answer(error: &AppError, cx: &mut Context<CoreDoor>) {
    tracing::warn!(error = %crate::chain(error), "a job failed");
    cx.emit(JobFailed(error.message()));
}

/// Gives the end of a job to its caller, after [`answered`].
fn give<T: 'static>(result: holm_core::Result<T>, done: Done<T>, cx: &mut Context<CoreDoor>) {
    let result = answered(result, cx);
    // A deferred step runs after the views took the event, in the same update.
    cx.defer(move |cx| (done.0)(result, cx));
}

/// The value of each result, or `None` after its failure went to [`answered`].
fn each_answered<T>(results: Vec<holm_core::Result<T>>, cx: &mut Context<CoreDoor>) -> Vec<Option<T>> {
    results.into_iter().map(|result| answered(result, cx).ok()).collect()
}

/// What [`open`] does when the notes folder is missing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MissingFolder {
    /// Makes the folder first.
    Make,
    /// Fails to open.
    Fail,
}

pub(crate) fn open(
    open: &dyn Fn(&Path, &Path) -> holm_core::Result<Core>,
    folder: &Path,
    index_dir: &Path,
    missing: MissingFolder,
    last: Option<&NotePath>,
    order: Order,
) -> Result<(Core, Option<Note>), AppError> {
    if missing == MissingFolder::Make
        && let Err(error) = std::fs::create_dir_all(folder)
    {
        tracing::warn!(path = %folder.display(), %error, "cannot make the notes folder");
    }
    let core = open(folder, index_dir)?;
    let note = first_note(&core, last, order)?;
    Ok((core, note))
}

/// The note to open first: the last open note when it still exists, else the first note in `order`. With no notes, it
/// is `None`.
fn first_note(core: &Core, last: Option<&NotePath>, order: Order) -> holm_core::Result<Option<Note>> {
    let snapshot = core.snapshot();
    snapshot
        .note_or_first(last, order)
        .map(|info| core.read(&info.path))
        .transpose()
}

#[cfg(test)]
mod tests;
