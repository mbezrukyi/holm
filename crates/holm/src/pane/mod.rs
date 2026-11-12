//! The editor pane: one note in the editor, with autosave. With no open note, it shows the
//! empty pane.
//!
//! The pane saves a short time after the user stops typing, and when the app quits.
//! An outside change of the note reloads it quietly when the pane has no unsaved
//! changes. Else a quiet bar asks to reload or to keep the text in the editor.
//! When a save fails, a quiet bar shows the error, and the text stays in the editor.
//! When another tool deletes the note, the app opens another one. With unsaved changes, a quiet
//! bar first asks to write the note again or to close it.
//!
//! When the user opens another note, the pane saves the note and renames the file to match
//! the title. When the window closes, the pane only saves the note.
//!
//! The tag row before the first line shows the tags and the pin of the note. It scrolls with
//! the text. The pane writes their changes at once. The editor undo stack holds only the text,
//! so Ctrl+Z does not undo them.
//!
//! The pane runs the link picker flow. `links` opens the links of the note and reads its
//! backlinks. The backlinks show in the tag row.

mod links;

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::{Theme as Ui, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement, Render, Role,
    StatefulInteractiveElement as _, Styled, Subscription, Task, TestSupportExt as _, Window, div,
    px,
};
use holm_core::Core;
use holm_core::model::{Change, Note, NoteInfo, Order};
use holm_editor::{Changed, Editor, UpFromStart};
use holm_markdown::Target;

use crate::backlinks::{Backlinks, OpenBacklink};
use crate::bar::{self, Bar};
use crate::error::AppError;
use crate::link_picker::{LinkChoice, LinkStart};
use crate::tags::{TagRow, TagsChanged};
use crate::{changes, text, theme};
use links::{Links, LinksEvent};

/// How long the pane waits after the last change before it saves.
const AUTOSAVE: Duration = Duration::from_millis(500);

/// What the pane tells the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PaneEvent {
    /// The pane shows the note at this path.
    Opened(PathBuf),
    /// The pane shows no note.
    Closed,
    /// Another tool deleted the open note at this path, and the pane has no text to keep.
    /// The app must open another note or call [`NotePane::close`]. The pane cannot pick the
    /// note, because the neighbor comes from the sidebar list.
    Gone(PathBuf),
}

/// What happens to the open note when the pane opens another one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Leave {
    /// Save it and rename its file to match the title.
    Keep,
    /// The note is gone. Drop its text.
    Drop,
}

