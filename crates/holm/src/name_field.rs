//! A field for a name that Holm may refuse, like a title or a folder name.
//!
//! The field opens for a new name or for a rename. It takes the write of its owner. `Enter`
//! sends the typed name to the write, and the field waits for its end. While it waits, the field
//! shows the typed name as plain text, and the focus goes to the home of the owner.
//!
//! - The write works: the field sends [`NameEvent::Written`] with the value of the write.
//! - Holm refuses the name: the field opens again with its text, and an answer names the
//!   reason.
//! - Another fail: an answer names it, and the field sends [`NameEvent::Closed`].
//!
//! `Enter` with the unchanged name of a rename writes nothing, and sends [`NameEvent::Closed`].
//! So does `Esc`, and the focus goes home. When the field loses the focus, it sends
//! [`NameEvent::Blur`]. While a write runs, the field sends neither.

use std::rc::Rc;

use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div,
};

use crate::error::AppError;
use crate::message_line::Answer;
use crate::notes_folder::{Step, window_step};
use crate::text_field::{self, FieldEvent, TextField};

/// Writes a typed name, and ends with the value of the write.
pub(crate) type NameWrite<T> =
    Rc<dyn Fn(String, Step<holm_core::Result<T>>, &mut Window, &mut App)>;

/// What the field tells its owner. After [`NameEvent::Written`] and [`NameEvent::Closed`], the
/// owner drops the field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NameEvent<T> {
    Written(T),
    Closed,
    Blur,
}

/// What the field does now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NameState {
    Editing,
    /// The write of this typed name runs.
    Writing(String),
}

pub(crate) struct NameField<T> {
    field: Entity<TextField>,
    /// The name before a rename.
    start: Option<String>,
    write: NameWrite<T>,
    /// It gets the focus while a write runs, and after `Esc` or the end of the field.
    home: FocusHandle,
    state: NameState,
    _subscription: Subscription,
}

/// What the field shows in its place.
pub(crate) struct Look {
    pub(crate) id: &'static str,
    pub(crate) label: SharedString,
    /// The muted text of an empty field.
    pub(crate) placeholder: SharedString,
}

impl<T: Clone + 'static> NameField<T> {
    /// Opens an empty field, and gives it the focus.
    pub(crate) fn new_name(
        look: Look,
        write: NameWrite<T>,
        home: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::open(look, None, write, home, window, cx)
    }

    /// Opens the field with the name `start`, selected, and gives it the focus.
    pub(crate) fn rename(
        look: Look,
        start: &str,
        write: NameWrite<T>,
        home: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::open(look, Some(start.to_owned()), write, home, window, cx)
    }

    fn open(
        look: Look,
        start: Option<String>,
        write: NameWrite<T>,
        home: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let text = start.as_deref().unwrap_or_default();
        let field = cx.new(|cx| {
            TextField::new(look.id, look.label, text, window, cx).placeholder(look.placeholder)
        });
        let subscription = cx.subscribe_in(&field, window, |this, _, event, window, cx| {
            this.field_event(*event, window, cx);
        });
        window.focus(&field.focus_handle(cx), cx);
        Self {
            field,
            start,
            write,
            home,
            state: NameState::Editing,
            _subscription: subscription,
        }
    }

    /// Gives the field the focus, unless a write runs.
    pub(crate) fn focus(&self, window: &mut Window, cx: &mut App) {
        if self.state == NameState::Editing {
            window.focus(&self.field.focus_handle(cx), cx);
        }
    }

    pub(crate) fn state(&self) -> &NameState {
        &self.state
    }

    fn field_event(&mut self, event: FieldEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.state != NameState::Editing {
            return;
        }
        match event {
            FieldEvent::Enter => self.submit(window, cx),
            FieldEvent::Escape => {
                window.focus(&self.home, cx);
                cx.emit(NameEvent::Closed);
            }
            // A refused name gives the field the focus again before the blur comes.
            FieldEvent::Blur if !self.field.focus_handle(cx).is_focused(window) => {
                cx.emit(NameEvent::Blur);
            }
            FieldEvent::Blur => {}
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.field.read(cx).text().to_owned();
        window.focus(&self.home, cx);
        if self.start.as_deref() == Some(name.trim()) {
            cx.emit(NameEvent::Closed);
            return;
        }
        self.state = NameState::Writing(name.clone());
        cx.notify();
        let ended = window_step(window, cx, Self::ended);
        // A write may end at once, and the field can't take its end in its own update.
        let done: Step<holm_core::Result<T>> =
            Box::new(move |result, cx| cx.defer(move |cx| ended(result, cx)));
        let write = self.write.clone();
        window.defer(cx, move |window, cx| write(name, done, window, cx));
    }

    fn ended(&mut self, result: holm_core::Result<T>, window: &mut Window, cx: &mut Context<Self>) {
        self.state = NameState::Editing;
        cx.notify();
        let error = match result {
            Ok(value) => {
                cx.emit(NameEvent::Written(value));
                return;
            }
            Err(error) => error,
        };
        let refused = matches!(
            error,
            holm_core::Error::NameTaken { .. } | holm_core::Error::BadName { .. }
        );
        let error = AppError::from(error);
        tracing::warn!(error = %crate::chain(&error), "cannot write the name");
        window.dispatch_action(Box::new(Answer(error.message())), cx);
        if refused {
            self.focus(window, cx);
        } else {
            cx.emit(NameEvent::Closed);
        }
    }
}

