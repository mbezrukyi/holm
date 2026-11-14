//! The editor pane: one note in the editor, with autosave. With no open note, it shows the
//! empty pane.
//!
//! The pane saves a short time after the user stops typing, and when the app quits.
//! An outside change of the note reloads it quietly when the pane has no unsaved
//! changes. Else the message line asks to reload or to keep the text in the editor.
//! When a save fails, the message line shows "Not saved", and the text stays in the editor.
//! When another tool deletes the note, the app opens another one. With unsaved changes, the
//! message line first asks to write the note again or to close it.
//!
//! When the user opens another note, the pane saves the note. Holm never renames a note by
//! itself.
//!
//! The title field in the tag row renames the note. Ctrl+N shows a [`NewNote`] in the place of
//! the editor. Its file comes only with its title.
//!
//! The tag row before the first line shows the tags, the pin and the path of the note. It
//! scrolls with the text. The pane writes the changes of the tags and the pin at once. The
//! editor undo stack holds only the text, so Ctrl+Z does not undo them.
//!
//! The pane runs the link picker flow. `links` opens the links of the note and reads its
//! backlinks. The backlinks show in the tag row.

mod disk_menu;
mod links;
mod new_note;

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::rc::Rc;
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
use crate::error::AppError;
use crate::link_picker::{LinkChoice, LinkStart};
use crate::message_line::{MessageLine, OpenState};
use crate::name_field::{NameEvent, NameWrite};
use crate::notes_folder::{FolderEvent, Move, NoteText, NotesFolder, Step, window_step};
use crate::tags::{TagChange, TagRow};
use crate::title::TitleField;
use crate::{text, theme};
use disk_menu::{Answer, Choice, Disk, DiskMenu};
use links::{Links, LinksEvent};
use new_note::{NewNote, NewNoteEvent};

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
    /// Save it.
    Keep,
    /// The note is gone. Drop its text.
    Drop,
}

pub(crate) struct NotePane {
    notes_folder: Entity<NotesFolder>,
    line: Entity<MessageLine>,
    disk_menu: Entity<DiskMenu>,
    /// The notes folder on disk, for the file links of the editor.
    root: PathBuf,
    editor: Entity<Editor>,
    tag_row: Entity<TagRow>,
    title: Entity<TitleField<PathBuf>>,
    backlinks: Entity<Backlinks>,
    links: Entity<Links>,
    /// The path of the open note. With no note and no new note, the pane is empty.
    path: Option<PathBuf>,
    /// The note after Ctrl+N, before it has a file.
    new_note: Option<(Entity<NewNote>, Subscription)>,
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
    /// Another tool changed or deleted the note while the editor had unsaved changes.
    disk: Option<Disk>,
    /// The reason of the last failed save, until a save works.
    not_saved: Option<&'static str>,
    /// The number of the last open or drop of a note. A read for an older one is dropped.
    opening: u64,
    /// The heading to show when the note that opens is ready.
    open_heading: Option<String>,
    /// Whether the link picker changes the link at the cursor, not inserts a new one.
    replace_link: bool,
    _subscriptions: Vec<Subscription>,
}

