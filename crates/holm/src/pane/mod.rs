//! The editor pane: one note in the editor, with autosave. With no open note, it shows the
//! empty pane.
//!
//! The pane keeps its unsaved text in the draft of the core door. The write line writes the
//! draft before each job, so each file action sees the text. The pane also saves a short time
//! after the user stops typing, and when the app quits. A close of the window saves first.
//! When that save fails, the window stays open until a second close.
//! An outside change of the note reloads it quietly when the pane has no unsaved
//! changes. Else the message line asks to reload or to keep the text in the editor.
//! When a save fails, the message line shows "Not saved", and the text stays in the editor.
//! When another tool deletes the note, the app opens another one. With unsaved changes, the
//! message line first asks to write the note again or to close it.
//!
//! When the user opens another note, the pane saves the note first. When that save fails, the
//! pane stays on the note. Holm never renames a note by itself.
//!
//! The title field in the tag row renames the note. Ctrl+N shows a [`NewNote`] in the place of
//! the editor. Its file comes only with its title. The editor keeps the note from before, so
//! `Esc` shows it again as it was.
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
use holm_core::model::{Change, Note, NoteInfo};
use holm_core::{FolderPath, NotePath, TagName};
use holm_editor::{Changed, Editor, UpFromStart};
use holm_markdown::Target;

use crate::backlinks::{Backlinks, OpenBacklink};
use crate::core_door::{CoreDoor, Done, FolderEvent, NotSaved, SaveEvent, Then, window_step};
use crate::error::AppError;
use crate::link_picker::{LinkChoice, LinkStart};
use crate::message_line::{MessageLine, OpenState};
use crate::name_field::{NameEvent, NameWrite};
use crate::tags::{TagChange, TagRow};
use crate::title::TitleField;
use crate::{text, theme};
use disk_menu::{Choice, Chosen, DiskMenu, DiskQuestion};
pub(crate) use links::Pick;
use links::{Links, LinksEvent};
use new_note::{NewNote, NewNoteEvent};

/// How long the pane waits after the last change before it saves.
const AUTOSAVE: Duration = Duration::from_millis(500);

/// What the pane tells the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PaneEvent {
    /// The pane starts to read this note. It is the open note from now on.
    Opening(NotePath),
    /// The pane shows the note at this path: after an open, a move, or an open that stopped.
    Opened(NotePath),
    /// The pane shows no note.
    Closed,
    /// Another tool deleted the open note at this path, and the pane has no text to keep.
    /// The app must open another note or call [`NotePane::close`]. The pane cannot pick the
    /// note, because the neighbor comes from the sidebar list.
    Gone(NotePath),
}

/// What happens to the open note when the pane opens another one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Leave {
    /// Save it.
    Keep,
    /// The note is gone. Drop its text.
    Drop,
}

/// What the pane shows.
enum Content {
    Empty,
    /// The note at this path, in the editor.
    Note(NotePath),
    /// The note after Ctrl+N, before it has a file, in the place of the editor. The editor
    /// keeps the note from before, at `kept`.
    New {
        note: Entity<NewNote>,
        kept: Option<NotePath>,
        _subscription: Subscription,
    },
}

impl Content {
    /// The note that the user sees and works on.
    fn note(&self) -> Option<&NotePath> {
        match self {
            Self::Note(path) => Some(path),
            Self::Empty | Self::New { .. } => None,
        }
    }

    /// The note in the editor, also under a new note. Saves, reloads and moves follow it.
    fn editor(&self) -> Option<&NotePath> {
        match self {
            Self::Note(path) => Some(path),
            Self::New { kept, .. } => kept.as_ref(),
            Self::Empty => None,
        }
    }

    fn editor_mut(&mut self) -> Option<&mut NotePath> {
        match self {
            Self::Note(path) => Some(path),
            Self::New { kept, .. } => kept.as_mut(),
            Self::Empty => None,
        }
    }
}

