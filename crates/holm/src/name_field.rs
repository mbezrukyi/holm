//! A field for a name that Holm may refuse, like a title or a folder name.
//!
//! The field takes the write of its owner. `Enter` sends the typed name to the write, and the
//! field waits for its end. While it waits, the field shows the typed name as plain text, and
//! the focus goes to the home of the owner.
//!
//! - The write works: the field sends [`NameEvent::Done`].
//! - Holm refuses the name: the field opens again with its text, and an answer names the
//!   reason.
//! - Another fail: an answer names it, and the field sends [`NameEvent::Done`].
//!
//! `Enter` with the unchanged name of a rename writes nothing, and sends
//! [`NameEvent::Done`]. `Esc` sends [`NameEvent::Escape`], and the focus goes home. When the
//! field loses the focus, it sends [`NameEvent::Blur`]. While a write runs, the field sends
//! neither.

use std::rc::Rc;

use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div,
};

use crate::error::AppError;
use crate::message_line::MessageLine;
use crate::notes_folder::{Step, window_step};
use crate::text_field::{FieldEvent, TextField};

/// The end of a name write.
pub(crate) type NameDone = Step<holm_core::Result<()>>;

/// Writes a typed name. It must end with `done`.
pub(crate) type NameWrite = Rc<dyn Fn(String, NameDone, &mut Window, &mut App)>;

/// What the field tells its owner. After [`NameEvent::Done`], the owner drops the field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NameEvent {
    Done,
    Escape,
    Blur,
}

pub(crate) struct NameField {
    field: Entity<TextField>,
    /// The name before the rename. It is empty for a new name.
    start: String,
    write: NameWrite,
    line: Entity<MessageLine>,
    /// It gets the focus while a write runs, and after `Esc` or the end of the field.
    home: FocusHandle,
    /// The typed name while its write runs.
    pending: Option<String>,
    _subscription: Subscription,
}

/// What the field shows in its place.
pub(crate) struct Look {
    pub(crate) id: &'static str,
    pub(crate) label: SharedString,
    /// The muted text of an empty field.
    pub(crate) placeholder: SharedString,
}

impl NameField {
    /// Opens the field with `start`, selected, and gives it the focus.
    pub(crate) fn new(
        look: Look,
        start: &str,
        write: NameWrite,
        line: Entity<MessageLine>,
        home: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let field = cx.new(|cx| {
            TextField::new(look.id, look.label, start, window, cx).placeholder(look.placeholder)
        });
        let subscription = cx.subscribe_in(&field, window, |this, _, event, window, cx| {
            this.field_event(*event, window, cx);
        });
        window.focus(&field.focus_handle(cx), cx);
        Self {
            field,
            start: start.to_owned(),
            write,
            line,
            home,
            pending: None,
            _subscription: subscription,
        }
    }

    /// Gives the field the focus, unless a write runs.
    pub(crate) fn focus(&self, window: &mut Window, cx: &mut App) {
        if self.pending.is_none() {
            window.focus(&self.field.focus_handle(cx), cx);
        }
    }

    /// The typed name while its write runs.
    pub(crate) fn pending(&self) -> Option<&str> {
        self.pending.as_deref()
    }

    fn field_event(&mut self, event: FieldEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        match event {
            FieldEvent::Enter => self.submit(window, cx),
            FieldEvent::Escape => {
                window.focus(&self.home, cx);
                cx.emit(NameEvent::Escape);
            }
            // A refused name gives the field the focus again before the blur comes.
            FieldEvent::Blur if !self.field.focus_handle(cx).is_focused(window) => {
                cx.emit(NameEvent::Blur);
            }
            FieldEvent::Blur => {}
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.field.read(cx).text().trim().to_owned();
        window.focus(&self.home, cx);
        if !self.start.is_empty() && name == self.start {
            cx.emit(NameEvent::Done);
            return;
        }
        self.pending = Some(name.clone());
        cx.notify();
        let ended = window_step(window, cx, Self::ended);
        // A write may end at once, and the field can't take its end in its own update.
        let done: NameDone = Box::new(move |result, cx| cx.defer(move |cx| ended(result, cx)));
        let write = self.write.clone();
        window.defer(cx, move |window, cx| write(name, done, window, cx));
    }

    fn ended(
        &mut self,
        result: holm_core::Result<()>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending = None;
        cx.notify();
        let Err(error) = result else {
            cx.emit(NameEvent::Done);
            return;
        };
        let refused = matches!(
            error,
            holm_core::Error::NameTaken { .. } | holm_core::Error::BadName { .. }
        );
        let error = AppError::from(error);
        tracing::warn!(error = %crate::chain(&error), "cannot write the name");
        self.line
            .update(cx, |line, cx| line.show(error.message(), cx));
        if refused {
            self.focus(window, cx);
        } else {
            cx.emit(NameEvent::Done);
        }
    }
}

impl EventEmitter<NameEvent> for NameField {}

impl Render for NameField {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        match &self.pending {
            Some(name) => div()
                .min_w_0()
                .truncate()
                .child(name.clone())
                .into_any_element(),
            None => self.field.clone().into_any_element(),
        }
    }
}