pub(crate) struct NotePane {
    core: Core,
    notes_folder: PathBuf,
    editor: Entity<Editor>,
    tag_row: Entity<TagRow>,
    backlinks: Entity<Backlinks>,
    links: Entity<Links>,
    /// The note as it is on disk, after the last read or save. Its tags and pin can be newer:
    /// a write of them may still run. With no note, the pane is empty.
    note: Option<Note>,
    /// Grows with each change of the tags or the pin. A read of the fields that started
    /// before a change does not undo it.
    fields_version: u64,
    /// The last write of the tags or the pin. The next write waits for it.
    writing: Option<Task<()>>,
    ending: LineEnding,
    /// The editor text that matches the file on disk.
    saved: String,
    autosave: Option<Task<()>>,
    saving: Option<Task<()>>,
    /// The note changed on disk while the editor had unsaved changes.
    conflict: bool,
    /// Another tool deleted the note while the editor had unsaved changes.
    deleted: bool,
    error: Option<&'static str>,
    opening: Option<Task<()>>,
    /// The heading to show when the note that opens is ready.
    open_heading: Option<String>,
    /// Whether the link picker changes the link at the cursor, not inserts a new one.
    replace_link: bool,
    _changes: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl NotePane {
    pub(crate) fn new(
        core: Core,
        notes_folder: PathBuf,
        note: Option<Note>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(Editor::new);
        let text = editor.focus_handle(cx);
        let backlinks = cx.new(|cx| Backlinks::new(text.clone(), window, cx));
        let tag_row = cx.new(|cx| TagRow::new(core.clone(), backlinks.clone(), text, window, cx));
        editor.update(cx, |editor, cx| {
            editor.set_start(Some(tag_row.clone().into()), cx);
        });
        let links = cx.new(|cx| Links::new(core.clone(), editor.clone(), backlinks.clone(), cx));
        let changes = changes::watch(&core, cx, Self::outside_change);
        let subscriptions = vec![
            cx.subscribe(&editor, |pane, _, _: &Changed, cx| pane.schedule_save(cx)),
            cx.subscribe_in(&editor, window, |pane, _, _: &UpFromStart, window, cx| {
                pane.focus_tags(window, cx);
            }),
            cx.subscribe(&tag_row, |pane, _, TagsChanged(tags), cx| {
                pane.set_tags(tags, cx);
            }),
            cx.subscribe(&links, Self::links_event),
            cx.subscribe_in(
                &backlinks,
                window,
                |pane, _, OpenBacklink(path), window, cx| {
                    window.focus(&pane.editor.focus_handle(cx), cx);
                    pane.open(path.clone(), Leave::Keep, cx);
                },
            ),
            cx.observe_global::<Ui>(|pane, cx| {
                let appearance = theme::appearance(cx);
                pane.editor
                    .update(cx, |editor, cx| editor.set_appearance(appearance, cx));
            }),
            cx.on_release(Self::released),
        ];
        let appearance = theme::appearance(cx);
        editor.update(cx, |editor, cx| editor.set_appearance(appearance, cx));
        let mut pane = Self {
            core,
            notes_folder,
            editor,
            tag_row,
            backlinks,
            links,
            note: None,
            fields_version: 0,
            writing: None,
            ending: LineEnding::Lf,
            saved: String::new(),
            autosave: None,
            saving: None,
            conflict: false,
            deleted: false,
            error: None,
            opening: None,
            open_heading: None,
            replace_link: false,
            _changes: changes,
            _subscriptions: subscriptions,
        };
        if let Some(note) = note {
            pane.show(note, false, cx);
        }
        pane
    }

    /// Puts `note` from the disk into the editor. A reload keeps the view and is one undo step.
    fn show(&mut self, note: Note, reload: bool, cx: &mut Context<Self>) {
        self.ending = LineEnding::of(&note.body);
        let folder = note
            .info
            .path
            .parent()
            .map(|parent| self.notes_folder.join(parent));
        self.saved = self.editor.update(cx, |editor, cx| {
            if reload {
                editor.reload_text(&note.body, cx);
            } else {
                editor.set_text(&note.body, cx);
                editor.set_folder(folder, cx);
            }
            editor.text().to_owned()
        });
        let path = note.info.path.clone();
        self.note = Some(note);
        self.fields_version += 1;
        self.show_fields(cx);
        self.conflict = false;
        self.deleted = false;
        self.error = None;
        if !reload && let Some(heading) = self.open_heading.take() {
            self.editor
                .update(cx, |editor, cx| editor.show_heading(&heading, cx));
        }
        self.links
            .update(cx, |links, cx| links.show(Some(path), cx));
        cx.notify();
    }

    /// Drops the open note and shows the empty pane.
    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        self.drop_note(cx);
        cx.emit(PaneEvent::Closed);
    }

    /// Drops the open note and its text, with no save.
    fn drop_note(&mut self, cx: &mut Context<Self>) {
        self.note = None;
        self.autosave = None;
        self.saving = None;
        self.writing = None;
        self.opening = None;
        self.conflict = false;
        self.deleted = false;
        self.error = None;
        self.editor.update(cx, |editor, cx| editor.set_text("", cx));
        self.saved = String::new();
        self.links.update(cx, |links, cx| links.show(None, cx));
        cx.notify();
    }

    fn links_event(&mut self, _: Entity<Links>, event: &LinksEvent, cx: &mut Context<Self>) {
        match event {
            LinksEvent::Open { path, heading } => {
                self.open_heading.clone_from(heading);
                self.open(path.clone(), Leave::Keep, cx);
            }
            LinksEvent::Error(message) => {
                self.error = Some(message);
                cx.notify();
            }
        }
    }

    /// Opens the system file picker, and embeds each picked file at the cursor.
    pub(crate) fn link_files(&mut self, cx: &mut Context<Self>) {
        if self.note.is_none() {
            return;
        }
        self.links.update(cx, Links::pick_files);
    }

