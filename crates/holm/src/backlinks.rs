//! The backlinks of a note: the other notes that link to it.
//!
//! A quiet count button at the right end of the tag row: "← 1 note" or "← N notes". A click
//! on it, or Ctrl+Shift+B, opens a list under it, the last changed note first.
//!
//! - `↑` and `↓` move the cursor. `Enter` or a click opens the note under it.
//! - `Esc`, or a click outside the list, closes it. See [`Dropdown`] for the focus.
//!
//! With no backlinks, the tag row leaves the button out.

use std::path::PathBuf;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, CursorStyle, ElementId, EventEmitter, FocusHandle, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, ParentElement, Pixels, Render, Role, ScrollHandle,
    StatefulInteractiveElement as _, Styled, TestSupportExt as _, Window, div, px,
};
use holm_core::model::NoteInfo;

use crate::popup::{self, Dropdown};
use crate::text;

/// The padding on each side of the count, inside its hover background.
pub(crate) const BUTTON_PAD: Pixels = px(6.);

const LIST_WIDTH: Pixels = px(260.);

/// The list scrolls when it is taller than this.
const LIST_HEIGHT: Pixels = px(280.);

/// The user picked the note at this path, to open it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OpenBacklink(pub(crate) PathBuf);

pub(crate) struct Backlinks {
    notes: Vec<NoteInfo>,
    /// Whether the list is open.
    open: bool,
    cursor: usize,
    scroll: ScrollHandle,
    dropdown: Dropdown<Self>,
}

impl Backlinks {
    /// `text` gets the focus back when the list closes with it.
    pub(crate) fn new(text: FocusHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            notes: Vec::new(),
            open: false,
            cursor: 0,
            scroll: ScrollHandle::new(),
            dropdown: Dropdown::new(text, |list, _, cx| list.close(cx), window, cx),
        }
    }

    /// Shows `notes` as the backlinks of the open note, in their order.
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
        self.open = true;
        self.cursor = 0;
        self.scroll.scroll_to_item(0);
        window.focus(self.dropdown.focus_handle(), cx);
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
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

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        match event.keystroke.key.as_str() {
            "up" => self.cursor = self.cursor.saturating_sub(1),
            "down" => self.cursor = (self.cursor + 1).min(self.notes.len().saturating_sub(1)),
            "enter" => self.pick(self.cursor, cx),
            "escape" => self.close(cx),
            _ => return,
        }
        self.scroll.scroll_to_item(self.cursor);
        cx.notify();
        cx.stop_propagation();
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
            .when(self.open, |button| button.bg(theme.secondary))
            .hover(|button| button.bg(theme.secondary))
            .child(label)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|list, _, window, cx| {
                    if list.open {
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

    /// The list under the button.
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
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |list, _, _, cx| list.pick(at, cx)),
            )
        });
        self.dropdown
            .panel(LIST_WIDTH, window, cx)
            .right_0()
            .on_key_down(cx.listener(Self::key_down))
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
            .when(self.open, |root| root.child(self.list(window, cx)))
    }
}
