//! The state menu: the reason and the choices of the state, right above the state in the message line. Its right edge
//! meets the right edge of the state.
//!
//! - It opens with the cursor on the first choice. A state with no choices shows only its reason.
//! - `Tab` and `Shift+Tab` move the cursor and wrap. `Enter` or a click picks the choice under it.
//! - `Esc`, or a click outside the menu and the state, closes it. See [`Dropdown`] for the focus.
//! - A new state closes the menu.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    Anchor, Bounds, Context, ElementId, EventEmitter, FocusHandle, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement, Pixels, Render, Role, SharedString, StatefulInteractiveElement as _, Styled, TestSupportExt as _,
    Window, anchored, deferred, div, point, px,
};

use crate::list_cursor::{self, Toward};
use crate::popup::{self, Cursor, Dropdown};
use crate::shortcuts::{Command, Place, Run};

const WIDTH: f32 = 280.;

/// A state of the message line, like "Changed on disk".
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct State {
    pub(crate) label: SharedString,
    /// One line on why the state is there.
    pub(crate) reason: SharedString,
    /// The first one starts under the cursor.
    pub(crate) choices: Vec<SharedString>,
}

/// The user picked the choice at this place in [`State::choices`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StatePick(pub(crate) usize);

pub(crate) struct StateMenu {
    state: Option<State>,
    open: bool,
    cursor: usize,
    /// The window bounds of the state in the last frame.
    opener: Rc<Cell<Bounds<Pixels>>>,
    /// Gets the focus when the menu closes and the focus was nowhere before.
    idle: FocusHandle,
    /// Made at each open, so the focus goes back where it was.
    dropdown: Option<Dropdown<Self>>,
}

impl StateMenu {
    /// The message line keeps the bounds of the state in `opener`.
    pub(crate) fn new(opener: Rc<Cell<Bounds<Pixels>>>, cx: &mut Context<Self>) -> Self {
        Self {
            state: None,
            open: false,
            cursor: 0,
            opener,
            idle: cx.focus_handle(),
            dropdown: None,
        }
    }

    pub(crate) fn state(&self) -> Option<&State> {
        self.state.as_ref()
    }

    /// Another state closes the menu.
    pub(crate) fn set_state(&mut self, state: Option<State>, cx: &mut Context<Self>) {
        if self.state != state {
            self.state = state;
            self.open = false;
            cx.notify();
        }
    }

    /// With no state, it does nothing.
    pub(crate) fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.close(window, cx);
        } else if self.state.is_some() {
            let back = window.focused(cx).unwrap_or_else(|| self.idle.clone());
            let dropdown = Dropdown::new(back, Self::close, window, cx).with_opener(self.opener.clone());
            window.focus(dropdown.focus_handle(), cx);
            self.dropdown = Some(dropdown);
            self.open = true;
            self.cursor = 0;
            cx.notify();
        }
    }

    fn close(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
            cx.notify();
        }
    }

    fn choices(&self) -> usize {
        self.state.as_ref().map_or(0, |state| state.choices.len())
    }

    fn pick(&mut self, at: usize, window: &mut Window, cx: &mut Context<Self>) {
        if at < self.choices() {
            self.close(window, cx);
            cx.emit(StatePick(at));
        }
    }

    fn run(&mut self, Run(command): &Run, window: &mut Window, cx: &mut Context<Self>) {
        match command {
            Command::Pick => self.pick(self.cursor, window, cx),
            Command::Close => self.close(window, cx),
            Command::Next | Command::Previous => {
                let toward = if *command == Command::Next {
                    Toward::Next
                } else {
                    Toward::Previous
                };
                let len = self.choices();
                if let Some(at) = list_cursor::step(Some(self.cursor), len, toward, |_| true) {
                    self.cursor = at;
                }
            }
            _ => return cx.propagate(),
        }
        cx.notify();
    }
}

impl EventEmitter<StatePick> for StateMenu {}

impl Render for StateMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (Some(dropdown), Some(state)) = (self.dropdown.as_ref().filter(|_| self.open), self.state.as_ref()) else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let rows = state.choices.iter().enumerate().map(|(at, choice)| {
            popup::option(
                ElementId::NamedInteger("state-choice".into(), at as u64),
                Cursor::of(at, self.cursor),
                cx,
            )
            .aria_label(choice.clone())
            .test_support()
            .child(choice.clone())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |menu, _, window, cx| menu.pick(at, window, cx)),
            )
        });
        let reason = div()
            .id("state-reason")
            .role(Role::Status)
            .aria_label(state.reason.clone())
            .test_support()
            .px_3()
            .py_1p5()
            .text_color(theme.muted_foreground)
            .child(state.reason.clone());
        let choices = (!state.choices.is_empty()).then(|| {
            div()
                .border_t_1()
                .border_color(theme.border)
                .child(popup::options().children(rows))
        });
        let opener = dropdown.opener();
        let panel = dropdown
            .frame(px(WIDTH), window, cx)
            .key_context(Place::ListPopup.key_context())
            .on_action(cx.listener(Self::run))
            .id("state-menu")
            .role(Role::ListBox)
            .test_support()
            .child(reason)
            .children(choices);
        deferred(
            anchored()
                .anchor(Anchor::BottomRight)
                .position(point(opener.right(), opener.top() - px(4.)))
                .child(panel),
        )
        .into_any_element()
    }
}