    /// Ctrl+K: reads what the link picker starts with. In a note link, the field starts with
    /// its target, and the pick changes that link. With no open note, it gives no task.
    pub(crate) fn start_link(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<Task<holm_core::Result<LinkStart>>> {
        let open_note = self.path()?.to_owned();
        let link = self.editor.read(cx).link_at_cursor();
        self.replace_link = link.as_ref().is_some_and(|link| !link.embedded);
        let target = link.map(|link| link.target);
        Some(self.run(
            move |core| {
                let mut notes = core.notes(Order::LastChange)?;
                notes.sort_by_key(|note| std::cmp::Reverse(note.modified));
                let query = match target {
                    Some(Target::Note { id, heading }) => match core.note_with_id(&id)? {
                        Some(note) => match heading {
                            Some(heading) => format!("{}#{heading}", note.title),
                            None => note.title,
                        },
                        None => String::new(),
                    },
                    Some(Target::Heading(heading)) => format!("#{heading}"),
                    _ => String::new(),
                };
                Ok(LinkStart {
                    notes,
                    open_note,
                    query,
                })
            },
            cx,
        ))
    }

    /// Makes the link that the user picked in the link picker, and puts it at the cursor.
    /// A new note goes in the folder of the open note. With no open note, it does nothing.
    pub(crate) fn link(
        &mut self,
        choice: LinkChoice,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<()>> {
        let Some(path) = self.path() else {
            return Task::ready(Ok(()));
        };
        let folder = path.parent().map(Path::to_owned).unwrap_or_default();
        let make = self.run(
            move |core| match choice {
                LinkChoice::Note {
                    path,
                    title,
                    heading,
                } => {
                    let id = core.link_id(&path)?.to_string();
                    let text = heading.clone().unwrap_or(title);
                    Ok((Target::Note { id, heading }, text))
                }
                LinkChoice::SameNote(heading) => Ok((Target::Heading(heading.clone()), heading)),
                LinkChoice::Create(title) => {
                    let mut note = core.create(&folder)?;
                    note.body = format!("# {title}\n");
                    let note = core.save(&note)?;
                    let info = core.rename_to_title(&note.info.path)?;
                    let id = core.link_id(&info.path)?.to_string();
                    Ok((Target::Note { id, heading: None }, title))
                }
            },
            cx,
        );
        cx.spawn(async move |pane, cx| {
            let (target, text) = make.await?;
            pane.update(cx, |pane, cx| pane.put_link(&target, &text, cx))
                .ok();
            Ok(())
        })
    }

    /// Puts a link to `target` at the cursor, with the selection or `text` as its text.
    /// When the picker started in a link, it points that link to `target` and keeps its text.
    fn put_link(&mut self, target: &Target, text: &str, cx: &mut Context<Self>) {
        if self.note.is_none() {
            return;
        }
        let replace = self.replace_link;
        self.editor.update(cx, |editor, cx| {
            if !(replace && editor.set_link_target(target, cx)) {
                editor.insert_link(text, target, false, cx);
            }
        });
    }

    fn show_fields(&mut self, cx: &mut Context<Self>) {
        let Some(note) = &self.note else {
            return;
        };
        let info = &note.info;
        self.tag_row
            .update(cx, |row, cx| row.show(&info.tags, info.pinned, cx));
    }

    fn set_tags(&mut self, tags: &[String], cx: &mut Context<Self>) {
        let Some(note) = &mut self.note else {
            return;
        };
        note.info.tags = tags.to_vec();
        let (path, tags) = (note.info.path.clone(), tags.to_vec());
        self.write_fields(move |core| core.set_tags(&path, &tags), cx);
    }

    /// Opens the tag popup. The editor scrolls to it.
    pub(crate) fn focus_tags(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.note.is_none() {
            return;
        }
        self.tag_row.update(cx, |row, cx| row.enter(window, cx));
    }

    /// Ctrl+Shift+B: opens the backlinks list. The editor scrolls to it.
    pub(crate) fn show_backlinks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.note.is_none() {
            return;
        }
        self.backlinks
            .update(cx, |backlinks, cx| backlinks.enter(window, cx));
    }

    /// Pins or unpins the open note.
    pub(crate) fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        let Some(note) = &mut self.note else {
            return;
        };
        let pinned = !note.info.pinned;
        note.info.pinned = pinned;
        let path = note.info.path.clone();
        self.show_fields(cx);
        self.write_fields(move |core| core.set_pinned(&path, pinned), cx);
    }

