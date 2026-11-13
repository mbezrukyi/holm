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
use holm_core::model::{Change, Note, NoteInfo, Order};
use holm_editor::{Changed, Editor, UpFromStart};
use holm_markdown::Target;

use crate::backlinks::{Backlinks, OpenBacklink};
use crate::bar::{self, Bar};
use crate::error::AppError;
use crate::link_picker::{LinkChoice, LinkStart};
use crate::notes_folder::{FolderEvent, NoteText, NotesFolder};
use crate::tags::{TagChange, TagRow};
use crate::{text, theme};
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
    notes_folder: Entity<NotesFolder>,
    /// The notes folder on disk, for the file links of the editor.
    root: PathBuf,
    editor: Entity<Editor>,
    tag_row: Entity<TagRow>,
    backlinks: Entity<Backlinks>,
    links: Entity<Links>,
    /// The path of the open note. With no note, the pane is empty.
    path: Option<PathBuf>,
    /// The facts of the open note in the last snapshot that had it. A restore after an
    /// outside delete writes them again.
    last_info: Option<NoteInfo>,
    /// The writes of the tags or the pin that did not end yet. Until they end, the tag row
    /// shows its own tags and pin, not the ones of the snapshot.
    fields_pending: usize,
    ending: LineEnding,
    /// The editor text that matches the file on disk.
    saved: String,
    autosave: Option<Task<()>>,
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
    _subscriptions: Vec<Subscription>,
}

