//! The note after Ctrl+N, before it has a title and a file.
//!
//! It shows in the place of the editor: the tag row with the tag of the note and an empty
//! title field, at the same place as in the editor. Nothing else takes a click or a key.
//!
//! - `Enter` writes `<title>.md`, and sends [`NewNoteEvent::Created`].
//! - `Esc`, or a fail that is not about the title, sends [`NewNoteEvent::Cancelled`].
//!
//! The note takes the focus while the create write runs, because the editor is hidden.
//! - A refused title keeps the field open.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, InteractiveElement as _,
    IntoElement, MouseButton, Render, Role, StatefulInteractiveElement as _, Subscription,
    TestSupportExt as _, Window,
};
use holm_core::model::Note;

use crate::name_field::{NameEvent, NameWrite};
use crate::notes_folder::NotesFolder;
use crate::tags;
use crate::text;
use crate::title::TitleField;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NewNoteEvent {
    /// The note has its file.
    Created(Note),
    /// No file came.
    Cancelled,
}

pub(crate) struct NewNote {
    tag: Option<String>,
    title: Entity<TitleField<Note>>,
    focus: FocusHandle,
    _subscription: Subscription,
}

impl NewNote {
    /// A note for `folder` with `tag`, with the focus in its title field.
    pub(crate) fn new(
        notes_folder: Entity<NotesFolder>,
        folder: PathBuf,
        tag: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let create_tag = tag.clone();
        let write: NameWrite<Note> = Rc::new(move |title, done, _, cx| {
            let (folder, tag) = (folder.clone(), create_tag.clone());
            notes_folder.update(cx, |notes_folder, cx| {
                notes_folder.create_note(folder, title, tag, done, cx);
            });
        });
        let focus = cx.focus_handle();
        let title = cx.new(|_| TitleField::new(write, focus.clone()));
        let subscription = cx.subscribe(&title, |_, _, event: &NameEvent<Note>, cx| match event {
            NameEvent::Written(note) => cx.emit(NewNoteEvent::Created(note.clone())),
            NameEvent::Closed => cx.emit(NewNoteEvent::Cancelled),
            // A click elsewhere loses nothing.
            NameEvent::Blur => {}
        });
        title.update(cx, |title, cx| title.open(window, cx));
        Self {
            tag,
            title,
            focus,
            _subscription: subscription,
        }
    }

    /// Gives the title field the focus.
    pub(crate) fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.title.update(cx, |title, cx| title.open(window, cx));
    }
}

impl EventEmitter<NewNoteEvent> for NewNote {}

impl Render for NewNote {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row = tags::new_note_row(
            self.tag.as_deref(),
            self.title.clone().into_any_element(),
            cx,
        );
        holm_editor::text_column(row)
            .id("new-note")
            .role(Role::Group)
            .aria_label(text::NEW_NOTE)
            .test_support()
            .track_focus(&self.focus)
            // The note has no text yet, so a click must not move the focus.
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
    }
}