    /// Writes a change of the tags or the pin, after the writes before it.
    fn write_fields(
        &mut self,
        job: impl FnOnce(&Core) -> holm_core::Result<NoteInfo> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        self.fields_version += 1;
        let before = self.writing.take();
        self.writing = Some(cx.spawn(async move |pane, cx| {
            if let Some(before) = before {
                before.await;
            }
            let Ok(run) = pane.update(cx, |pane, cx| pane.run(job, cx)) else {
                return;
            };
            if let Err(error) = run.await {
                pane.update(cx, |pane, cx| {
                    let error = AppError::from(error);
                    tracing::warn!(error = %crate::chain(&error), "cannot change the tags or the pin");
                    pane.error = Some(error.message());
                    cx.notify();
                })
                .ok();
            }
        }));
    }

    /// Reads the tags and the pin of the open note again, after a file action.
    /// It keeps a change of them that came while it read.
    fn read_fields(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let Some(path) = self.path().map(Path::to_owned) else {
            return Task::ready(());
        };
        let core = self.core.clone();
        let version = self.fields_version;
        let job = cx.background_spawn(async move { core.read(&path) });
        cx.spawn(async move |pane, cx| {
            let Ok(note) = job.await else {
                return;
            };
            pane.update(cx, |pane, cx| {
                if pane.fields_version == version
                    && let Some(open) = &mut pane.note
                    && open.info.path == note.info.path
                {
                    open.info = note.info;
                    pane.show_fields(cx);
                }
            })
            .ok();
        })
    }

    fn is_dirty(&self, cx: &App) -> bool {
        self.editor.read(cx).text() != self.saved
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.autosave = Some(cx.spawn(async move |pane, cx| {
            cx.background_executor().timer(AUTOSAVE).await;
            pane.update(cx, |pane, cx| pane.save(false, cx)).ok();
        }));
    }

    /// Saves the editor text. With no `overwrite`, it saves only unsaved changes with no conflict.
    fn save(&mut self, overwrite: bool, cx: &mut Context<Self>) {
        if self.saving.is_none() {
            self.saving = self.save_task(overwrite, cx);
        }
    }

    fn save_task(&mut self, overwrite: bool, cx: &mut Context<Self>) -> Option<Task<()>> {
        if !overwrite && (self.conflict || self.deleted || !self.is_dirty(cx)) {
            return None;
        }
        self.write_task(Core::save, cx)
    }

    /// Writes unsaved text at once. It wins over an outside change, and writes the note again
    /// after an outside delete.
    fn write_now(&mut self, cx: &mut Context<Self>) -> Option<Task<()>> {
        if !self.is_dirty(cx) {
            return None;
        }
        if self.deleted {
            self.write_task(Core::restore, cx)
        } else {
            self.write_task(Core::save, cx)
        }
    }

    /// Writes the editor text to the open note with `write`.
    fn write_task(
        &mut self,
        write: fn(&Core, &Note) -> holm_core::Result<Note>,
        cx: &mut Context<Self>,
    ) -> Option<Task<()>> {
        let mut note = self.note.clone()?;
        let text = self.editor.read(cx).text().to_owned();
        note.body = self.ending.to_disk(&text).into_owned();
        let core = self.core.clone();
        let job = cx.background_spawn(async move { write(&core, &note) });
        Some(cx.spawn(async move |pane, cx| {
            let result = job.await;
            pane.update(cx, |pane, cx| pane.saved(text, result, cx))
                .ok();
        }))
    }

    fn saved(&mut self, text: String, result: holm_core::Result<Note>, cx: &mut Context<Self>) {
        self.saving = None;
        match result {
            Ok(note) => {
                self.note = Some(note);
                self.saved = text;
                self.conflict = false;
                self.deleted = false;
                self.error = None;
                if self.is_dirty(cx) {
                    self.schedule_save(cx);
                }
            }
            Err(error) => {
                let error = AppError::from(error);
                tracing::warn!(error = %crate::chain(&error), "cannot save the note");
                self.error = Some(error.message());
            }
        }
        cx.notify();
    }

