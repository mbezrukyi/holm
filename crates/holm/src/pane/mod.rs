//! The editor pane: one note in the editor, with autosave. With no open note, it shows the empty pane.
//!
//! The pane keeps its unsaved text in the draft of the core door. The write line writes the draft before each job, so
//! each file action sees the text. The pane also saves a short time after the user stops typing, and when the app
//! quits. A close of the window saves first. When that save fails, a second close closes the window while the text
//! stays the same. An outside change of the note reloads it quietly when the pane has no unsaved changes. Else the
//! message line asks to reload or to keep the text in the editor. When a save fails, the message line shows "Not
//! saved", and the text stays in the editor. When another tool deletes the note, the app opens another one. With
//! unsaved changes, the message line first asks to write the note again or to close it.
//!
//! When the user opens another note, the pane saves the note first. When that save fails, the pane stays on the note.
//! Holm never renames a note by itself.
//!
//! The title field in the tag row renames the note. Ctrl+N shows a [`NewNote`] in the place of the editor. Its file
//! comes only with its title. The editor keeps the note from before, so `Esc` shows it again as it was.
//!
//! The tag row before the first line shows the tags, the pin and the path of the note. It scrolls with the text. The
//! pane writes the changes of the tags and the pin at once. The editor undo stack holds only the text, so Ctrl+Z does
//! not undo them.
//!
//! `links` runs the link picker flow, opens the links of the note and reads its backlinks. The backlinks show in the
//! tag row.

mod links;
mod new_note;
mod open;

use std::borrow::Cow;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::{Theme as Ui, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Div, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement, Render, Role, StatefulInteractiveElement as _, Styled,
    Subscription, Task, TestSupportExt as _, Window, div, px,
};
use holm_core::model::{Change, Note, NoteInfo, Source};
use holm_core::{FolderPath, NotePath, TagName};
use holm_editor::{Changed, Editor, UpFromStart};

use crate::backlinks::{Backlinks, OpenBacklink};
use crate::config::Config;
use crate::core_door::{CoreDoor, Done, DoorEvent, DraftVersion, NotSaved, SaveEvent, Then};
use crate::error::AppError;
use crate::link_picker::{LinkChoice, LinkStart};
use crate::message_line::MessageLine;
use crate::name_field::{NameEvent, NameWrite};
use crate::state_menu::{State, StatePick};
use crate::tags::{TagChange, TagRow};
use crate::title::TitleField;
use crate::{text, theme};
pub(crate) use links::Pick;
use links::{Links, LinksEvent};
use new_note::{NewNote, NewNoteEvent};
use open::{Open, Opens};

/// How long the pane waits after the last change before it saves.
const AUTOSAVE: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PaneEvent {
    /// The pane starts to read this note. It is the open note from now on.
    Opening(NotePath),
    /// The pane shows the note at this path: after an open, a move, or an open that stopped.
    Opened(NotePath),
    Closed,
    /// The open note at this path is gone, by a Holm delete or another tool, and the pane has no text to keep. The app
    /// must open another note or call [`NotePane::close`]. The pane cannot pick the note, because the neighbor comes
    /// from the sidebar list.
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

/// A question about the open note after another tool changed its file, while the editor had unsaved changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DiskQuestion {
    Changed,
    Deleted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    KeepMine,
    Reload,
    Close,
}

impl DiskQuestion {
    /// In the state menu. The first one starts under the cursor.
    fn choices(self) -> [Choice; 2] {
        match self {
            Self::Changed => [Choice::KeepMine, Choice::Reload],
            Self::Deleted => [Choice::KeepMine, Choice::Close],
        }
    }

    fn state(self) -> State {
        let (label, reason) = match self {
            Self::Changed => (text::CHANGED_ON_DISK, text::CHANGED_REASON),
            Self::Deleted => (text::DELETED_ON_DISK, text::DELETED_REASON),
        };
        let choices = self.choices().map(|choice| match choice {
            Choice::KeepMine => text::KEEP_MINE,
            Choice::Reload => text::RELOAD,
            Choice::Close => text::CLOSE,
        });
        State {
            label: label.into(),
            reason: reason.into(),
            choices: choices.into_iter().map(Into::into).collect(),
        }
    }
}