impl NotePane {
    pub(crate) fn new(
        notes_folder: Entity<NotesFolder>,
        root: PathBuf,
        note: Option<Note>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(Editor::new);
        let text = editor.focus_handle(cx);
        let backlinks = cx.new(|cx| Backlinks::new(text.clone(), window, cx));
        let tag_row =
            cx.new(|cx| TagRow::new(notes_folder.clone(), backlinks.clone(), text, window, cx));
        editor.update(cx, |editor, cx| {
            editor.set_start(Some(tag_row.clone().into()), cx);
        });
        let links =
            cx.new(|cx| Links::new(notes_folder.clone(), editor.clone(), backlinks.clone(), cx));
        let subscriptions = vec![
            cx.subscribe(&notes_folder, |pane, _, event, cx| match event {
                FolderEvent::Changed => pane.show_fields(cx),
                FolderEvent::Outside(change) => pane.outside_change(change.clone(), cx),
            }),
            cx.subscribe(&editor, |pane, _, _: &Changed, cx| pane.schedule_save(cx)),
            cx.subscribe_in(&editor, window, |pane, _, _: &UpFromStart, window, cx| {
                pane.focus_tags(window, cx);
            }),
            cx.subscribe(&tag_row, |pane, _, change: &TagChange, cx| {
                pane.change_tag(change.clone(), cx);
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
            notes_folder,
            root,
            editor,
            tag_row,
            backlinks,
            links,
            path: None,
            last_info: None,
            fields_pending: 0,
            ending: LineEnding::Lf,
            saved: String::new(),
            autosave: None,
            conflict: false,
            deleted: false,
            error: None,
            opening: None,
            open_heading: None,
            replace_link: false,
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
        let folder = note.info.path.parent().map(|parent| self.root.join(parent));
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
        self.path = Some(path.clone());
        self.last_info = Some(note.info);
        if !reload {
            self.fields_pending = 0;
        }
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
        self.path = None;
        self.last_info = None;
        self.fields_pending = 0;
        self.autosave = None;
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
        if self.path.is_none() {
            return;
        }
        self.links.update(cx, Links::pick_files);
    }

    /// Ctrl+K: reads what the link picker starts with, after the pane saves the open note.
    /// In a note link, the field starts with its target, and the pick changes that link.
    /// With no open note, it gives no task.
    pub(crate) fn start_link(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<Task<holm_core::Result<LinkStart>>> {
        let open_note = self.path.clone()?;
        let link = self.editor.read(cx).link_at_cursor();
        self.replace_link = link.as_ref().is_some_and(|link| !link.embedded);
        let target = link.map(|link| link.target);
        let save = self.save_now(cx);
        Some(cx.spawn(async move |pane, cx| {
            save.await;
            let start = pane.read_with(cx, |pane, cx| {
                let snapshot = pane.notes_folder.read(cx).snapshot();
                let mut notes = snapshot.notes(Order::LastChange);
                notes.sort_by_key(|note| std::cmp::Reverse(note.modified));
                let query = match target {
                    Some(Target::Note { id, heading }) => match snapshot.note_with_id(&id) {
                        Some(note) => match heading {
                            Some(heading) => format!("{}#{heading}", note.title),
                            None => note.title.clone(),
                        },
                        None => String::new(),
                    },
                    Some(Target::Heading(heading)) => format!("#{heading}"),
                    _ => String::new(),
                };
                LinkStart {
                    notes,
                    open_note,
                    query,
                }
            });
            match start {
                Ok(start) => Ok(start),
                Err(_) => std::future::pending().await,
            }
        }))
    }

    /// Makes the link that the user picked in the link picker, and puts it at the cursor.
    /// A new note goes in the folder of the open note. With no open note, it does nothing.
    pub(crate) fn link(
        &mut self,
        choice: LinkChoice,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<()>> {
        let Some(path) = self.path.clone() else {
            return Task::ready(Ok(()));
        };
        let folder = path.parent().map(Path::to_owned).unwrap_or_default();
        self.save_now(cx).detach();
        let make: Task<holm_core::Result<(Target, String)>> = match choice {
            LinkChoice::Note {
                path,
                title,
                heading,
            } => {
                let id = self
                    .notes_folder
                    .update(cx, |notes_folder, cx| notes_folder.link_id(path, cx));
                cx.background_spawn(async move {
                    let text = heading.clone().unwrap_or(title);
                    Ok((
                        Target::Note {
                            id: id.await?,
                            heading,
                        },
                        text,
                    ))
                })
            }
            LinkChoice::SameNote(heading) => {
                Task::ready(Ok((Target::Heading(heading.clone()), heading)))
            }
            LinkChoice::Create(title) => {
                let id = self.notes_folder.update(cx, |notes_folder, cx| {
                    notes_folder.create_titled_note(folder, title.clone(), cx)
                });
                cx.background_spawn(async move {
                    let id = id.await?;
                    Ok((Target::Note { id, heading: None }, title))
                })
            }
        };
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
        if self.path.is_none() {
            return;
        }
        let replace = self.replace_link;
        self.editor.update(cx, |editor, cx| {
            if !(replace && editor.set_link_target(target, cx)) {
                editor.insert_link(text, target, false, cx);
            }
        });
    }

    /// Shows the tags and the pin of the open note from the snapshot, when no write of them
    /// waits.
    fn show_fields(&mut self, cx: &mut Context<Self>) {
        let Some(path) = &self.path else {
            return;
        };
        if let Some(info) = self.notes_folder.read(cx).snapshot().note(path) {
            self.last_info = Some(info.clone());
        }
        if self.fields_pending > 0 {
            return;
        }
        if let Some(info) = &self.last_info {
            self.tag_row
                .update(cx, |row, cx| row.show(&info.tags, info.pinned, cx));
        }
    }

    fn change_tag(&mut self, change: TagChange, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let write = self.notes_folder.update(cx, |notes_folder, cx| {
            notes_folder.change_tag(path, change, cx)
        });
        self.write_fields(write, cx);
    }

    /// Opens the tag popup. The editor scrolls to it.
    pub(crate) fn focus_tags(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.path.is_none() {
            return;
        }
        self.tag_row.update(cx, |row, cx| row.enter(window, cx));
    }

    /// Ctrl+Shift+B: opens the backlinks list. The editor scrolls to it.
    pub(crate) fn show_backlinks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.path.is_none() {
            return;
        }
        self.backlinks
            .update(cx, |backlinks, cx| backlinks.enter(window, cx));
    }

    /// Pins or unpins the open note.
    pub(crate) fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let pinned = self.tag_row.update(cx, TagRow::toggle_pin);
        let write = self.notes_folder.update(cx, |notes_folder, cx| {
            notes_folder.set_pinned(path, pinned, cx)
        });
        self.write_fields(write, cx);
    }

    /// Waits for a write of the tags or the pin. An error shows in the quiet bar.
    fn write_fields(&mut self, write: Task<holm_core::Result<()>>, cx: &mut Context<Self>) {
        self.fields_pending += 1;
        cx.spawn(async move |pane, cx| {
            let result = write.await;
            pane.update(cx, |pane, cx| {
                pane.fields_pending = pane.fields_pending.saturating_sub(1);
                if let Err(error) = result {
                    let error = AppError::from(error);
                    tracing::warn!(error = %crate::chain(&error), "cannot change the tags or the pin");
                    pane.error = Some(error.message());
                    cx.notify();
                }
                pane.show_fields(cx);
            })
            .ok();
        })
        .detach();
    }

    fn is_dirty(&self, cx: &App) -> bool {
        self.editor.read(cx).text() != self.saved
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.autosave = Some(cx.spawn(async move |pane, cx| {
            cx.background_executor().timer(AUTOSAVE).await;
            pane.update(cx, Self::autosave).ok();
        }));
    }

    /// Saves unsaved changes with no conflict.
    fn autosave(&mut self, cx: &mut Context<Self>) {
        if !(self.conflict || self.deleted) {
            self.save_now(cx).detach();
        }
    }

    /// Puts the unsaved text in the write line now. It wins over an outside change, and
    /// writes the note again after an outside delete. The task gives whether the text reached
    /// the file.
    pub(crate) fn save_now(&mut self, cx: &mut Context<Self>) -> Task<bool> {
        self.autosave = None;
        if !self.is_dirty(cx) {
            return Task::ready(true);
        }
        let Some(text) = self.note_text(cx) else {
            return Task::ready(true);
        };
        let shown = self.editor.read(cx).text().to_owned();
        let write = self
            .notes_folder
            .update(cx, |notes_folder, cx| notes_folder.save_text(text, cx));
        cx.spawn(async move |pane, cx| {
            let result = write.await;
            pane.update(cx, |pane, cx| pane.saved(shown, result, cx))
                .unwrap_or(false)
        })
    }

    /// The editor text of the open note, for a write: a restore after an outside delete,
    /// else a save.
    fn note_text(&self, cx: &App) -> Option<NoteText> {
        let path = self.path.clone()?;
        let body = self
            .ending
            .to_disk(self.editor.read(cx).text())
            .into_owned();
        Some(if self.deleted {
            let mut info = self.last_info.clone()?;
            info.path = path;
            NoteText::Restore { info, body }
        } else {
            NoteText::Save { path, body }
        })
    }

    /// Takes the end of a save of `text`. It gives whether the save worked.
    fn saved(
        &mut self,
        text: String,
        result: holm_core::Result<()>,
        cx: &mut Context<Self>,
    ) -> bool {
        let done = match result {
            Ok(()) => {
                self.saved = text;
                self.conflict = false;
                self.deleted = false;
                self.error = None;
                if self.is_dirty(cx) {
                    self.schedule_save(cx);
                }
                true
            }
            Err(error) => {
                let error = AppError::from(error);
                tracing::warn!(error = %crate::chain(&error), "cannot save the note");
                self.error = Some(error.message());
                false
            }
        };
        cx.notify();
        done
    }

    fn outside_change(&mut self, change: Change, cx: &mut Context<Self>) {
        let Some(path) = &mut self.path else {
            return;
        };
        let unsaved = self.editor.read(cx).text() != self.saved;
        match change {
            Change::Changed(info) if info.path == *path => {
                self.deleted = false;
                if unsaved {
                    self.conflict = true;
                    cx.notify();
                } else {
                    self.reload(cx);
                }
            }
            Change::Moved { from, note } if from == *path => {
                path.clone_from(&note.path);
                self.links
                    .update(cx, |links, cx| links.show(Some(note.path), cx));
            }
            Change::Removed(gone) if gone == *path => {
                if unsaved {
                    self.conflict = false;
                    self.deleted = true;
                    self.autosave = None;
                    cx.notify();
                } else {
                    cx.emit(PaneEvent::Gone(gone));
                }
            }
            _ => {}
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let read = self
            .notes_folder
            .update(cx, |notes_folder, cx| notes_folder.read(path, cx));
        cx.spawn(async move |pane, cx| {
            let result = read.await;
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
        self.save_now(cx).detach();
        cx.notify();
    }

    /// "Keep mine" after an outside delete: writes the note again at its path.
    fn keep_deleted(&mut self, cx: &mut Context<Self>) {
        self.save_now(cx).detach();
        cx.notify();
    }

    /// "Close" after an outside delete: drops the note and its text, and the app opens another
    /// note.
    fn close_deleted(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else {
            return;
        };
        self.drop_note(cx);
        cx.emit(PaneEvent::Gone(path));
    }

    /// When the window closes or the app quits: the notes folder runs the writes in its line,
    /// and then writes unsaved text. Unsaved text wins over an outside change.
    fn released(&mut self, cx: &mut App) {
        let last = if self.is_dirty(cx) {
            self.note_text(cx)
        } else {
            None
        };
        self.notes_folder
            .update(cx, |notes_folder, _| notes_folder.quit(last));
    }

    /// The path of the open note, relative to the notes folder.
    pub(crate) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Opens the note at `path`. With [`Leave::Keep`], the pane first saves the open note and
    /// renames its file to match the title. When that save fails, the pane stays on the note.
    pub(crate) fn open(&mut self, path: PathBuf, leave: Leave, cx: &mut Context<Self>) {
        if leave == Leave::Keep && self.path() == Some(path.as_path()) {
            return;
        }
        let old = self.path.clone().filter(|_| leave == Leave::Keep);
        let save = old.is_some().then(|| self.save_now(cx));
        self.opening = Some(cx.spawn(async move |pane, cx| {
            if let Some(save) = save
                && !save.await
            {
                return;
            }
            let Ok((renamed, read)) = pane.update(cx, |pane, cx| {
                pane.notes_folder.update(cx, |notes_folder, cx| {
                    let renamed = old.map(|old| notes_folder.rename_to_title(old, cx));
                    (renamed, notes_folder.read(path, cx))
                })
            }) else {
                return;
            };
            if let Some(renamed) = renamed
                && let Err(error) = renamed.await
            {
                let error = AppError::from(error);
                tracing::warn!(error = %crate::chain(&error), "cannot rename the note to its title");
            }
            let read = read.await;
            pane.update(cx, |pane, cx| pane.opened(read, cx)).ok();
        }));
    }

    fn opened(&mut self, read: holm_core::Result<Note>, cx: &mut Context<Self>) {
        self.opening = None;
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

    /// Follows a move of the open note, or of a folder that holds it, from `from` to `to`, and
    /// reads it again. A move can change the file links in the note.
    pub(crate) fn moved(&mut self, from: &Path, to: &Path, cx: &mut Context<Self>) {
        let Some(path) = &mut self.path else {
            return;
        };
        let Ok(rest) = path.strip_prefix(from) else {
            return;
        };
        let new = if rest.as_os_str().is_empty() {
            to.to_owned()
        } else {
            to.join(rest)
        };
        path.clone_from(&new);
        let folder = new.parent().map(|parent| self.root.join(parent));
        self.editor
            .update(cx, |editor, cx| editor.set_folder(folder, cx));
        self.reload(cx);
        cx.emit(PaneEvent::Opened(new));
    }

    /// Selects the title text in the editor and gives the editor the focus.
    pub(crate) fn select_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.path.is_none() {
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
        let body = if self.path.is_some() {
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