impl NotePane {
    pub(crate) fn new(
        notes_folder: Entity<NotesFolder>,
        line: Entity<MessageLine>,
        root: PathBuf,
        note: Option<Note>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(Editor::new);
        let text = editor.focus_handle(cx);
        let backlinks = cx.new(|cx| Backlinks::new(text.clone(), window, cx));
        let state = line.read(cx).state_bounds();
        let disk_menu = cx.new(|cx| DiskMenu::new(text.clone(), state, window, cx));
        let pane = cx.weak_entity();
        let rename: NameWrite<PathBuf> = Rc::new(move |title, done, _, cx| {
            pane.update(cx, |pane, cx| pane.rename(title, done, cx))
                .ok();
        });
        let title = cx.new(|_| TitleField::new(rename, text.clone()));
        let tag_row = cx.new(|cx| {
            TagRow::new(
                notes_folder.clone(),
                title.clone(),
                backlinks.clone(),
                text,
                window,
                cx,
            )
        });
        editor.update(cx, |editor, cx| {
            editor.set_start(Some(tag_row.clone().into()), cx);
        });
        let links =
            cx.new(|cx| Links::new(notes_folder.clone(), editor.clone(), backlinks.clone(), cx));
        let subscriptions = vec![
            cx.subscribe(&notes_folder, |pane, _, event, cx| match event {
                FolderEvent::Changed(moves) => {
                    pane.follow(moves, cx);
                    pane.show_fields(cx);
                }
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
            cx.subscribe(&title, |_, title, event: &NameEvent<PathBuf>, cx| {
                if *event == NameEvent::Blur {
                    title.update(cx, TitleField::close);
                }
            }),
            cx.subscribe(&disk_menu, |pane, _, answer: &Answer, cx| {
                pane.answer_disk(*answer, cx);
            }),
            cx.subscribe_in(&line, window, |pane, _, _: &OpenState, window, cx| {
                pane.open_state(window, cx);
            }),
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
            line,
            disk_menu,
            root,
            editor,
            tag_row,
            title,
            backlinks,
            links,
            path: None,
            new_note: None,
            last_info: None,
            fields_pending: 0,
            ending: LineEnding::Lf,
            saved: String::new(),
            autosave: None,
            disk: None,
            not_saved: None,
            opening: 0,
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
        self.settle(cx);
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
        self.new_note = None;
        self.title.update(cx, TitleField::close);
        self.path = None;
        self.last_info = None;
        self.fields_pending = 0;
        self.autosave = None;
        self.opening += 1;
        self.settle(cx);
        self.editor.update(cx, |editor, cx| editor.set_text("", cx));
        self.saved = String::new();
        self.links.update(cx, |links, cx| links.show(None, cx));
        self.show_fields(cx);
        cx.notify();
    }

    fn links_event(&mut self, _: Entity<Links>, event: &LinksEvent, cx: &mut Context<Self>) {
        match event {
            LinksEvent::Open { path, heading } => {
                self.open_heading.clone_from(heading);
                self.open(path.clone(), Leave::Keep, cx);
            }
            LinksEvent::Error(message) => self.answer(message, cx),
        }
    }

    /// Shows a one-time error in the message line.
    fn answer(&self, message: &'static str, cx: &mut Context<Self>) {
        self.line.update(cx, |line, cx| line.show(message, cx));
    }

    /// Sets the disk question. A new question closes the menu of the old one.
    fn set_disk(&mut self, disk: Option<Disk>, cx: &mut Context<Self>) {
        if self.disk != disk {
            self.disk = disk;
            self.disk_menu.update(cx, DiskMenu::close);
        }
        self.show_state(cx);
    }

    /// The editor text matches the file: no disk question, and no "Not saved".
    fn settle(&mut self, cx: &mut Context<Self>) {
        self.not_saved = None;
        self.set_disk(None, cx);
    }

    /// Shows the state in the message line. A disk question wins over "Not saved".
    fn show_state(&self, cx: &mut Context<Self>) {
        let state = match (self.disk, self.not_saved) {
            (Some(Disk::Changed), _) => Some(text::CHANGED_ON_DISK),
            (Some(Disk::Deleted), _) => Some(text::DELETED_ON_DISK),
            (None, Some(_)) => Some(text::NOT_SAVED),
            (None, None) => None,
        };
        self.line
            .update(cx, |line, cx| line.set_state(state.map(Into::into), cx));
    }

    /// Alt+D or a click on the state: opens or closes the disk menu, or shows why the note is
    /// not saved.
    pub(crate) fn open_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(disk) = self.disk {
            self.disk_menu
                .update(cx, |menu, cx| menu.toggle(disk, window, cx));
        } else if let Some(reason) = self.not_saved {
            self.answer(reason, cx);
        }
    }

    fn answer_disk(&mut self, Answer { disk, choice }: Answer, cx: &mut Context<Self>) {
        match (disk, choice) {
            (Disk::Changed, Choice::KeepMine) => self.keep_mine(cx),
            (Disk::Deleted, Choice::KeepMine) => self.keep_deleted(cx),
            (_, Choice::Reload) => self.reload(cx),
            (_, Choice::Close) => self.close_deleted(cx),
        }
    }

    /// Opens the system file picker, and embeds each picked file at the cursor.
    pub(crate) fn link_files(&mut self, cx: &mut Context<Self>) {
        if self.path.is_none() {
            return;
        }
        self.links.update(cx, Links::pick_files);
    }

    /// Ctrl+K: gives `done` what the link picker starts with, after the pane saves the open
    /// note. In a note link, the field starts with its target, and the pick changes that link.
    /// With no open note, it does nothing.
    pub(crate) fn start_link(
        &mut self,
        done: impl FnOnce(LinkStart, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(open_note) = self.path.clone() else {
            return;
        };
        let link = self.editor.read(cx).link_at_cursor();
        self.replace_link = link.as_ref().is_some_and(|link| !link.embedded);
        let target = link.map(|link| link.target);
        self.save_then(
            move |pane, _, cx| {
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
                let start = LinkStart {
                    notes,
                    open_note,
                    query,
                };
                // The caller may still be in its own update here.
                cx.defer(move |cx| done(start, cx));
            },
            cx,
        );
    }

    /// Makes the link that the user picked in the link picker, and puts it at the cursor.
    /// A new note goes in the folder of the open note. With no open note, it does nothing.
    pub(crate) fn link(
        &mut self,
        choice: LinkChoice,
        done: impl FnOnce(holm_core::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.path.clone() else {
            cx.defer(move |cx| done(Ok(()), cx));
            return;
        };
        let folder = path.parent().map(Path::to_owned).unwrap_or_default();
        self.save(cx);
        let pane = cx.weak_entity();
        let put = move |target: Target, text: String, cx: &mut App| {
            pane.update(cx, |pane, cx| pane.put_link(&target, &text, cx))
                .ok();
        };
        match choice {
            LinkChoice::Note {
                path,
                title,
                heading,
            } => self.notes_folder.update(cx, |notes_folder, cx| {
                notes_folder.link_id(
                    path,
                    move |id, cx| {
                        let made = id.map(|id| {
                            let text = heading.clone().unwrap_or(title);
                            put(Target::Note { id, heading }, text, cx);
                        });
                        done(made, cx);
                    },
                    cx,
                );
            }),
            LinkChoice::SameNote(heading) => {
                self.put_link(&Target::Heading(heading.clone()), &heading, cx);
                cx.defer(move |cx| done(Ok(()), cx));
            }
            LinkChoice::Create(title) => self.notes_folder.update(cx, |notes_folder, cx| {
                notes_folder.create_linked_note(
                    folder,
                    title.clone(),
                    move |id, cx| {
                        let made = id.map(|id| put(Target::Note { id, heading: None }, title, cx));
                        done(made, cx);
                    },
                    cx,
                );
            }),
        }
    }

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
        let path = self.path.clone();
        self.title.update(cx, |title, cx| title.show(path, cx));
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
        let done = self.field_step(cx);
        self.notes_folder.update(cx, |notes_folder, cx| {
            notes_folder.change_tag(path, change, done, cx);
        });
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
        let done = self.field_step(cx);
        self.notes_folder.update(cx, |notes_folder, cx| {
            notes_folder.set_pinned(path, pinned, done, cx);
        });
    }

    /// The end of a write of the tags or the pin. An error shows in the message line.
    fn field_step(&mut self, cx: &Context<Self>) -> Step<holm_core::Result<()>> {
        self.fields_pending += 1;
        step(cx, |pane, result, cx| {
            pane.fields_pending = pane.fields_pending.saturating_sub(1);
            if let Err(error) = result {
                let error = AppError::from(error);
                tracing::warn!(error = %crate::chain(&error), "cannot change the tags or the pin");
                pane.answer(error.message(), cx);
            }
            pane.show_fields(cx);
        })
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
        if self.disk.is_none() {
            self.save(cx);
        }
    }

    /// Puts the unsaved text in the write line now. It wins over an outside change, and
    /// writes the note again after an outside delete.
    pub(crate) fn save(&mut self, cx: &mut Context<Self>) {
        self.save_then(|_, _, _| {}, cx);
    }

    /// Like [`Self::save`], and then gives `then` whether the text reached the file. With no
    /// unsaved text, `then` runs at once.
    fn save_then(
        &mut self,
        then: impl FnOnce(&mut Self, bool, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.autosave = None;
        let text = self.note_text(cx).filter(|_| self.is_dirty(cx));
        let Some(text) = text else {
            then(self, true, cx);
            return;
        };
        let shown = self.editor.read(cx).text().to_owned();
        let done = step(cx, move |pane, result, cx| {
            let saved = pane.saved(shown, result, cx);
            then(pane, saved, cx);
        });
        self.notes_folder.update(cx, |notes_folder, cx| {
            notes_folder.save_text(text, done, cx);
        });
    }

    /// The editor text of the open note, for a write: a restore after an outside delete,
    /// else a save.
    fn note_text(&self, cx: &App) -> Option<NoteText> {
        let path = self.path.clone()?;
        let body = self
            .ending
            .to_disk(self.editor.read(cx).text())
            .into_owned();
        Some(if self.disk == Some(Disk::Deleted) {
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
                self.settle(cx);
                if self.is_dirty(cx) {
                    self.schedule_save(cx);
                }
                true
            }
            Err(error) => {
                let error = AppError::from(error);
                tracing::warn!(error = %crate::chain(&error), "cannot save the note");
                // Only the first fail since the last good save shows the reason.
                if self.not_saved.is_none() {
                    self.answer(error.message(), cx);
                }
                self.not_saved = Some(error.message());
                self.show_state(cx);
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
                if unsaved {
                    self.set_disk(Some(Disk::Changed), cx);
                } else {
                    self.set_disk(None, cx);
                    self.reload(cx);
                }
            }
            Change::Moved { from, note } if from == *path => {
                path.clone_from(&note.path);
                self.links
                    .update(cx, |links, cx| links.show(Some(note.path), cx));
                self.show_fields(cx);
            }
            Change::Removed(gone) if gone == *path => {
                if unsaved {
                    self.autosave = None;
                    self.set_disk(Some(Disk::Deleted), cx);
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
        let opening = self.opening;
        let done = step(cx, move |pane: &mut Self, result, cx| {
            if pane.opening != opening {
                return;
            }
            match result {
                Ok(note) => pane.show(note, true, cx),
                Err(error) => {
                    let error = AppError::from(error);
                    tracing::warn!(error = %crate::chain(&error), "cannot reload the note");
                    pane.answer(error.message(), cx);
                }
            }
        });
        self.notes_folder
            .update(cx, |notes_folder, cx| notes_folder.read(path, done, cx));
    }

    fn keep_mine(&mut self, cx: &mut Context<Self>) {
        self.set_disk(None, cx);
        self.save(cx);
    }

    /// "Keep mine" after an outside delete: writes the note again at its path.
    fn keep_deleted(&mut self, cx: &mut Context<Self>) {
        self.save(cx);
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

    /// Opens the note at `path`, and drops a new note. With [`Leave::Keep`], the pane first
    /// saves the open note. When that save fails, the pane stays on the note.
    pub(crate) fn open(&mut self, path: PathBuf, leave: Leave, cx: &mut Context<Self>) {
        if leave == Leave::Keep && self.path() == Some(path.as_path()) {
            return;
        }
        if self.new_note.take().is_some() {
            cx.notify();
        }
        self.opening += 1;
        let opening = self.opening;
        if self.path.is_none() || leave == Leave::Drop {
            self.read_note(path, opening, cx);
            return;
        }
        self.save_then(
            move |pane, saved, cx| {
                if saved && pane.opening == opening {
                    pane.read_note(path, opening, cx);
                }
            },
            cx,
        );
    }

    /// Reads the note at `path` and shows it, unless another open or drop came after
    /// `opening`.
    fn read_note(&mut self, path: PathBuf, opening: u64, cx: &mut Context<Self>) {
        let done = step(cx, move |pane: &mut Self, read, cx| {
            if pane.opening == opening {
                pane.opened(read, cx);
            }
        });
        self.notes_folder
            .update(cx, |notes_folder, cx| notes_folder.read(path, done, cx));
    }

    fn opened(&mut self, read: holm_core::Result<Note>, cx: &mut Context<Self>) {
        match read {
            Ok(note) => {
                let path = note.info.path.clone();
                self.show(note, false, cx);
                cx.emit(PaneEvent::Opened(path));
            }
            Err(error) => {
                let error = AppError::from(error);
                tracing::warn!(error = %crate::chain(&error), "cannot open the note");
                self.answer(error.message(), cx);
            }
        }
    }

    /// Follows a write that moved the open note, or a folder that holds it. A move to
    /// another folder can change the file links in the note, so the pane reads it again.
    fn follow(&mut self, moves: &[Move], cx: &mut Context<Self>) {
        let Some(path) = &self.path else {
            return;
        };
        let Some(new) = moves.iter().find_map(|one| one.path(path)) else {
            return;
        };
        let other_folder = new.parent() != path.parent();
        self.path = Some(new.clone());
        if other_folder {
            let folder = new.parent().map(|parent| self.root.join(parent));
            self.editor
                .update(cx, |editor, cx| editor.set_folder(folder, cx));
            self.reload(cx);
        } else {
            self.links.update(cx, |links, cx| links.show(Some(new), cx));
        }
    }

    /// F2: opens the title field of the open note.
    pub(crate) fn edit_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.path.is_some() {
            self.title.update(cx, |title, cx| title.open(window, cx));
        }
    }

    /// Gives the open note the title `title`, after its unsaved text.
    fn rename(
        &mut self,
        title: String,
        done: Step<holm_core::Result<PathBuf>>,
        cx: &mut Context<Self>,
    ) {
        // With no note, the title field is gone too.
        let Some(path) = self.path.clone() else {
            return;
        };
        self.save(cx);
        self.notes_folder.update(cx, |notes_folder, cx| {
            notes_folder.rename_note(path, title, done, cx);
        });
    }

    /// Ctrl+N: saves the open note, and then shows a new note for `folder`, with `tag` when it
    /// has one. When the save fails, the pane stays on the note. Another Ctrl+N on a new note
    /// only gives its field the focus.
    pub(crate) fn new_note(
        &mut self,
        folder: PathBuf,
        tag: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((note, _)) = &self.new_note {
            note.update(cx, |note, cx| note.focus(window, cx));
            return;
        }
        self.opening += 1;
        let opening = self.opening;
        let start = window_step(window, cx, move |pane: &mut Self, (), window, cx| {
            if pane.opening != opening {
                return;
            }
            let before = pane.path.clone();
            pane.drop_note(cx);
            let notes_folder = pane.notes_folder.clone();
            let note = cx.new(|cx| NewNote::new(notes_folder, folder, tag, before, window, cx));
            let subscription = cx.subscribe_in(&note, window, Self::new_note_event);
            pane.new_note = Some((note, subscription));
            cx.notify();
        });
        self.save_then(
            move |_, saved, cx| {
                if saved {
                    cx.defer(move |cx| start((), cx));
                }
            },
            cx,
        );
    }

    fn new_note_event(
        &mut self,
        _: &Entity<NewNote>,
        event: &NewNoteEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_note = None;
        window.focus(&self.editor.focus_handle(cx), cx);
        cx.notify();
        match event {
            NewNoteEvent::Created(path) => self.open(path.clone(), Leave::Drop, cx),
            NewNoteEvent::Cancelled {
                before: Some(before),
            } => {
                self.open(before.clone(), Leave::Drop, cx);
            }
            NewNoteEvent::Cancelled { before: None } => self.close(cx),
        }
    }
}

/// A `done` step for a write of the notes folder: it runs `step` on the pane, when the pane
/// still exists.
fn step<R: 'static>(
    cx: &Context<NotePane>,
    step: impl FnOnce(&mut NotePane, R, &mut Context<NotePane>) + 'static,
) -> Step<R> {
    let pane = cx.weak_entity();
    Box::new(move |result, cx| {
        pane.update(cx, |pane, cx| step(pane, result, cx)).ok();
    })
}

impl EventEmitter<PaneEvent> for NotePane {}

impl Focusable for NotePane {
    fn focus_handle(&self, cx: &gpui_kit::App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Render for NotePane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = if let Some((note, _)) = &self.new_note {
            note.clone().into_any_element()
        } else if self.path.is_some() {
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
            .child(body)
            .child(self.disk_menu.clone())
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
