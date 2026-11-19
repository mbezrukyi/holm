//! The disk menu: the choices for a disk question, right above the state in the message line
//! and as wide as it.
//!
//! - It opens with the cursor on "Keep mine".
//! - `↑` and `↓` move the cursor. `Enter` or a click picks the choice under it.
//! - `Esc`, or a click outside the menu and the state, closes it. See [`Dropdown`] for the
//!   focus.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::{
    Anchor, Bounds, Context, ElementId, EventEmitter, FocusHandle, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, ParentElement, Pixels, Render, Role,
    StatefulInteractiveElement as _, TestSupportExt as _, Window, anchored, deferred, div, point,
    px,
};

use crate::popup::{self, Dropdown};
use crate::shortcuts::{self, Command, Place};
use crate::text;

/// A question about the open note after another tool changed its file, while the editor had
/// unsaved changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiskQuestion {
    Changed,
    Deleted,
}

/// A row of the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    KeepMine,
    Reload,
    Close,
}

/// The user picked `choice` for `question`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Chosen {
    pub(crate) question: DiskQuestion,
    pub(crate) choice: Choice,
}

impl DiskQuestion {
    /// The choices in the menu. The first one starts under the cursor.
    fn choices(self) -> [Choice; 2] {
        match self {
            Self::Changed => [Choice::KeepMine, Choice::Reload],
            Self::Deleted => [Choice::KeepMine, Choice::Close],
        }
    }
}

pub(crate) struct DiskMenu {
    /// The question of the open menu. `None` while the menu is closed.
    open: Option<DiskQuestion>,
    cursor: usize,
    dropdown: Dropdown<Self>,
}

impl DiskMenu {
    /// `text` gets the focus back when the menu closes with it. The message line keeps the
    /// bounds of its state in `state`.
    pub(crate) fn new(
        text: FocusHandle,
        state: Rc<Cell<Bounds<Pixels>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            open: None,
            cursor: 0,
            dropdown: Dropdown::new(text, |menu, _, cx| menu.close(cx), window, cx)
                .with_opener(state),
        }
    }

    /// Opens the menu for `disk`, or closes it when it is open.
    pub(crate) fn toggle(
        &mut self,
        disk: DiskQuestion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open.is_some() {
            self.close(cx);
        } else {
            self.open = Some(disk);
            self.cursor = 0;
            window.focus(self.dropdown.focus_handle(), cx);
            cx.notify();
        }
    }

    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        if self.open.take().is_some() {
            cx.notify();
        }
    }

    fn pick(&mut self, at: usize, cx: &mut Context<Self>) {
        let Some(question) = self.open else {
            return;
        };
        if let Some(choice) = question.choices().get(at).copied() {
            self.close(cx);
            cx.emit(Chosen { question, choice });
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        match shortcuts::command(Place::ListPopup, keystroke) {
            Some(Command::Pick) => self.pick(self.cursor, cx),
            Some(Command::Close) => self.close(cx),
            _ if keystroke.modifiers.modified() => return,
            _ => match keystroke.key.as_str() {
                "up" => self.cursor = 0,
                "down" => self.cursor = 1,
                _ => return,
            },
        }
        cx.notify();
        cx.stop_propagation();
    }
}

impl EventEmitter<Chosen> for DiskMenu {}

impl Render for DiskMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(disk) = self.open else {
            return div().into_any_element();
        };
        let rows = disk.choices().into_iter().enumerate().map(|(at, choice)| {
            let label = match choice {
                Choice::KeepMine => text::KEEP_MINE,
                Choice::Reload => text::RELOAD,
                Choice::Close => text::CLOSE,
            };
            popup::option(
                ElementId::NamedInteger("disk-choice".into(), at as u64),
                self.cursor == at,
                cx,
            )
            .aria_label(label)
            .test_support()
            .child(label)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |menu, _, _, cx| menu.pick(at, cx)),
            )
        });
        let state = self.dropdown.opener();
        let panel = self
            .dropdown
            .frame(state.size.width, window, cx)
            .on_key_down(cx.listener(Self::key_down))
            .id("disk-menu")
            .role(Role::ListBox)
            .test_support()
            .child(popup::options().children(rows));
        deferred(
            anchored()
                .anchor(Anchor::BottomLeft)
                .position(point(state.left(), state.top() - px(4.)))
                .child(panel),
        )
        .into_any_element()
    }
}
