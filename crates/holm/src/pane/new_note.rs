//! The note after Ctrl+N, before it has a title and a file.
//!
//! It shows in the place of the editor: the tag row with the tag of the note and an empty
//! title field. The pane puts it in the text column, so it is at the same place as in the editor. Nothing else takes a click or a key.
//!
//! - `Enter` writes `<title>.md`, and sends [`NewNoteEvent::Created`].
//! - `Esc`, or a fail that is not about the title, sends [`NewNoteEvent::Cancelled`].
//!
//! The note takes the focus while the create job runs, because the editor is hidden.
//! - A refused title keeps the field open.

use std::rc::Rc;

use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Render, Role, StatefulInteractiveElement as _,
    Styled as _, Subscription, TestSupportExt as _, Window, div,
};
use holm_core::model::Note;
use holm_core::{FolderPath, TagName};

use crate::core_door::{CoreDoor, Done};
use crate::name_field::{NameEvent, NameWrite};
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
    tag: Option<TagName>,
    title: Entity<TitleField<Note>>,
    focus: FocusHandle,
    _subscription: Subscription,
}

impl NewNote {
    /// A note for `folder` with `tag`, with the focus in its title field.
    pub(crate) fn new(
        core_door: Entity<CoreDoor>,
        folder: FolderPath,
        tag: Option<TagName>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let create_tag = tag.clone();
        let write: NameWrite<Note> = Rc::new(move |title, done, _, cx| {
            let (folder, tag) = (folder.clone(), create_tag.clone());
            core_door.update(cx, |core_door, cx| {
                core_door.create_note(folder, title, tag, Done::result(done), cx);
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
        let row = tags::new_note_row(self.tag.as_ref(), self.title.clone().into_any_element(), cx);
        div()
            .size_full()
            .child(row)
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