pub(crate) struct NotePane {
    core_door: Entity<CoreDoor>,
    message_line: Entity<MessageLine>,
    disk_menu: Entity<DiskMenu>,
    editor: Entity<Editor>,
    tag_row: Entity<TagRow>,
    title: Entity<TitleField<NotePath>>,
    backlinks: Entity<Backlinks>,
    links: Entity<Links>,
    content: Content,
    /// The note that an open reads, until it shows. Moves follow it too.
    next: Option<NotePath>,
    /// The facts of the open note in the last snapshot that had it.
    last_info: Option<NoteInfo>,
    ending: LineEnding,
    autosave: Option<Task<()>>,
    /// Another tool changed or deleted the note while the editor had unsaved changes.
    disk: Option<DiskQuestion>,
    /// The number of the last open or drop of a note. A read for an older one is dropped.
    opening: u64,
    /// The heading to show when the note that opens is ready.
    open_heading: Option<String>,
    /// Whether the link picker changes the link at the cursor, not inserts a new one.
    replace_link: bool,
    /// A save for a close of the window runs.
    closing: bool,
    _subscriptions: Vec<Subscription>,
}

impl NotePane {
    pub(crate) fn new(
        core_door: Entity<CoreDoor>,
        line: Entity<MessageLine>,
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
        let rename: NameWrite<NotePath> = Rc::new(move |title, done, _, cx| {
            pane.update(cx, |pane, cx| pane.rename(title, done, cx))
                .ok();
        });
        let title = cx.new(|_| TitleField::new(rename, text.clone()));
        let tag_row = cx.new(|cx| {
            TagRow::new(
                core_door.clone(),
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
            cx.new(|cx| Links::new(core_door.clone(), editor.clone(), backlinks.clone(), cx));
        let subscriptions = vec![
            cx.subscribe(&core_door, |pane, _, event: &FolderEvent, cx| {
                pane.follow(event, cx);
                for change in event.outside() {
                    pane.outside_change(change, cx);
                }
                pane.show_fields(cx);
            }),
            cx.subscribe(&core_door, |pane, _, event: &SaveEvent, cx| {
                pane.save_end(event, cx);
            }),
            cx.subscribe(&editor, |pane, _, _: &Changed, cx| pane.edited(cx)),
            cx.subscribe_in(&editor, window, |pane, _, _: &UpFromStart, window, cx| {
                pane.focus_tags(window, cx);
            }),
            cx.subscribe(&tag_row, |pane, _, change: &TagChange, cx| {
                pane.change_tag(change.clone(), cx);
            }),
            cx.subscribe(&links, Self::links_event),
            cx.subscribe(&title, |_, title, event: &NameEvent<NotePath>, cx| {
                if *event == NameEvent::Blur {
                    title.update(cx, TitleField::close);
                }
            }),
            cx.subscribe(&disk_menu, |pane, _, chosen: &Chosen, cx| {
                pane.choose(*chosen, cx);
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
        ];
        let appearance = theme::appearance(cx);
        editor.update(cx, |editor, cx| editor.set_appearance(appearance, cx));
        let mut pane = Self {
            core_door,
            message_line: line,
            disk_menu,
            editor,
            tag_row,
            title,
            backlinks,
            links,
            content: Content::Empty,
            next: None,
            last_info: None,
            ending: LineEnding::Lf,
            autosave: None,
            disk: None,
            opening: 0,
            open_heading: None,
            replace_link: false,
            closing: false,
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
        let folder = Some(self.core_door.read(cx).root().join(note.info.path.folder()));
        self.editor.update(cx, |editor, cx| {
            if reload {
                editor.reload_text(&note.body, cx);
            } else {
                editor.set_text(&note.body, cx);
                editor.set_folder(folder, cx);
            }
        });
        let path = note.info.path.clone();
        if !reload {
            self.content = Content::Note(path.clone());
        } else if let Some(open) = self.content.editor_mut() {
            open.clone_from(&path);
        }
        self.last_info = Some(note.info);
        self.show_fields(cx);
        self.set_disk(None, cx);
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
        if let Some(path) = self.content.editor().cloned() {
            self.core_door
                .update(cx, |core_door, _| core_door.drop_text(&path));
        }
        self.title.update(cx, TitleField::close);
        self.content = Content::Empty;
        self.next = None;
        self.last_info = None;
        self.autosave = None;
        self.opening += 1;
        self.set_disk(None, cx);
        self.editor.update(cx, |editor, cx| editor.set_text("", cx));
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
        self.message_line
            .update(cx, |line, cx| line.show(message, cx));
    }

    /// Sets the disk question. A new question closes the menu of the old one. A question holds
    /// the text of the note in the draft.
    fn set_disk(&mut self, disk: Option<DiskQuestion>, cx: &mut Context<Self>) {
        if disk.is_some() {
            self.hold_text(cx);
        }
        if self.disk != disk {
            self.disk = disk;
            self.disk_menu.update(cx, DiskMenu::close);
        }
        self.show_state(cx);
    }

    fn hold_text(&self, cx: &mut Context<Self>) {
        if let Some(path) = self.content.editor() {
            self.core_door
                .update(cx, |core_door, _| core_door.hold_text(path));
        }
    }

    /// Shows the state in the message line. A disk question wins over "Not saved".
    fn show_state(&self, cx: &mut Context<Self>) {
        let state = match (self.disk, self.core_door.read(cx).not_saved()) {
            (Some(DiskQuestion::Changed), _) => Some(text::CHANGED_ON_DISK),
            (Some(DiskQuestion::Deleted), _) => Some(text::DELETED_ON_DISK),
            (None, Some(_)) => Some(text::NOT_SAVED),
            (None, None) => None,
        };
        self.message_line
            .update(cx, |line, cx| line.set_state(state.map(Into::into), cx));
    }

    /// Ctrl+Shift+D or a click on the state: opens or closes the disk menu, or shows why the note is
    /// not saved.
    pub(crate) fn open_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(disk) = self.disk {
            self.disk_menu
                .update(cx, |menu, cx| menu.toggle(disk, window, cx));
        } else if let Some(reason) = self.core_door.read(cx).not_saved() {
            self.answer(reason, cx);
        }
    }

    fn choose(&mut self, Chosen { question, choice }: Chosen, cx: &mut Context<Self>) {
        match (question, choice) {
            (DiskQuestion::Changed, Choice::KeepMine) => self.keep_mine(cx),
            (DiskQuestion::Deleted, Choice::KeepMine) => self.keep_text(cx),
            (_, Choice::Reload) => self.reload(cx),
            (_, Choice::Close) => self.close_deleted(cx),
        }
    }

    /// Opens the system picker for `pick`, and embeds each picked path at the cursor.
    pub(crate) fn link_paths(&mut self, pick: Pick, cx: &mut Context<Self>) {
        if self.content.note().is_none() {
            return;
        }
        self.links.update(cx, |links, cx| links.pick(pick, cx));
    }

    /// Ctrl+K: gives `done` what the link picker starts with, after the write line writes the
    /// unsaved text. In a note link, the field starts with its target, and the pick changes that
    /// link.
    /// With no open note, it does nothing.
    pub(crate) fn start_link(
        &mut self,
        done: impl FnOnce(LinkStart, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(open_note) = self.content.note().cloned() else {
            return;
        };
        let link = self.editor.read(cx).link_at_cursor();
        self.replace_link = link.as_ref().is_some_and(|link| !link.embedded);
        let target = link.map(|link| link.target);
        let done = Done::value(step(cx, move |pane: &mut Self, (), cx| {
            let snapshot = pane.core_door.read(cx).snapshot();
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
            done(LinkStart::new(snapshot, open_note, query), cx);
        }));
        self.core_door
            .update(cx, |core_door, cx| core_door.wait(done, cx));
    }

    /// Makes the link that the user picked in the link picker, and puts it at the cursor.
    /// A new note goes in the folder of the open note. With no open note, it does nothing.
    /// `done` runs when the link is in place.
    pub(crate) fn link(&mut self, choice: LinkChoice, done: Then<()>, cx: &mut Context<Self>) {
        let Some(path) = self.content.note().cloned() else {
            cx.defer(move |cx| done((), cx));
            return;
        };
        let folder = path.folder().clone();
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
            } => self.core_door.update(cx, |core_door, cx| {
                core_door.link_id(
                    path,
                    Done::value(move |id, cx| {
                        let text = heading.clone().unwrap_or(title);
                        put(Target::Note { id, heading }, text, cx);
                        done((), cx);
                    }),
                    cx,
                );
            }),
            LinkChoice::SameNote(heading) => {
                self.put_link(&Target::Heading(heading.clone()), &heading, cx);
                cx.defer(move |cx| done((), cx));
            }
            LinkChoice::Create(title) => self.core_door.update(cx, |core_door, cx| {
                core_door.create_linked_note(
                    folder,
                    title.clone(),
                    Done::value(move |id, cx| {
                        put(Target::Note { id, heading: None }, title, cx);
                        done((), cx);
                    }),
                    cx,
                );
            }),
        }
    }

    fn put_link(&mut self, target: &Target, text: &str, cx: &mut Context<Self>) {
        if self.content.note().is_none() {
            return;
        }
        let replace = self.replace_link;
        self.editor.update(cx, |editor, cx| {
            if !(replace && editor.set_link_target(target, cx)) {
                editor.insert_link(text, target, false, cx);
            }
        });
    }

    /// Shows the tags and the pin of the open note from the snapshot.
    fn show_fields(&mut self, cx: &mut Context<Self>) {
        let path = self.content.editor().cloned();
        self.title
            .update(cx, |title, cx| title.show(path.clone(), cx));
        let Some(path) = path else {
            return;
        };
        if let Some(info) = self.core_door.read(cx).snapshot().note(&path) {
            self.last_info = Some(info.clone());
        }
        if let Some(info) = &self.last_info {
            self.tag_row
                .update(cx, |row, cx| row.show(&info.tags, info.pinned, cx));
        }
    }

    fn change_tag(&mut self, change: TagChange, cx: &mut Context<Self>) {
        let Some(path) = self.content.note().cloned() else {
            return;
        };
        // The event of the job shows the fields.
        self.core_door.update(cx, |core_door, cx| {
            core_door.change_tag(path, change, Done::value(|(), _| {}), cx);
        });
    }

    /// Opens the tag popup. The editor scrolls to it.
    pub(crate) fn focus_tags(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.content.note().is_none() {
            return;
        }
        self.tag_row.update(cx, |row, cx| row.enter(window, cx));
    }

    /// Ctrl+Shift+B: opens the backlinks list. The editor scrolls to it.
    pub(crate) fn show_backlinks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.content.note().is_none() {
            return;
        }
        self.backlinks
            .update(cx, |backlinks, cx| backlinks.enter(window, cx));
    }

    /// Pins or unpins the open note.
    pub(crate) fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.content.note().cloned() else {
            return;
        };
        let pinned = !self.last_info.as_ref().is_some_and(|info| info.pinned);
        self.core_door.update(cx, |core_door, cx| {
            core_door.set_pinned(path, pinned, Done::value(|(), _| {}), cx);
        });
    }

    fn unsaved(&self, cx: &App) -> bool {
        self.core_door.read(cx).unsaved()
    }

    /// Gives the editor text to the draft of the core door, and saves it soon.
    fn edited(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.content.editor().cloned() {
            let body = self
                .ending
                .to_disk(self.editor.read(cx).text())
                .into_owned();
            self.core_door
                .update(cx, |core_door, cx| core_door.set_text(path, body, cx));
        }
        self.schedule_save(cx);
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.autosave = Some(cx.spawn(async move |pane, cx| {
            cx.background_executor().timer(AUTOSAVE).await;
            pane.update(cx, Self::save).ok();
        }));
    }

    /// Puts the unsaved text in the write line now. A text that a disk question holds stays.
    fn save(&mut self, cx: &mut Context<Self>) {
        self.autosave = None;
        self.core_door.update(cx, CoreDoor::save_text);
    }

    /// A close of the window: gives whether the window may close now. With unsaved text, the
    /// pane saves first and closes the window when the save works. When it fails, the answer
    /// asks for a second close. A close while that answer shows gives `true`.
    pub(crate) fn may_close(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        if self.closing {
            return false;
        }
        if !self.unsaved(cx) || self.message_line.read(cx).message() == Some(text::CLOSE_AGAIN) {
            return true;
        }
        self.closing = true;
        let window = window.window_handle();
        let done = step(cx, move |pane, saved: Result<(), NotSaved>, cx| {
            pane.closing = false;
            if saved.is_err() {
                pane.answer(text::CLOSE_AGAIN, cx);
            } else {
                // The close handler of the window may still run, so the window goes later.
                cx.spawn(async move |_, cx| {
                    window
                        .update(cx, |_, window, _| window.remove_window())
                        .ok();
                })
                .detach();
            }
        });
        self.core_door
            .update(cx, |core_door, cx| core_door.wait_saved(done, cx));
        false
    }

    /// Takes the end of a write of the editor text. After a failed write, the core door
    /// keeps the text, so the next job tries again.
    fn save_end(&mut self, event: &SaveEvent, cx: &mut Context<Self>) {
        match event {
            SaveEvent::Saved => {
                self.set_disk(None, cx);
                if self.unsaved(cx) {
                    self.schedule_save(cx);
                }
            }
            SaveEvent::Failed { error, first } => {
                tracing::warn!(error = %crate::chain(error), "cannot save the note");
                if *first {
                    self.answer(error.message(), cx);
                }
                self.show_state(cx);
            }
        }
        cx.notify();
    }

    /// Takes a change of the open note by another tool. [`Self::follow`] takes the moves.
    fn outside_change(&mut self, change: &Change, cx: &mut Context<Self>) {
        let Some(path) = self.content.editor() else {
            return;
        };
        let unsaved = self.core_door.read(cx).unsaved_note(path);
        match change {
            Change::Changed(changed) if changed == path => {
                if unsaved {
                    self.set_disk(Some(DiskQuestion::Changed), cx);
                } else {
                    self.set_disk(None, cx);
                    self.reload(cx);
                }
            }
            Change::Removed(gone) if gone == path => {
                if unsaved {
                    self.set_disk(Some(DiskQuestion::Deleted), cx);
                } else {
                    cx.emit(PaneEvent::Gone(gone.clone()));
                }
            }
            _ => {}
        }
    }

    /// Reads the note again, in place of its unsaved text. A failed read keeps the text.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.content.editor().cloned() else {
            return;
        };
        let opening = self.opening;
        let done = Done::value(step(cx, move |pane: &mut Self, note, cx| {
            if pane.opening == opening {
                pane.show(note, true, cx);
            }
        }));
        self.core_door.update(cx, |core_door, cx| {
            core_door.read_and_drop_text(path, done, cx);
        });
    }

