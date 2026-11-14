//! The disk menu: the answers to a disk question, right above the state in the message line.
//!
//! - It opens with the cursor on "Keep mine".
//! - `↑` and `↓` move the cursor. `Enter` or a click picks the answer under it.
//! - `Esc`, or a click outside the menu and the state, closes it. See [`Dropdown`] for the
//!   focus.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, Bounds, Context, CursorStyle, ElementId, EventEmitter, FocusHandle,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, ParentElement, Pixels, Render,
    Role, StatefulInteractiveElement as _, Styled, TestSupportExt as _, Window, anchored, deferred,
    div, point, px,
};

use crate::popup::Dropdown;
use crate::text;

/// A question about the open note after another tool changed its file, while the editor had
/// unsaved changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Disk {
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

/// The user picked `choice` for the question `disk`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Answer {
    pub(crate) disk: Disk,
    pub(crate) choice: Choice,
}

impl Disk {
    /// The answers in the menu. The first one starts under the cursor.
    fn choices(self) -> [Choice; 2] {
        match self {
            Self::Changed => [Choice::KeepMine, Choice::Reload],
            Self::Deleted => [Choice::KeepMine, Choice::Close],
        }
    }
}

pub(crate) struct DiskMenu {
    /// The question of the open menu. `None` while the menu is closed.
    open: Option<Disk>,
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
    pub(crate) fn toggle(&mut self, disk: Disk, window: &mut Window, cx: &mut Context<Self>) {
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
        let Some(disk) = self.open else {
            return;
        };
        if let Some(choice) = disk.choices().get(at).copied() {
            self.close(cx);
            cx.emit(Answer { disk, choice });
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        match event.keystroke.key.as_str() {
            "up" => self.cursor = 0,
            "down" => self.cursor = 1,
            "enter" => self.pick(self.cursor, cx),
            "escape" => self.close(cx),
            _ => return,
        }
        cx.notify();
        cx.stop_propagation();
    }
}

impl EventEmitter<Answer> for DiskMenu {}

impl Render for DiskMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(disk) = self.open else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let rows = disk.choices().into_iter().enumerate().map(|(at, choice)| {
            let label = match choice {
                Choice::KeepMine => text::KEEP_MINE,
                Choice::Reload => text::RELOAD,
                Choice::Close => text::CLOSE,
            };
            let current = self.cursor == at;
            div()
                .id(ElementId::NamedInteger("disk-choice".into(), at as u64))
                .role(Role::ListBoxOption)
                .aria_label(label)
                .aria_selected(current)
                .test_support()
                .px_3()
                .py_0p5()
                .cursor(CursorStyle::PointingHand)
                .when(current, |row| row.bg(theme.selection))
                .when(!current, |row| row.hover(|row| row.bg(theme.secondary)))
                .child(label)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |menu, _, _, cx| menu.pick(at, cx)),
                )
        });
        let state = self.dropdown.opener();
        let panel = self
            .dropdown
            .frame(px(160.), window, cx)
            .on_key_down(cx.listener(Self::key_down))
            .id("disk-menu")
            .role(Role::ListBox)
            .test_support()
            .children(rows);
        deferred(
            anchored()
                .anchor(Anchor::BottomRight)
                .position(point(state.right(), state.top() - px(4.)))
                .child(panel),
        )
        .into_any_element()
    }
}
