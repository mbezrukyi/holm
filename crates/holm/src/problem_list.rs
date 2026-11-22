//! The problem list: the problems of Holm, right above the problem mark in the message line.
//!
//! - It opens with the cursor on the first problem.
//! - `Tab` and `Shift+Tab` move the cursor and wrap. A click moves it too.
//! - `Esc`, or a click outside the list and the mark, closes it. See [`Dropdown`] for the focus.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::{
    Anchor, Bounds, Context, ElementId, FocusHandle, InteractiveElement as _, IntoElement, MouseButton, ParentElement,
    Pixels, Render, Role, SharedString, StatefulInteractiveElement as _, TestSupportExt as _, Window, anchored,
    deferred, div, point, px,
};

use crate::list_cursor::{self, Toward};
use crate::popup::{self, Dropdown};
use crate::shortcuts::{Command, Place, Run};

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

    /// With none, the list closes.
    pub(crate) fn set_problems(&mut self, problems: Vec<SharedString>, cx: &mut Context<Self>) {
        if self.problems != problems {
            self.cursor = self.cursor.min(problems.len().saturating_sub(1));
            self.open &= !problems.is_empty();
            self.problems = problems;
            cx.notify();
        }
    }

    /// With no problems, it does nothing.
    pub(crate) fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.close(window, cx);
        } else if !self.problems.is_empty() {
            let back = window.focused(cx).unwrap_or_else(|| self.idle.clone());
            let dropdown = Dropdown::new(back, Self::close, window, cx).with_opener(self.mark.clone());
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

    fn run(&mut self, Run(command): &Run, window: &mut Window, cx: &mut Context<Self>) {
        match command {
            Command::Close => self.close(window, cx),
            Command::Next | Command::Previous => {
                let toward = if *command == Command::Next {
                    Toward::Next
                } else {
                    Toward::Previous
                };
                let len = self.problems.len();
                if let Some(at) = list_cursor::step(Some(self.cursor), len, toward, |_| true) {
                    self.cursor = at;
                }
            }
            _ => return cx.propagate(),
        }
        cx.notify();
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
            .key_context(Place::ListPopup.key_context())
            .on_action(cx.listener(Self::run))
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
