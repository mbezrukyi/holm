//! The message line: one line at the bottom of the window, under the sidebar and the note. It
//! shows only when it has a message or a state.
//!
//! - The left side shows one answer or start message. A new one takes its place. The next key
//!   clears it, and the key still does its own job.
//! - The right side shows a state, like "Not saved", after a mark. A click on it sends
//!   [`OpenState`].
//! - While a write is late, the right side shows "Writing…" in place of the state. It takes
//!   no clicks.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    Action, Bounds, Context, CursorStyle, EventEmitter, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement, Pixels, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, TestSupportExt as _, Window, div,
};

use crate::text;

/// Shows this answer in the message line. As an action, it goes up the element tree to the
/// window.
#[derive(Clone, Debug, PartialEq, Eq, Action)]
#[action(namespace = holm, no_json)]
pub(crate) struct Answer(pub(crate) &'static str);

/// The user clicked the state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OpenState;

pub(crate) struct MessageLine {
    message: Option<SharedString>,
    state: Option<SharedString>,
    writing: bool,
    /// The window bounds of the state in the last frame.
    state_bounds: Rc<Cell<Bounds<Pixels>>>,
    _pressed: Subscription,
}

impl MessageLine {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let this = cx.weak_entity();
        // It sees each key before any part of the window can stop it.
        let pressed = cx.intercept_keystrokes(move |event, _, cx| {
            if !event.keystroke.is_modifier_key() {
                this.update(cx, Self::pressed).ok();
            }
        });
        Self {
            message: None,
            state: None,
            writing: false,
            state_bounds: Rc::default(),
            _pressed: pressed,
        }
    }

    /// Shows `text` on the left until the next key.
    pub(crate) fn show(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.message = Some(text.into());
        cx.notify();
    }

    /// Shows `state` on the right, or no state with `None`.
    pub(crate) fn set_state(&mut self, state: Option<SharedString>, cx: &mut Context<Self>) {
        if self.state != state {
            self.state = state;
            cx.notify();
        }
    }

    /// Shows "Writing…" while `writing` is true.
    pub(crate) fn set_writing(&mut self, writing: bool, cx: &mut Context<Self>) {
        if self.writing != writing {
            self.writing = writing;
            cx.notify();
        }
    }

    /// The window bounds of the state in the last frame. The line updates them at each frame.
    pub(crate) fn state_bounds(&self) -> Rc<Cell<Bounds<Pixels>>> {
        self.state_bounds.clone()
    }

    fn pressed(&mut self, cx: &mut Context<Self>) {
        if self.message.take().is_some() {
            cx.notify();
        }
    }

    fn state(&self, label: SharedString, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let bounds = self.state_bounds.clone();
        div()
            .flex_none()
            .on_children_prepainted(move |children, _, _| {
                if let Some(state) = children.first() {
                    bounds.set(*state);
                }
            })
            .child(
                h_flex()
                    .id("message-state")
                    .role(Role::Button)
                    .aria_label(label.clone())
                    .test_support()
                    .gap_1()
                    .px_2()
                    .rounded(theme.radius)
                    .cursor(CursorStyle::PointingHand)
                    .hover(|state| state.bg(theme.secondary_hover))
                    .child(div().text_color(theme.primary).child(text::STATE_MARK))
                    .child(label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, window, cx| {
                            cx.emit(OpenState);
                            // The part under the line would take the focus.
                            window.prevent_default();
                            cx.stop_propagation();
                        }),
                    ),
            )
    }
}

impl EventEmitter<OpenState> for MessageLine {}

impl Render for MessageLine {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.message.is_none() && self.state.is_none() && !self.writing {
            return div().into_any_element();
        }
        let theme = cx.theme();
        let message = self.message.clone().map(|message| {
            div()
                .id("message")
                .role(Role::Status)
                .aria_label(message.clone())
                .test_support()
                .truncate()
                .child(message)
        });
        h_flex()
            .id("message-line")
            .test_support()
            .flex_none()
            .w_full()
            .px_3()
            .py_1()
            .gap_3()
            .text_sm()
            .bg(theme.status_bar)
            .border_t_1()
            .border_color(theme.status_bar_border)
            .child(div().flex_1().min_w_0().children(message))
            .children(self.writing.then(|| writing(cx)))
            .children(
                (!self.writing)
                    .then(|| self.state.clone())
                    .flatten()
                    .map(|state| self.state(state, cx)),
            )
            .into_any_element()
    }
}

/// "Writing…" with the mark of a state, and no button.
fn writing(cx: &Context<MessageLine>) -> impl IntoElement {
    let label = SharedString::from(text::WRITING);
    h_flex()
        .id("message-state")
        .role(Role::Status)
        .aria_label(label.clone())
        .test_support()
        .flex_none()
        .gap_1()
        .px_2()
        .child(div().text_color(cx.theme().primary).child(text::STATE_MARK))
        .child(label)
}