impl<T: 'static> EventEmitter<NameEvent<T>> for NameField<T> {}

impl<T: 'static> Render for NameField<T> {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        match &self.state {
            NameState::Writing(name) => div()
                .min_w_0()
                .overflow_hidden()
                .child(text_field::label(name.clone()))
                .into_any_element(),
            NameState::Editing => self.field.clone().into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::path::PathBuf;

    use gpui_kit::{TestAppContext, VisualTestContext};

    use super::*;

    type Events = Rc<RefCell<Vec<NameEvent<String>>>>;

    /// A write that refuses the first name, and then writes each name.
    fn refuse_once() -> NameWrite<String> {
        let calls = Rc::new(Cell::new(0));
        Rc::new(move |name, done, _, cx| {
            calls.set(calls.get() + 1);
            let result = if calls.get() == 1 {
                Err(holm_core::Error::NameTaken {
                    path: PathBuf::from(&name),
                })
            } else {
                Ok(name)
            };
            done(result, cx);
        })
    }

    fn field(
        cx: &mut TestAppContext,
    ) -> (Entity<NameField<String>>, Events, &mut VisualTestContext) {
        cx.update(gpui_kit::init);
        let look = Look {
            id: "name",
            label: "Name".into(),
            placeholder: SharedString::default(),
        };
        let (field, cx) = cx.add_window_view(|window, cx| {
            let home = cx.focus_handle();
            NameField::rename(look, "Old", refuse_once(), home, window, cx)
        });
        let events: Events = Rc::default();
        let seen = Rc::clone(&events);
        cx.update(|_, cx| {
            cx.subscribe(&field, move |_, event: &NameEvent<String>, _| {
                seen.borrow_mut().push(event.clone());
            })
            .detach();
        });
        (field, events, cx)
    }

    #[gpui_kit::test]
    fn a_refused_name_keeps_its_text_and_the_next_write_closes_the_field(cx: &mut TestAppContext) {
        let (field, events, cx) = field(cx);

        cx.simulate_input("Taken");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        field.read_with(cx, |field, cx| {
            assert_eq!(field.state(), &NameState::Editing);
            assert_eq!(field.field.read(cx).text(), "Taken");
        });
        assert!(events.borrow().is_empty());

        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(*events.borrow(), [NameEvent::Written("Taken".to_owned())]);
    }

    #[gpui_kit::test]
    fn the_unchanged_name_closes_the_field_with_no_write(cx: &mut TestAppContext) {
        let (_, events, cx) = field(cx);

        cx.simulate_input(" Old ");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(*events.borrow(), [NameEvent::Closed]);
    }
}