    fn keep_mine(&mut self, cx: &mut Context<Self>) {
        self.set_disk(None, cx);
        self.keep_text(cx);
    }

    /// Ends the hold of the disk question, and writes the text over the file. After an outside
    /// delete, it writes the note again at its path.
    fn keep_text(&mut self, cx: &mut Context<Self>) {
        self.autosave = None;
        if let Some(path) = self.content.editor().cloned() {
            self.core_door
                .update(cx, |core_door, cx| core_door.keep_text(&path, cx));
        }
    }

    /// "Close" after an outside delete: drops the note and its text, and the app opens another
    /// note.
    fn close_deleted(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.content.editor().cloned() else {
            return;
        };
        self.drop_note(cx);
        cx.emit(PaneEvent::Gone(path));
    }

    /// The open note: the note that an open reads, or else the note in the editor, also
    /// under a new note.
    pub(crate) fn path(&self) -> Option<&NotePath> {
        self.next.as_ref().or_else(|| self.content.editor())
    }

    /// Whether the pane shows a new note with no file yet. Speed tests read it with no frame.
    #[cfg(test)]
    pub(crate) fn shows_new_note(&self) -> bool {
        matches!(self.content, Content::New { .. })
    }

    /// The note that the user sees. Speed tests read it with no frame.
    #[cfg(test)]
    pub(crate) fn shown_note(&self) -> Option<&NotePath> {
        self.content.note()
    }