enum Content {
    Empty,
    Note(NotePath),
    /// The note after Ctrl+N, before it has a file, in the place of the editor. The editor keeps the note from before,
    /// at `kept`.
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

/// How [`NotePane::show`] puts a note into the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Load {
    /// A new note, with its own view.
    Open,
    /// The open note again from the disk: it keeps the view and is one undo step.
    Reload,
}

pub(crate) struct NotePane {
    core_door: Entity<CoreDoor>,
    message_line: Entity<MessageLine>,
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
    opens: Opens,
    /// The heading to show when the note that opens is ready.
    open_heading: Option<String>,
    is_saving_for_close: bool,
    /// The draft version at a close that failed to save. A close at the same version closes, so typed text stops it.
    close_again: Option<DraftVersion>,
    _subscriptions: Vec<Subscription>,
}

impl NotePane {
    /// The editor takes the writing help of `config`, also after each change.
    pub(crate) fn new(
        core_door: Entity<CoreDoor>,
        config: &Entity<Config>,
        line: Entity<MessageLine>,
        note: Option<Note>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(Editor::new);
        let text = editor.focus_handle(cx);
        let backlinks = cx.new(|cx| Backlinks::new(text.clone(), window, cx));
        let pane = cx.weak_entity();
        let rename: NameWrite<NotePath> = Rc::new(move |title, done, _, cx| {
            pane.update(cx, |pane, cx| pane.rename(title, done, cx)).ok();
        });
        let title = cx.new(|_| TitleField::new(rename, text.clone()));
        let tag_row = cx.new(|cx| TagRow::new(core_door.clone(), title.clone(), backlinks.clone(), text, window, cx));
        editor.update(cx, |editor, cx| {
            editor.set_start(Some(tag_row.clone().into()), cx);
        });
        let links = cx.new(|cx| Links::new(core_door.clone(), editor.clone(), backlinks.clone(), cx));
        let subscriptions = vec![
            cx.subscribe(&core_door, |pane, _, event: &DoorEvent, cx| {
                pane.follow(event, cx);
                for (source, change) in event.sourced() {
                    pane.take_change(source, change, cx);
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
            cx.subscribe(&line, |pane, _, StatePick(at): &StatePick, cx| {
                pane.choose(*at, cx);
            }),
            cx.subscribe_in(&backlinks, window, |pane, _, OpenBacklink(path), window, cx| {
                window.focus(&pane.editor.focus_handle(cx), cx);
                pane.open(path.clone(), Leave::Keep, cx);
            }),
            Self::take_settings(&editor, config, cx),
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
            opens: Opens::default(),
            open_heading: None,
            is_saving_for_close: false,
            close_again: None,
            _subscriptions: subscriptions,
        };
        if let Some(note) = note {
            pane.show(note, Load::Open, cx);
        }
        pane
    }

    /// Puts `note` from the disk into the editor.
    fn show(&mut self, note: Note, load: Load, cx: &mut Context<Self>) {
        self.ending = LineEnding::of(&note.body);
        let folder = Some(self.core_door.read(cx).root().join(note.info.path.folder()));
        self.editor.update(cx, |editor, cx| match load {
            Load::Open => {
                editor.set_text(&note.body, cx);
                editor.set_folder(folder, cx);
            }
            Load::Reload => editor.reload_text(&note.body, cx),
        });
        let path = note.info.path.clone();
        match load {
            Load::Open => self.content = Content::Note(path.clone()),
            Load::Reload => {
                if let Some(open) = self.content.editor_mut() {
                    open.clone_from(&path);
                }
            }
        }
        self.last_info = Some(note.info);
        self.show_fields(cx);
        self.set_disk(None, cx);
        if load == Load::Open
            && let Some(heading) = self.open_heading.take()
        {
            self.editor.update(cx, |editor, cx| editor.show_heading(&heading, cx));
        }
        self.links.update(cx, |links, cx| links.show(Some(path), cx));
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
            self.core_door.update(cx, |core_door, _| core_door.drop_text(&path));
        }
        self.title.update(cx, TitleField::close);
        self.content = Content::Empty;
        self.next = None;
        self.last_info = None;
        self.autosave = None;
        self.opens.cancel();
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
        self.message_line.update(cx, |line, cx| line.show(message, cx));
    }

    /// A question holds the text of the note in the draft.
    fn set_disk(&mut self, disk: Option<DiskQuestion>, cx: &mut Context<Self>) {
        if disk.is_some() {
            self.hold_text(cx);
        }
        self.disk = disk;
        self.show_state(cx);
    }

    fn hold_text(&self, cx: &mut Context<Self>) {
        if let Some(path) = self.content.editor() {
            self.core_door.update(cx, |core_door, _| core_door.hold_text(path));
        }
    }

    /// Shows the state in the message line. A disk question wins over "Not saved", which has only its reason.
    fn show_state(&self, cx: &mut Context<Self>) {
        let state = match (self.disk, self.core_door.read(cx).not_saved()) {
            (Some(question), _) => Some(question.state()),
            (None, Some(reason)) => Some(State {
                label: text::NOT_SAVED.into(),
                reason: reason.into(),
                choices: Vec::new(),
            }),
            (None, None) => None,
        };
        self.message_line.update(cx, |line, cx| line.set_state(state, cx));
    }

    /// Takes the pick at `at` in the state menu of the disk question.
    fn choose(&mut self, at: usize, cx: &mut Context<Self>) {
        let Some(question) = self.disk else {
            return;
        };
        let Some(choice) = question.choices().get(at).copied() else {
            return;
        };
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

    /// Ctrl+K: gives `done` what the link picker starts with. See [`Links::start`]. With no note in the editor, it does
    /// nothing.
    pub(crate) fn start_link(&mut self, done: impl FnOnce(LinkStart, &mut App) + 'static, cx: &mut Context<Self>) {
        if self.content.note().is_some() {
            self.links.update(cx, |links, cx| links.start(done, cx));
        }
    }

    /// Puts the link that the user picked in the link picker at the cursor. See [`Links::link`]. With no note in the
    /// editor, it does nothing. `done` runs at the end.
    pub(crate) fn link(&mut self, choice: LinkChoice, done: Then<()>, cx: &mut Context<Self>) {
        if self.content.note().is_some() {
            self.links.update(cx, |links, cx| links.link(choice, done, cx));
        } else {
            cx.defer(move |cx| done((), cx));
        }
    }

    /// Shows the tags and the pin of the open note from the snapshot.
    fn show_fields(&mut self, cx: &mut Context<Self>) {
        let path = self.content.editor().cloned();
        self.title.update(cx, |title, cx| title.show(path.clone(), cx));
        let Some(path) = path else {
            return;
        };
        if let Some(info) = self.core_door.read(cx).snapshot().note(&path) {
            self.last_info = Some(info.clone());
        }
        if let Some(info) = &self.last_info {
            self.tag_row.update(cx, |row, cx| row.show(&info.tags, info.pinned, cx));
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
        self.backlinks.update(cx, |backlinks, cx| backlinks.enter(window, cx));
    }

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
            let body = self.ending.to_disk(self.editor.read(cx).text()).into_owned();
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

    /// A close of the window: gives whether the window may close now. With unsaved text, the pane saves first and
    /// closes the window when the save works. When it fails, the answer asks for a second close. A close with the same
    /// text as at that failure gives `true`.
    pub(crate) fn may_close(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        if self.is_saving_for_close {
            return false;
        }
        let version = self.core_door.read(cx).draft_version();
        if !self.unsaved(cx) || self.close_again == Some(version) {
            return true;
        }
        self.is_saving_for_close = true;
        let window = window.window_handle();
        let done = step(cx, move |pane, saved: Result<(), NotSaved>, cx| {
            pane.is_saving_for_close = false;
            if saved.is_err() {
                pane.close_again = Some(version);
                pane.answer(text::CLOSE_AGAIN, cx);
            } else {
                // The close handler of the window may still run, so the window goes later.
                cx.spawn(async move |_, cx| {
                    window.update(cx, |_, window, _| window.remove_window()).ok();
                })
                .detach();
            }
        });
        self.core_door
            .update(cx, |core_door, cx| core_door.wait_saved(done, cx));
        false
    }

    /// Gives the writing help and the text width of `config` to `editor`, now and after each change.
    fn take_settings(editor: &Entity<Editor>, config: &Entity<Config>, cx: &mut Context<Self>) -> Subscription {
        let give = |editor: &Entity<Editor>, config: &Entity<Config>, cx: &mut Context<Self>| {
            let settings = config.read(cx).settings();
            let (help, width) = (settings.writing_help, settings.text_width.text_width());
            editor.update(cx, |editor, cx| {
                editor.set_writing_help(help);
                editor.set_text_width(width, cx);
            });
        };
        give(editor, config, cx);
        cx.observe(config, move |pane, config, cx| {
            let editor = pane.editor.clone();
            give(&editor, &config, cx);
            // A new note lines up with the text column.
            cx.notify();
        })
    }

    /// After a failed write, the core door keeps the text, so the next job tries again.
    fn save_end(&mut self, event: &SaveEvent, cx: &mut Context<Self>) {
        match event {
            SaveEvent::Saved => {
                self.set_disk(None, cx);
                if self.unsaved(cx) {
                    self.schedule_save(cx);
                }
            }
            SaveEvent::Failed => self.show_state(cx),
            SaveEvent::Changed => self.set_disk(Some(DiskQuestion::Changed), cx),
        }
        cx.notify();
    }

    /// Takes a change of the open note. Only a change by another tool can ask the disk question. [`Self::follow`] takes
    /// the moves.
    fn take_change(&mut self, source: Source, change: &Change, cx: &mut Context<Self>) {
        let Some(path) = self.content.editor() else {
            return;
        };
        let outside = source == Source::Outside;
        let unsaved = outside && self.core_door.read(cx).unsaved_note(path);
        match change {
            Change::Changed(changed) if outside && changed == path => {
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
        let open = self.opens.current();
        let done = Done::value(open.step(cx, |pane, note, cx| pane.show(note, Load::Reload, cx)));
        self.core_door.update(cx, |core_door, cx| {
            core_door.read_and_drop_text(path, done, cx);
        });
    }

    fn keep_mine(&mut self, cx: &mut Context<Self>) {
        self.set_disk(None, cx);
        self.keep_text(cx);
    }

    /// Ends the hold of the disk question, and writes the text over the file. After an outside delete, it writes the
    /// note again at its path.
    fn keep_text(&mut self, cx: &mut Context<Self>) {
        self.autosave = None;
        if let Some(path) = self.content.editor().cloned() {
            self.core_door
                .update(cx, |core_door, cx| core_door.keep_text(&path, cx));
        }
    }

    /// "Close" after an outside delete: drops the note and its text, and the app opens another note.
    fn close_deleted(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.content.editor().cloned() else {
            return;
        };
        self.drop_note(cx);
        cx.emit(PaneEvent::Gone(path));
    }

    /// The open note: the note that an open reads, or else the note in the editor, also under a new note.
    pub(crate) fn path(&self) -> Option<&NotePath> {
        self.next.as_ref().or_else(|| self.content.editor())
    }

    /// Speed tests read it with no frame.
    #[cfg(test)]
    pub(crate) fn shows_new_note(&self) -> bool {
        matches!(self.content, Content::New { .. })
    }

    /// Speed tests read it with no frame.
    #[cfg(test)]
    pub(crate) fn shown_note(&self) -> Option<&NotePath> {
        self.content.note()
    }

    /// Opens the note at `path`, and drops a new note. With [`Leave::Keep`], the write line first writes the unsaved
    /// text. When that save fails, the pane stays on the note.
    pub(crate) fn open(&mut self, path: NotePath, leave: Leave, cx: &mut Context<Self>) {
        if leave == Leave::Keep && self.content.note() == Some(&path) {
            self.stop_opening(cx);
            return;
        }
        if let Content::New { kept, .. } = &mut self.content {
            self.content = kept.take().map_or(Content::Empty, Content::Note);
            cx.notify();
        }
        let open = self.opens.start();
        self.next = Some(path.clone());
        cx.emit(PaneEvent::Opening(path.clone()));
        match leave {
            Leave::Keep => {
                let read = open.step(cx, move |pane, saved: Result<(), NotSaved>, cx| {
                    match (saved, pane.next.clone()) {
                        (Ok(()), Some(next)) => pane.read_note(next, open, cx),
                        _ => pane.stop_opening(cx),
                    }
                });
                self.core_door
                    .update(cx, |core_door, cx| core_door.wait_saved(read, cx));
            }
            Leave::Drop => {
                if let Some(gone) = self.content.editor().cloned() {
                    self.core_door.update(cx, |core_door, _| core_door.drop_text(&gone));
                }
                self.read_note(path, open, cx);
            }
        }
    }

    /// Reads the note at `path` and shows it, when no newer open came after `open`.
    fn read_note(&mut self, path: NotePath, open: Open, cx: &mut Context<Self>) {
        // The pane owns the error: a failed read stops the open.
        let done = Done::result(open.step(cx, |pane, note, cx| match note {
            Ok(note) => pane.opened(note, cx),
            Err(_) => pane.stop_opening(cx),
        }));
        self.core_door
            .update(cx, |core_door, cx| core_door.read(path, done, cx));
    }

    fn opened(&mut self, note: Note, cx: &mut Context<Self>) {
        let path = note.info.path.clone();
        self.next = None;
        self.show(note, Load::Open, cx);
        cx.emit(PaneEvent::Opened(path));
    }

    /// Stops an open that waits, so the note in the editor is the open note again.
    fn stop_opening(&mut self, cx: &mut Context<Self>) {
        if self.next.take().is_none() {
            return;
        }
        self.opens.cancel();
        if let Some(path) = self.content.editor() {
            cx.emit(PaneEvent::Opened(path.clone()));
        }
    }

    /// Follows a move of the open note, or of a folder that holds it. A move to another folder can change the file
    /// links in the note, so the pane reads it again when the note has no unsaved text.
    fn follow(&mut self, event: &DoorEvent, cx: &mut Context<Self>) {
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
            self.editor.update(cx, |editor, cx| editor.set_folder(folder, cx));
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

    fn rename(&mut self, title: String, done: Then<Result<NotePath, AppError>>, cx: &mut Context<Self>) {
        // With no note, the title field is gone too.
        let Some(path) = self.content.note().cloned() else {
            return;
        };
        self.core_door.update(cx, |core_door, cx| {
            core_door.rename_note(path, title, Done::result(done), cx);
        });
    }

    /// Ctrl+N: after the write line writes the unsaved text, shows a new note for `folder`, with `tag` when it has one.
    /// When the save fails, the pane stays on the note. Another Ctrl+N on a new note only gives its field the focus.
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
        let open = self.opens.start();
        let start = open.window_step(window, cx, move |pane, saved: Result<(), NotSaved>, window, cx| {
            if saved.is_err() {
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
        });
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
                self.opens.cancel();
                self.opened(note.clone(), cx);
            }
            (NewNoteEvent::Cancelled, Some(kept)) => self.content = Content::Note(kept),
            (NewNoteEvent::Cancelled, None) => cx.emit(PaneEvent::Closed),
        }
    }
}

/// A `done` step for a job of the core door: it runs `step` on the pane, when the pane still exists.
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
            // In the text style of the editor, the text column has the same width.
            Content::New { note, .. } => note_text()
                .child(holm_editor::text_column(
                    self.editor.read(cx).text_width(),
                    note.clone(),
                ))
                .into_any_element(),
            Content::Note(_) => note_text().child(self.editor.clone()).into_any_element(),
            Content::Empty => empty_pane(&self.editor.focus_handle(cx), cx).into_any_element(),
        };
        v_flex().size_full().bg(cx.theme().background).child(body)
    }
}

/// The place of the editor, with the text style of a note.
fn note_text() -> Div {
    div()
        .flex_1()
        .min_h_0()
        .font_family(theme::NOTE_FONT)
        .text_size(theme::NOTE_SIZE)
        .line_height(theme::NOTE_LINE)
}

/// The pane with no open note: a line, and the keys that open one. It holds the focus of the editor, so the window keys
/// still work when no other part has the focus.
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

/// The editor works with LF only.
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
mod tests;
