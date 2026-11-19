//! A field for a name that Holm may refuse, like a title or a folder name.
//!
//! The field opens for a new name or for a rename. It takes the job of its owner. `Enter`
//! sends the typed name to the job, and the field waits for its end. While it waits, the field
//! keeps its look and the focus, and takes no keys.
//!
//! - The job works: the focus goes to the home of the owner, and the field sends
//!   [`NameEvent::Written`] with the value of the job.
//! - Holm refuses the name: the field opens again with its text.
//! - Another fail: the field sends [`NameEvent::Closed`].
//!
//! The job shows the answer of a fail.
//!
//! `Enter` with the unchanged name of a rename writes nothing, and sends [`NameEvent::Closed`].
//! So does `Esc`, and the focus goes home. When the field loses the focus, it sends
//! [`NameEvent::Blur`]. While a job runs, the field sends neither.

use std::rc::Rc;

use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable as _, IntoElement,
    Render, SharedString, Subscription, Window,
};

use crate::core_door::Then;
use crate::error::AppError;
use crate::text_field::{FieldEvent, TextField};
use crate::window_step::window_step;

/// Writes a typed name, and ends with the result of the job. It shows the answer of a fail.
pub(crate) type NameWrite<T> = Rc<dyn Fn(String, Then<Result<T, AppError>>, &mut Window, &mut App)>;

/// What the field tells its owner. After [`NameEvent::Written`] and [`NameEvent::Closed`], the
/// owner drops the field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NameEvent<T> {
    Written(T),
    Closed,
    Blur,
}

pub(crate) struct NameField<T> {
    field: Entity<TextField>,
    /// The name before a rename.
    start: Option<String>,
    write: NameWrite<T>,
    /// It gets the focus after `Esc` or the end of the field.
    home: FocusHandle,
    /// The job of the typed name runs.
    writing: bool,
    _subscription: Subscription,
}

/// What the field shows in its place.
pub(crate) struct Wording {
    pub(crate) id: &'static str,
    pub(crate) label: SharedString,
    /// The muted text of an empty field.
    pub(crate) placeholder: SharedString,
}

impl<T: Clone + 'static> NameField<T> {
    /// Opens an empty field, and gives it the focus.
    pub(crate) fn new_name(
        wording: Wording,
        write: NameWrite<T>,
        home: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::open(wording, None, write, home, window, cx)
    }

    /// Opens the field with the name `start`, selected, and gives it the focus.
    pub(crate) fn rename(
        wording: Wording,
        start: &str,
        write: NameWrite<T>,
        home: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::open(wording, Some(start.to_owned()), write, home, window, cx)
    }

    fn open(
        wording: Wording,
        start: Option<String>,
        write: NameWrite<T>,
        home: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let text = start.as_deref().unwrap_or_default();
        let field = cx.new(|cx| {
            TextField::new(wording.id, wording.label, text, window, cx)
                .placeholder(wording.placeholder)
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
            writing: false,
            _subscription: subscription,
        }
    }

    pub(crate) fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.field.focus_handle(cx), cx);
    }

    pub(crate) fn writing(&self) -> bool {
        self.writing
    }

    fn field_event(&mut self, event: FieldEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.writing {
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
        if self.start.as_deref() == Some(name.trim()) {
            window.focus(&self.home, cx);
            cx.emit(NameEvent::Closed);
            return;
        }
        self.set_writing(true, cx);
        let ended = window_step(window, cx, Self::ended);
        // A job may end at once, and the field can't take its end in its own update.
        let done: Then<Result<T, AppError>> =
            Box::new(move |result, cx| cx.defer(move |cx| ended(result, cx)));
        let write = self.write.clone();
        window.defer(cx, move |window, cx| write(name, done, window, cx));
    }

    fn set_writing(&mut self, writing: bool, cx: &mut Context<Self>) {
        self.writing = writing;
        self.field.update(cx, |field, _| field.set_locked(writing));
    }

    /// Gives the focus home, unless the user moved it while the job ran.
    fn go_home(&self, window: &mut Window, cx: &mut App) {
        if self.field.focus_handle(cx).is_focused(window) {
            window.focus(&self.home, cx);
        }
    }

    fn ended(&mut self, result: Result<T, AppError>, window: &mut Window, cx: &mut Context<Self>) {
        self.set_writing(false, cx);
        let error = match result {
            Ok(value) => {
                self.go_home(window, cx);
                cx.emit(NameEvent::Written(value));
                return;
            }
            Err(error) => error,
        };
        if !error.is_refused_name() {
            self.go_home(window, cx);
            cx.emit(NameEvent::Closed);
        }
    }
}

impl<T: 'static> EventEmitter<NameEvent<T>> for NameField<T> {}

impl<T: 'static> Render for NameField<T> {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.field.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::path::PathBuf;

    use gpui_kit::{TestAppContext, VisualTestContext};

    use super::*;

    type Events = Rc<RefCell<Vec<NameEvent<String>>>>;

    /// A job that refuses the first name, and then writes each name.
    fn refuse_once() -> NameWrite<String> {
        let calls = Rc::new(Cell::new(0));
        Rc::new(move |name, done, _, cx| {
            calls.set(calls.get() + 1);
            let result = if calls.get() == 1 {
                Err(AppError::Core(holm_core::Error::NameTaken {
                    path: PathBuf::from(&name),
                }))
            } else {
                Ok(name)
            };
            done(result, cx);
        })
    }

    fn field(
        cx: &mut TestAppContext,
    ) -> (Entity<NameField<String>>, Events, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::shortcuts::bind(cx);
        });
        let wording = Wording {
            id: "name",
            label: "Name".into(),
            placeholder: SharedString::default(),
        };
        let (field, cx) = cx.add_window_view(|window, cx| {
            let home = cx.focus_handle();
            NameField::rename(wording, "Old", refuse_once(), home, window, cx)
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
            assert!(!field.writing());
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