    fn outside_change(&mut self, change: Change, cx: &mut Context<Self>) {
        let Some(open) = &mut self.note else {
            return;
        };
        let unsaved = self.editor.read(cx).text() != self.saved || self.saving.is_some();
        match change {
            Change::Changed(info) if info.path == open.info.path => {
                self.deleted = false;
                if unsaved {
                    self.conflict = true;
                    cx.notify();
                } else {
                    self.reload(cx);
                }
            }
            Change::Moved { from, note } if from == open.info.path => {
                open.info.path.clone_from(&note.path);
                self.links
                    .update(cx, |links, cx| links.show(Some(note.path), cx));
            }
            Change::Removed(path) if path == open.info.path => {
                if unsaved {
                    self.conflict = false;
                    self.deleted = true;
                    self.autosave = None;
                    cx.notify();
                } else {
                    cx.emit(PaneEvent::Gone(path));
                }
            }
            _ => {}
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.path().map(Path::to_owned) else {
            return;
        };
        let core = self.core.clone();
        let job = cx.background_spawn(async move { core.read(path) });
        cx.spawn(async move |pane, cx| {
            let result = job.await;
            pane.update(cx, |pane, cx| match result {
                Ok(note) => pane.show(note, true, cx),
                Err(error) => {
                    let error = AppError::from(error);
                    tracing::warn!(error = %crate::chain(&error), "cannot reload the note");
                    pane.error = Some(error.message());
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn keep_mine(&mut self, cx: &mut Context<Self>) {
        self.conflict = false;
        self.autosave = None;
        self.save(true, cx);
        cx.notify();
    }

    /// "Keep mine" after an outside delete: writes the note again at its path.
    fn keep_deleted(&mut self, cx: &mut Context<Self>) {
        self.autosave = None;
        self.saving = self.write_task(Core::restore, cx);
        cx.notify();
    }

    /// "Close" after an outside delete: drops the note and its text, and the app opens another
    /// note.
    fn close_deleted(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.path().map(Path::to_owned) else {
            return;
        };
        self.drop_note(cx);
        cx.emit(PaneEvent::Gone(path));
    }

    /// When the window closes or the app quits: writes unsaved text. Unsaved text wins over
    /// an outside change. It works on the UI thread, because no task runs after the window
    /// is gone.
    fn released(&mut self, cx: &mut App) {
        if !self.is_dirty(cx) {
            return;
        }
        let Some(mut note) = self.note.clone() else {
            return;
        };
        let write = if self.deleted {
            Core::restore
        } else {
            Core::save
        };
        note.body = self
            .ending
            .to_disk(self.editor.read(cx).text())
            .into_owned();
        if let Err(error) = write(&self.core, &note) {
            let error = AppError::from(error);
            tracing::warn!(error = %crate::chain(&error), "cannot save the note");
        }
    }

    /// The path of the open note, relative to the notes folder.
    pub(crate) fn path(&self) -> Option<&Path> {
        self.note.as_ref().map(|note| note.info.path.as_path())
    }

    /// Saves unsaved text now. Like at quit, the text wins over an outside change.
    /// The task ends when the file is written, or when the save failed.
    fn flush(&mut self, cx: &mut Context<Self>) -> Task<()> {
        self.autosave = None;
        let running = self.saving.take();
        cx.spawn(async move |pane, cx| {
            if let Some(running) = running {
                running.await;
            }
            let save = pane.update(cx, Self::write_now);
            if let Ok(Some(save)) = save {
                save.await;
            }
        })
    }

    /// Opens the note at `path`. With [`Leave::Keep`], the pane first saves the open note and
    /// renames its file to match the title. When that save fails, the pane stays on the note.
    pub(crate) fn open(&mut self, path: PathBuf, leave: Leave, cx: &mut Context<Self>) {
        if leave == Leave::Keep && self.path() == Some(path.as_path()) {
            return;
        }
        let old = self
            .path()
            .filter(|_| leave == Leave::Keep)
            .map(Path::to_owned);
        let flush = old.is_some().then(|| self.flush(cx));
        self.opening = Some(cx.spawn(async move |pane, cx| {
            if let Some(flush) = flush {
                flush.await;
                let saved = pane.update(cx, |pane, cx| !pane.is_dirty(cx));
                if !matches!(saved, Ok(true)) {
                    return;
                }
            }
            let Ok(core) = pane.read_with(cx, |pane, _| pane.core.clone()) else {
                return;
            };
            let job = cx.background_spawn(async move {
                let renamed = old.map(|old| core.rename_to_title(&old));
                (renamed, core.read(&path))
            });
            let (renamed, read) = job.await;
            pane.update(cx, |pane, cx| pane.opened(renamed, read, cx))
                .ok();
        }));
    }

    fn opened(
        &mut self,
        renamed: Option<holm_core::Result<holm_core::model::NoteInfo>>,
        read: holm_core::Result<Note>,
        cx: &mut Context<Self>,
    ) {
        self.opening = None;
        if let Some(Err(error)) = renamed {
            let error = AppError::from(error);
            tracing::warn!(error = %crate::chain(&error), "cannot rename the note to its title");
        }
        match read {
            Ok(note) => {
                let path = note.info.path.clone();
                self.show(note, false, cx);
                cx.emit(PaneEvent::Opened(path));
            }
            Err(error) => {
                let error = AppError::from(error);
                tracing::warn!(error = %crate::chain(&error), "cannot open the note");
                self.error = Some(error.message());
                cx.notify();
            }
        }
    }

    /// Runs a file action off the UI thread, after the pane saves the open note. So a late
    /// save never writes a file back after the action moved or deleted it. After the action,
    /// the pane reads the tags and the pin of the open note again, so its next save keeps them.
    pub(crate) fn run<T: Send + 'static>(
        &mut self,
        job: impl FnOnce(&Core) -> holm_core::Result<T> + Send + 'static,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<T>> {
        let flush = self.flush(cx);
        let core = self.core.clone();
        cx.spawn(async move |pane, cx| {
            flush.await;
            let result = cx.background_spawn(async move { job(&core) }).await;
            if let Ok(read) = pane.update(cx, Self::read_fields) {
                read.await;
            }
            result
        })
    }

    /// Like [`Self::run`], for an action that moves the note or folder at `from` and gives its
    /// new path. When that holds the open note, the pane follows it.
    pub(crate) fn run_move(
        &mut self,
        from: PathBuf,
        job: impl FnOnce(&Core) -> holm_core::Result<PathBuf> + Send + 'static,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<PathBuf>> {
        let run = self.run(job, cx);
        cx.spawn(async move |pane, cx| {
            let result = run.await;
            if let Ok(to) = &result {
                pane.update(cx, |pane, cx| pane.moved(&from, to, cx)).ok();
            }
            result
        })
    }

    /// Follows a move of the open note, or of a folder that holds it, and reads it again.
    /// A move can change the file links in the note.
    fn moved(&mut self, from: &Path, to: &Path, cx: &mut Context<Self>) {
        let Some(open) = &mut self.note else {
            return;
        };
        let Ok(rest) = open.info.path.strip_prefix(from) else {
            return;
        };
        let path = if rest.as_os_str().is_empty() {
            to.to_owned()
        } else {
            to.join(rest)
        };
        open.info.path.clone_from(&path);
        let folder = path.parent().map(|parent| self.notes_folder.join(parent));
        self.editor
            .update(cx, |editor, cx| editor.set_folder(folder, cx));
        self.reload(cx);
        cx.emit(PaneEvent::Opened(path));
    }

    /// Selects the title text in the editor and gives the editor the focus.
    pub(crate) fn select_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.note.is_none() {
            return;
        }
        self.editor.update(cx, Editor::select_title);
        window.focus(&self.editor.focus_handle(cx), cx);
    }
}

impl EventEmitter<PaneEvent> for NotePane {}

impl Focusable for NotePane {
    fn focus_handle(&self, cx: &gpui_kit::App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Render for NotePane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let conflict = self.conflict.then(|| {
            Bar::new("changed-on-disk", text::CHANGED_ON_DISK)
                .button(
                    bar::button("reload", text::RELOAD)
                        .on_click(cx.listener(|pane, _, _, cx| pane.reload(cx))),
                )
                .button(
                    bar::button("keep-mine", text::KEEP_MINE)
                        .on_click(cx.listener(|pane, _, _, cx| pane.keep_mine(cx))),
                )
        });
        let deleted = self.deleted.then(|| {
            Bar::new("deleted-on-disk", text::DELETED_ON_DISK)
                .button(
                    bar::button("keep-deleted", text::KEEP_MINE)
                        .on_click(cx.listener(|pane, _, _, cx| pane.keep_deleted(cx))),
                )
                .button(
                    bar::button("close-deleted", text::CLOSE)
                        .on_click(cx.listener(|pane, _, _, cx| pane.close_deleted(cx))),
                )
        });
        let error = self.error.map(|message| Bar::new("save-error", message));
        let body = if self.note.is_some() {
            div()
                .flex_1()
                .min_h_0()
                .font_family(theme::NOTE_FONT)
                .text_size(theme::NOTE_SIZE)
                .line_height(theme::NOTE_LINE)
                .child(self.editor.clone())
                .into_any_element()
        } else {
            empty_pane(&self.editor.focus_handle(cx), cx).into_any_element()
        };
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .children(conflict)
            .children(deleted)
            .children(error)
            .child(body)
    }
}

/// The pane with no open note: a line, and the keys that open one. It holds the focus of the
/// editor, so the window keys still work when no other part has the focus.
fn empty_pane(focus: &FocusHandle, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let chip = |key: &'static str| {
        div()
            .px(px(7.))
            .py(px(2.))
            .text_xs()
            .border_1()
            .border_color(theme.border)
            .rounded(px(4.))
            .bg(theme.sidebar)
            .child(key)
    };
    let row = |keys: &[&'static str], label: &'static str| {
        [
            h_flex()
                .justify_end()
                .gap_1()
                .children(keys.iter().map(|key| chip(key)))
                .into_any_element(),
            div()
                .whitespace_nowrap()
                .text_color(theme.muted_foreground)
                .child(label)
                .into_any_element(),
        ]
    };
    v_flex()
        .id("empty-pane")
        .role(Role::Group)
        .aria_label(text::NO_NOTE_OPEN)
        .test_support()
        .track_focus(focus)
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .items_center()
        .justify_center()
        .gap_5()
        .text_sm()
        .child(
            div()
                .whitespace_nowrap()
                .text_size(theme::EMPTY_LINE)
                .font_weight(FontWeight::MEDIUM)
                .child(text::NO_NOTE_OPEN),
        )
        .child(
            div()
                .grid()
                .grid_cols(2)
                .gap_x_4()
                .gap_y(px(10.))
                .items_center()
                .children(row(&["Ctrl", "N"], text::NEW_NOTE))
                .children(row(&["Ctrl", "Shift", "O"], text::QUICK_OPEN)),
        )
}

/// The line break of a note. The editor works with LF only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    /// The line break of most lines in `body`.
    fn of(body: &str) -> Self {
        let crlf = body.matches("\r\n").count();
        if crlf > 0 && crlf * 2 >= body.matches('\n').count() {
            Self::CrLf
        } else {
            Self::Lf
        }
    }

    /// `text` from the editor with this line break.
    fn to_disk(self, text: &str) -> Cow<'_, str> {
        match self {
            Self::Lf => Cow::Borrowed(text),
            Self::CrLf => Cow::Owned(text.replace('\n', "\r\n")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_keeps_its_line_breaks() {
        assert_eq!(LineEnding::of("a\nb\n"), LineEnding::Lf);
        assert_eq!(LineEnding::of("a\r\nb\r\n"), LineEnding::CrLf);
        assert_eq!(LineEnding::of("a\r\nb\nc\n"), LineEnding::Lf);
        assert_eq!(LineEnding::of(""), LineEnding::Lf);
        assert_eq!(LineEnding::CrLf.to_disk("a\nb\n"), "a\r\nb\r\n");
        assert_eq!(LineEnding::Lf.to_disk("a\nb\n"), "a\nb\n");
    }
}
