//! The note after Ctrl+N, before it has a title and a file.
//!
//! It shows in the place of the editor: the tag row with the tag of the note and an empty
//! title field, at the same place as in the editor. Nothing else takes a click or a key.
//!
//! - `Enter` writes `<title>.md`, and sends [`NewNoteEvent::Created`].
//! - `Esc`, or a fail that is not about the title, sends [`NewNoteEvent::Cancelled`].
//! - A refused title keeps the field open.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Render, Role, StatefulInteractiveElement as _,
    Styled as _, Subscription, TestSupportExt as _, Window, div,
};
use holm_editor::{COLUMN_WIDTH, SIDE_MARGIN};

use crate::message_line::MessageLine;
use crate::name_field::{NameEvent, NameWrite};
use crate::notes_folder::{NotesFolder, window_step};
use crate::tags;
use crate::text;
use crate::title::TitleField;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NewNoteEvent {
    /// The note has its file at this path.
    Created(PathBuf),
    /// No file came. The pane opens `before` again, the note that was open before.
    Cancelled { before: Option<PathBuf> },
}

pub(crate) struct NewNote {
    tag: Option<String>,
    before: Option<PathBuf>,
    title: Entity<TitleField>,
    /// The path of the file after the create write.
    created: Option<PathBuf>,
    _subscription: Subscription,
}

impl NewNote {
    /// A note for `folder` with `tag`, with the focus in its title field. `home` gets the focus
    /// when the field closes.
    #[allow(clippy::too_many_arguments, reason = "the parts of one new note")]
    pub(crate) fn new(
        notes_folder: Entity<NotesFolder>,
        folder: PathBuf,
        tag: Option<String>,
        before: Option<PathBuf>,
        line: Entity<MessageLine>,
        home: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let this = cx.weak_entity();
        let create_tag = tag.clone();
        let write: NameWrite = Rc::new(move |title, done, window, cx| {
            let Ok(created) = this.update(cx, |_, cx| {
                window_step(
                    window,
                    cx,
                    move |note: &mut Self, result: holm_core::Result<PathBuf>, _, cx| {
                        done(result.map(|path| note.created = Some(path)), cx);
                    },
                )
            }) else {
                return;
            };
            let (folder, tag) = (folder.clone(), create_tag.clone());
            notes_folder.update(cx, |notes_folder, cx| {
                notes_folder.create_note(folder, title, tag, created, cx);
            });
        });
        let title = cx.new(|_| TitleField::new(write, line, home));
        let subscription = cx.subscribe(&title, |note, _, event: &NameEvent, cx| match event {
            NameEvent::Done => match note.created.clone() {
                Some(path) => cx.emit(NewNoteEvent::Created(path)),
                None => cx.emit(NewNoteEvent::Cancelled {
                    before: note.before.clone(),
                }),
            },
            NameEvent::Escape => cx.emit(NewNoteEvent::Cancelled {
                before: note.before.clone(),
            }),
            // A click elsewhere loses nothing.
            NameEvent::Blur => {}
        });
        title.update(cx, |title, cx| title.open(window, cx));
        Self {
            tag,
            before,
            title,
            created: None,
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
        div()
            .id("new-note")
            .role(Role::Group)
            .aria_label(text::NEW_NOTE)
            .test_support()
            .size_full()
            .flex()
            .justify_center()
            .px(SIDE_MARGIN)
            // The note has no text yet, so a click must not move the focus.
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .child(div().w_full().max_w(COLUMN_WIDTH).child(tags::new_note_row(
                self.tag.as_deref(),
                self.title.clone(),
                cx,
            )))
    }
}
