//! The backlinks of a note: the other notes that link to it.
//!
//! A quiet count button at the right end of the tag row: "← 1 note" or "← N notes". A click on it, or Ctrl+Shift+B,
//! opens a list under it, the last changed note first.
//!
//! - `Tab` and `Shift+Tab` move the cursor and wrap. `Enter` or a click opens the note under it.
//! - `Esc`, or a click outside the list, closes it. See [`Dropdown`] for the focus.
//!
//! With no backlinks, the tag row leaves the button out.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, CursorStyle, ElementId, EventEmitter, FocusHandle, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement, Pixels, Render, Role, ScrollHandle, StatefulInteractiveElement as _, Styled, TestSupportExt as _,
    Window, div, px,
};
use holm_core::NotePath;
use holm_core::model::NoteInfo;

use crate::list_cursor::{self, Toward};
use crate::popup::{self, Dropdown};
use crate::shortcuts::{Command, Place, Run};
use crate::text;

/// The padding on each side of the count, inside its hover background.
pub(crate) const BUTTON_PAD: Pixels = px(6.);

const LIST_WIDTH: Pixels = px(260.);

/// The list scrolls when it is taller than this.
const LIST_HEIGHT: Pixels = px(280.);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OpenBacklink(pub(crate) NotePath);

pub(crate) struct Backlinks {
    notes: Vec<NoteInfo>,
    is_open: bool,
    cursor: usize,
    scroll: ScrollHandle,
    dropdown: Dropdown<Self>,
}

impl Backlinks {
    /// `text` gets the focus back when the list closes with it.
    pub(crate) fn new(text: FocusHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            notes: Vec::new(),
            is_open: false,
            cursor: 0,
            scroll: ScrollHandle::new(),
            dropdown: Dropdown::new(text, |list, _, cx| list.close(cx), window, cx),
        }
    }

    /// Keeps the order of `notes`.
    pub(crate) fn show(&mut self, notes: Vec<NoteInfo>, cx: &mut Context<Self>) {
        if notes == self.notes {
            return;
        }
        self.notes = notes;
        self.cursor = self.cursor.min(self.notes.len().saturating_sub(1));
        if self.notes.is_empty() {
            self.close(cx);
        }
        cx.notify();
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// Opens the list with the cursor on the first note. With no backlinks, it does nothing.
    pub(crate) fn enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.notes.is_empty() {
            return;
        }
        self.is_open = true;
        self.cursor = 0;
        self.scroll.scroll_to_item(0);
        window.focus(self.dropdown.focus_handle(), cx);
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if self.is_open {
            self.is_open = false;
            cx.notify();
        }
    }

    fn pick(&mut self, at: usize, cx: &mut Context<Self>) {
        if let Some(note) = self.notes.get(at) {
            let path = note.path.clone();
            self.close(cx);
            cx.emit(OpenBacklink(path));
        }
    }

    fn run(&mut self, Run(command): &Run, _window: &mut Window, cx: &mut Context<Self>) {
        match command {
            Command::Pick => self.pick(self.cursor, cx),
            Command::Close => self.close(cx),
            Command::Next | Command::Previous => {
                let toward = if *command == Command::Next {
                    Toward::Next
                } else {
                    Toward::Previous
                };
                let len = self.notes.len();
                if let Some(at) = list_cursor::step(Some(self.cursor), len, toward, |_| true) {
                    self.cursor = at;
                    self.scroll.scroll_to_item(at);
                }
            }
            _ => return cx.propagate(),
        }
        cx.notify();
    }

    fn button(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let label = text::backlink_count(self.notes.len());
        div()
            .id("backlinks-button")
            .role(Role::Button)
            .aria_label(label.clone())
            .test_support()
            .px(BUTTON_PAD)
            .rounded(theme.radius)
            .text_size(px(12.))
            .text_color(theme.muted_foreground)
            .cursor(CursorStyle::PointingHand)
            .when(self.is_open, |button| button.bg(theme.secondary))
            .hover(|button| button.bg(theme.secondary))
            .child(label)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|list, _, window, cx| {
                    if list.is_open {
                        list.close(cx);
                    } else {
                        list.enter(window, cx);
                    }
                    // The tag row would open the tag popup, and the editor take the focus.
                    window.prevent_default();
                    cx.stop_propagation();
                }),
            )
    }

    /// It shows under the button.
    fn list(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let rows = self.notes.iter().enumerate().map(|(at, note)| {
            popup::option(
                ElementId::NamedInteger("backlink".into(), at as u64),
                self.cursor == at,
                cx,
            )
            .aria_label(note.title.clone())
            .test_support()
            .truncate()
            .child(note.title.clone())
            .on_mouse_down(MouseButton::Left, cx.listener(move |list, _, _, cx| list.pick(at, cx)))
        });
        self.dropdown
            .panel(LIST_WIDTH, window, cx)
            .right_0()
            .key_context(Place::ListPopup.key_context())
            .on_action(cx.listener(Self::run))
            .id("backlinks")
            .role(Role::Dialog)
            .aria_label(text::LINKED_FROM)
            .test_support()
            .child(
                div()
                    .px_3()
                    .pt_2()
                    .text_size(px(11.))
                    .text_color(theme.muted_foreground)
                    .child(text::LINKED_FROM),
            )
            .child(
                popup::options()
                    .id("backlink-list")
                    .role(Role::ListBox)
                    .max_h(LIST_HEIGHT)
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(rows),
            )
    }
}

impl EventEmitter<OpenBacklink> for Backlinks {}

impl Render for Backlinks {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.dropdown
            .root()
            .ml_auto()
            .child(self.button(cx))
            .when(self.is_open, |root| root.child(self.list(window, cx)))
    }
}
