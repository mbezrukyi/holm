//! The problem list: the problems of Holm, right above the problem mark in the message line.
//!
//! - It opens with the cursor on the first problem.
//! - `↑` and `↓` move the cursor. A click moves it too.
//! - `Esc`, or a click outside the list and the mark, closes it. See [`Dropdown`] for the focus.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::{
    Anchor, Bounds, Context, ElementId, FocusHandle, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, Pixels, Render, Role, SharedString,
    StatefulInteractiveElement as _, TestSupportExt as _, Window, anchored, deferred, div, point,
    px,
};

use crate::popup::{self, Dropdown};

const WIDTH: f32 = 240.;

pub(crate) struct ProblemList {
    problems: Vec<SharedString>,
    open: bool,
    cursor: usize,
    /// The window bounds of the mark in the last frame.
    mark: Rc<Cell<Bounds<Pixels>>>,
    /// Gets the focus when the list closes and the focus was nowhere before.
    idle: FocusHandle,
    /// Made at each open, so the focus goes back where it was.
    dropdown: Option<Dropdown<Self>>,
}

impl ProblemList {
    /// The message line keeps the bounds of the mark in `mark`.
    pub(crate) fn new(mark: Rc<Cell<Bounds<Pixels>>>, cx: &mut Context<Self>) -> Self {
        Self {
            problems: Vec::new(),
            open: false,
            cursor: 0,
            mark,
            idle: cx.focus_handle(),
            dropdown: None,
        }
    }

    pub(crate) fn problems(&self) -> &[SharedString] {
        &self.problems
    }

    /// Shows `problems`. With none, the list closes.
    pub(crate) fn set_problems(&mut self, problems: Vec<SharedString>, cx: &mut Context<Self>) {
        if self.problems != problems {
            self.cursor = self.cursor.min(problems.len().saturating_sub(1));
            self.open &= !problems.is_empty();
            self.problems = problems;
            cx.notify();
        }
    }

    /// Opens the list, or closes it when it is open. With no problems, it does nothing.
    pub(crate) fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.close(window, cx);
        } else if !self.problems.is_empty() {
            let back = window.focused(cx).unwrap_or_else(|| self.idle.clone());
            let dropdown =
                Dropdown::new(back, Self::close, window, cx).with_opener(self.mark.clone());
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

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        let last = self.problems.len().saturating_sub(1);
        match event.keystroke.key.as_str() {
            "up" => self.cursor = self.cursor.saturating_sub(1),
            "down" => self.cursor = (self.cursor + 1).min(last),
            "escape" => self.close(window, cx),
            _ => return,
        }
        cx.notify();
        cx.stop_propagation();
    }
}

impl Render for ProblemList {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(dropdown) = self.dropdown.as_ref().filter(|_| self.open) else {
            return div().into_any_element();
        };
        let rows = self.problems.iter().enumerate().map(|(at, problem)| {
            popup::option(
                ElementId::NamedInteger("problem".into(), at as u64),
                self.cursor == at,
                cx,
            )
            .aria_label(problem.clone())
            .test_support()
            .child(problem.clone())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |list, _, _, cx| {
                    list.cursor = at;
                    cx.notify();
                }),
            )
        });
        let mark = dropdown.opener();
        let panel = dropdown
            .frame(px(WIDTH), window, cx)
            .on_key_down(cx.listener(Self::key_down))
            .id("problem-list")
            .role(Role::ListBox)
            .test_support()
            .child(popup::options().children(rows));
        deferred(
            anchored()
                .anchor(Anchor::BottomRight)
                .position(point(mark.right(), mark.top() - px(4.)))
                .child(panel),
        )
        .into_any_element()
    }
}