    /// Opens the note at `path`, and drops a new note. With [`Leave::Keep`], the write
    /// line first writes the unsaved text. When that save fails, the pane stays on the note.
    pub(crate) fn open(&mut self, path: NotePath, leave: Leave, cx: &mut Context<Self>) {
        if leave == Leave::Keep && self.content.note() == Some(&path) {
            self.stop_opening(cx);
            return;
        }
        if let Content::New { kept, .. } = &mut self.content {
            self.content = kept.take().map_or(Content::Empty, Content::Note);
            cx.notify();
        }
        self.opening += 1;
        let opening = self.opening;
        self.next = Some(path.clone());
        cx.emit(PaneEvent::Opening(path.clone()));
        match leave {
            Leave::Keep => {
                let read = step(
                    cx,
                    move |pane: &mut Self, saved: Result<(), NotSaved>, cx| {
                        if pane.opening != opening {
                            return;
                        }
                        match (saved, pane.next.clone()) {
                            (Ok(()), Some(next)) => pane.read_note(next, opening, cx),
                            _ => pane.stop_opening(cx),
                        }
                    },
                );
                self.core_door
                    .update(cx, |core_door, cx| core_door.wait_saved(read, cx));
            }
            Leave::Drop => {
                if let Some(gone) = self.content.editor().cloned() {
                    self.core_door
                        .update(cx, |core_door, _| core_door.drop_text(&gone));
                }
                self.read_note(path, opening, cx);
            }
        }
    }

