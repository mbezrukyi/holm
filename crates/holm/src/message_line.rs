//! The message line: one line at the bottom of the window, under the sidebar and the note. It
//! shows only when it has a message or a state.
//!
//! - The left side shows one answer or start message. A new one takes its place. The next key
//!   clears it, and the key still works as usual.
//! - `Ctrl+Shift+E` opens the problem list, wherever the focus is.
//! - The right side shows a state, like "Not saved", after a mark. A click on it or
//!   `Ctrl+Shift+D` opens the [`StateMenu`]. A pick in it goes back to the owner of the
//!   state as a [`StatePick`].
//! - While a job is late, the right side shows "Writing…" in place of the state. It takes
//!   no clicks.
//! - Right before the state, with no gap, a gray mark counts the problems of Holm. A click on
//!   it opens the [`ProblemList`].

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    AppContext as _, Bounds, Context, CursorStyle, Entity, EventEmitter, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement, Pixels, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, TestSupportExt as _, Window, div,
};

use crate::path_text::PathText;
use crate::problem_list::ProblemList;
use crate::state_menu::{State, StateMenu, StatePick};
use crate::text;

pub(crate) struct MessageLine {
    message: Option<PathText>,
    writing: bool,
    /// The window bounds of the state in the last frame.
    state_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// The window bounds of the problem mark in the last frame.
    mark_bounds: Rc<Cell<Bounds<Pixels>>>,
    problems: Entity<ProblemList>,
    state_menu: Entity<StateMenu>,
    _subscriptions: [Subscription; 2],
}

impl MessageLine {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let this = cx.weak_entity();
        // It sees each key before any part of the window can stop it, also with no focus.
        let pressed = cx.intercept_keystrokes(move |event, _, cx| {
            if !event.keystroke.is_modifier_key() {
                this.update(cx, MessageLine::clear).ok();
            }
        });
        let mark_bounds: Rc<Cell<Bounds<Pixels>>> = Rc::default();
        let problems = cx.new({
            let mark_bounds = mark_bounds.clone();
            |cx| ProblemList::new(mark_bounds, cx)
        });
        let state_bounds: Rc<Cell<Bounds<Pixels>>> = Rc::default();
        let state_menu = cx.new({
            let state_bounds = state_bounds.clone();
            |cx| StateMenu::new(state_bounds, cx)
        });
        let picked = cx.subscribe(&state_menu, |_, _, pick: &StatePick, cx| cx.emit(*pick));
        Self {
            message: None,
            writing: false,
            state_bounds,
            mark_bounds,
            problems,
            state_menu,
            _subscriptions: [pressed, picked],
        }
    }

    /// Shows the mark with the count of `problems`, or no mark when there are none.
    pub(crate) fn set_problems(&mut self, problems: Vec<SharedString>, cx: &mut Context<Self>) {
        self.problems
            .update(cx, |list, cx| list.set_problems(problems, cx));
        cx.notify();
    }

    /// Opens the problem list, or closes it when it is open.
    pub(crate) fn toggle_problems(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.problems.update(cx, |list, cx| list.toggle(window, cx));
    }

    /// Shows `text`, an answer or a start message, on the left until the next key.
    pub(crate) fn show(&mut self, text: impl Into<PathText>, cx: &mut Context<Self>) {
        self.message = Some(text.into());
        cx.notify();
    }

    /// Shows `state` on the right, or no state with `None`. Another state closes the state
    /// menu.
    pub(crate) fn set_state(&mut self, state: Option<State>, cx: &mut Context<Self>) {
        self.state_menu
            .update(cx, |menu, cx| menu.set_state(state, cx));
        cx.notify();
    }

    /// Opens the state menu, or closes it when it is open. With no state, it does nothing.
    pub(crate) fn toggle_state_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state_menu
            .update(cx, |menu, cx| menu.toggle(window, cx));
    }

    /// Shows "Writing…" while `writing` is true.
    pub(crate) fn set_writing(&mut self, writing: bool, cx: &mut Context<Self>) {
        if self.writing != writing {
            self.writing = writing;
            cx.notify();
        }
    }

    /// Clears the message.
    fn clear(&mut self, cx: &mut Context<Self>) {
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
                        cx.listener(|line, _, window, cx| {
                            line.toggle_state_menu(window, cx);
                            // The part under the line would take the focus.
                            window.prevent_default();
                            cx.stop_propagation();
                        }),
                    ),
            )
    }

    /// The gray mark with the count of the problems.
    fn mark(&self, count: usize, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let bounds = self.mark_bounds.clone();
        let label = SharedString::from(text::problem_mark(count));
        div()
            .flex_none()
            .on_children_prepainted(move |children, _, _| {
                if let Some(mark) = children.first() {
                    bounds.set(*mark);
                }
            })
            .child(
                div()
                    .id("problem-mark")
                    .role(Role::Button)
                    .aria_label(label.clone())
                    .test_support()
                    .px_2()
                    .rounded(theme.radius)
                    .text_color(theme.muted_foreground)
                    .cursor(CursorStyle::PointingHand)
                    .hover(|mark| mark.bg(theme.secondary_hover))
                    .child(label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|line, _, window, cx| {
                            line.toggle_problems(window, cx);
                            // The part under the line would take the focus.
                            window.prevent_default();
                            cx.stop_propagation();
                        }),
                    ),
            )
    }
}

impl EventEmitter<StatePick> for MessageLine {}

impl Render for MessageLine {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let problems = self.problems.read(cx).problems().len();
        let state = self
            .state_menu
            .read(cx)
            .state()
            .map(|state| state.label.clone());
        if self.message.is_none() && state.is_none() && !self.writing && problems == 0 {
            return div().into_any_element();
        }
        let theme = cx.theme();
        let message = self.message.as_ref().map(|message| {
            div()
                .id("message")
                .role(Role::Status)
                .aria_label(message.plain_text())
                .test_support()
                .overflow_hidden()
                .child(message.one_line())
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
            .child(
                h_flex()
                    .flex_none()
                    .children((problems > 0).then(|| self.mark(problems, cx)))
                    .children(self.writing.then(|| writing(cx)))
                    .children(
                        (!self.writing)
                            .then_some(state)
                            .flatten()
                            .map(|state| self.state(state, cx)),
                    )
                    .child(self.problems.clone())
                    .child(self.state_menu.clone()),
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