    /// Reads the note at `path` and shows it, when no newer open came after `opening`.
    fn read_note(&mut self, path: NotePath, opening: u64, cx: &mut Context<Self>) {
        // The pane owns the error: a failed read stops the open.
        let done = Done::result(step(cx, move |pane: &mut Self, note, cx| {
            if pane.opening == opening {
                match note {
                    Ok(note) => pane.opened(note, cx),
                    Err(_) => pane.stop_opening(cx),
                }
            }
        }));
        self.core_door
            .update(cx, |core_door, cx| core_door.read(path, done, cx));
    }

    fn opened(&mut self, note: Note, cx: &mut Context<Self>) {
        let path = note.info.path.clone();
        self.next = None;
        self.show(note, false, cx);
        cx.emit(PaneEvent::Opened(path));
    }

    /// Stops an open that waits, so the note in the editor is the open note again.
    fn stop_opening(&mut self, cx: &mut Context<Self>) {
        if self.next.take().is_none() {
            return;
        }
        self.opening += 1;
        if let Some(path) = self.content.editor() {
            cx.emit(PaneEvent::Opened(path.clone()));
        }
    }

    /// Follows a move of the open note, or of a folder that holds it. A move to another
    /// folder can change the file links in the note, so the pane reads it again when the note
    /// has no unsaved text.
    fn follow(&mut self, event: &FolderEvent, cx: &mut Context<Self>) {
        if let Some(next) = &mut self.next
            && let Some(new) = event.changes().find_map(|change| change.note_after(next))
        {
            next.clone_from(&new);
            cx.emit(PaneEvent::Opening(new));
        }
        let Some(path) = self.content.editor_mut() else {
            return;
        };
        let Some(new) = event.changes().find_map(|change| change.note_after(path)) else {
            return;
        };
        let other_folder = new.folder() != path.folder();
        path.clone_from(&new);
        let unsaved = self.core_door.read(cx).unsaved_note(&new);
        cx.emit(PaneEvent::Opened(new.clone()));
        if other_folder {
            let folder = Some(self.core_door.read(cx).root().join(new.folder()));
            self.editor
                .update(cx, |editor, cx| editor.set_folder(folder, cx));
        }
        if other_folder && !unsaved {
            self.reload(cx);
        } else {
            self.links.update(cx, |links, cx| links.show(Some(new), cx));
        }
    }

    /// F2: opens the title field of the open note.
    pub(crate) fn edit_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.content.note().is_some() {
            self.title.update(cx, |title, cx| title.open(window, cx));
        }
    }

    /// Gives the open note the title `title`.
    fn rename(
        &mut self,
        title: String,
        done: Then<Result<NotePath, AppError>>,
        cx: &mut Context<Self>,
    ) {
        // With no note, the title field is gone too.
        let Some(path) = self.content.note().cloned() else {
            return;
        };
        self.core_door.update(cx, |core_door, cx| {
            core_door.rename_note(path, title, Done::result(done), cx);
        });
    }

    /// Ctrl+N: after the write line writes the unsaved text, shows a new note for `folder`, with
    /// `tag` when it has one. When the save fails, the pane stays on the note. Another Ctrl+N on a new note
    /// only gives its field the focus.
    pub(crate) fn new_note(
        &mut self,
        folder: FolderPath,
        tag: Option<TagName>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Content::New { note, .. } = &self.content {
            note.update(cx, |note, cx| note.focus(window, cx));
            return;
        }
        self.stop_opening(cx);
        self.opening += 1;
        let opening = self.opening;
        let start = window_step(
            window,
            cx,
            move |pane: &mut Self, saved: Result<(), NotSaved>, window, cx| {
                if pane.opening != opening || saved.is_err() {
                    return;
                }
                pane.title.update(cx, TitleField::close);
                let core_door = pane.core_door.clone();
                let note = cx.new(|cx| NewNote::new(core_door, folder, tag, window, cx));
                let subscription = cx.subscribe_in(&note, window, Self::new_note_event);
                pane.content = Content::New {
                    note,
                    kept: pane.content.editor().cloned(),
                    _subscription: subscription,
                };
                cx.notify();
            },
        );
        self.core_door
            .update(cx, |core_door, cx| core_door.wait_saved(start, cx));
    }

    fn new_note_event(
        &mut self,
        _: &Entity<NewNote>,
        event: &NewNoteEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Content::New { kept, .. } = std::mem::replace(&mut self.content, Content::Empty) else {
            return;
        };
        window.focus(&self.editor.focus_handle(cx), cx);
        cx.notify();
        match (event, kept) {
            (NewNoteEvent::Created(note), _) => {
                self.opening += 1;
                self.opened(note.clone(), cx);
            }
            (NewNoteEvent::Cancelled, Some(kept)) => self.content = Content::Note(kept),
            (NewNoteEvent::Cancelled, None) => cx.emit(PaneEvent::Closed),
        }
    }
}

/// A `done` step for a job of the core door: it runs `step` on the pane, when the pane
/// still exists.
fn step<R: 'static>(
    cx: &Context<NotePane>,
    step: impl FnOnce(&mut NotePane, R, &mut Context<NotePane>) + 'static,
) -> Then<R> {
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
        let body = match &self.content {
            Content::New { note, .. } => note.clone().into_any_element(),
            Content::Note(_) => div()
                .flex_1()
                .min_h_0()
                .font_family(theme::NOTE_FONT)
                .text_size(theme::NOTE_SIZE)
                .line_height(theme::NOTE_LINE)
                .child(self.editor.clone())
                .into_any_element(),
            Content::Empty => empty_pane(&self.editor.focus_handle(cx), cx).into_any_element(),
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
