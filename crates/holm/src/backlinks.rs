//! The backlinks of a note: the other notes that link to it.
//!
//! A quiet count button at the right end of the tag row: "← 1 note" or "← N notes". A click
//! on it, or Ctrl+Shift+B, opens a list under it, the last changed note first.
//!
//! - `↑` and `↓` move the cursor. `Enter` or a click opens the note under it.
//! - `Esc`, or a click outside the list, closes it. The focus goes back to the text.
//!
//! With no backlinks, the tag row leaves the button out.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Bounds, Context, CursorStyle, ElementId, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, ParentElement,
    Pixels, Render, Role, ScrollHandle, StatefulInteractiveElement as _, Styled, Subscription,
    TestSupportExt as _, Window, div, px,
};
use holm_core::model::NoteInfo;

use crate::{popup, text};

const LIST_WIDTH: Pixels = px(260.);

/// The list scrolls when it is taller than this.
const LIST_HEIGHT: Pixels = px(280.);

/// What the backlinks tell the pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BacklinksEvent {
    /// Open the note at this path.
    Open(PathBuf),
    /// The list closed. When the focus is still in it, give the focus back to the text.
    Leave,
}

pub(crate) struct Backlinks {
    notes: Vec<NoteInfo>,
    /// Whether the list is open.
    open: bool,
    cursor: usize,
    focus: FocusHandle,
    scroll: ScrollHandle,
    /// The window bounds of the button in the last frame. A click there is not outside the list.
    button_bounds: Rc<Cell<Bounds<Pixels>>>,
    _subscriptions: Vec<Subscription>,
}

impl Backlinks {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let subscriptions = vec![cx.on_focus_out(&focus, window, |list, _, _, cx| list.close(cx))];
        Self {
            notes: Vec::new(),
            open: false,
            cursor: 0,
            focus,
            scroll: ScrollHandle::new(),
            button_bounds: Rc::default(),
            _subscriptions: subscriptions,
        }
    }

    /// Shows `notes` as the backlinks of the open note.
    pub(crate) fn show(&mut self, mut notes: Vec<NoteInfo>, cx: &mut Context<Self>) {
        notes.sort_by_key(|note| std::cmp::Reverse(note.modified));
        if notes == self.notes {
            return;
        }
        self.notes = notes;
        self.cursor = self.cursor.min(self.notes.len().saturating_sub(1));
        if self.notes.is_empty() && self.open {
            self.close(cx);
            cx.emit(BacklinksEvent::Leave);
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
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
            cx.notify();
        }
    }

    fn leave(&mut self, cx: &mut Context<Self>) {
        self.close(cx);
        cx.emit(BacklinksEvent::Leave);
    }

    fn pick(&mut self, at: usize, cx: &mut Context<Self>) {
        if let Some(note) = self.notes.get(at) {
            let path = note.path.clone();
            self.close(cx);
            cx.emit(BacklinksEvent::Open(path));
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
            "escape" => self.leave(cx),
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
            .px_2()
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
                        list.leave(cx);
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
    fn list(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let rows = self.notes.iter().enumerate().map(|(at, note)| {
            let current = self.cursor == at;
            div()
                .id(ElementId::NamedInteger("backlink".into(), at as u64))
                .role(Role::ListBoxOption)
                .aria_label(note.title.clone())
                .aria_selected(current)
                .test_support()
                .px_3()
                .py_0p5()
                .truncate()
                .cursor(CursorStyle::PointingHand)
                .when(current, |row| row.bg(theme.selection))
                .when(!current, |row| row.hover(|row| row.bg(theme.secondary)))
                .child(note.title.clone())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |list, _, _, cx| list.pick(at, cx)),
                )
        });
        popup::panel(LIST_WIDTH, cx)
            .id("backlinks")
            .role(Role::Dialog)
            .aria_label(text::LINKED_FROM)
            .test_support()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .absolute()
            .top_full()
            .right_0()
            .mt_1()
            .py_1()
            .occlude()
            .text_size(px(12.))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, window, cx| {
                    // The editor under the list would move its cursor and take the focus.
                    window.prevent_default();
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down_out(cx.listener(|list, event: &MouseDownEvent, _, cx| {
                // The button closes the list itself.
                if !list.button_bounds.get().contains(&event.position) {
                    list.leave(cx);
                }
            }))
            .child(
                div()
                    .px_3()
                    .pt_1()
                    .pb_1()
                    .text_size(px(11.))
                    .text_color(theme.muted_foreground)
                    .child(text::LINKED_FROM),
            )
            .child(
                v_flex()
                    .id("backlink-list")
                    .role(Role::ListBox)
                    .max_h(LIST_HEIGHT)
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(rows),
            )
    }
}

impl EventEmitter<BacklinksEvent> for Backlinks {}

impl Focusable for Backlinks {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Backlinks {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let button_bounds = self.button_bounds.clone();
        div()
            .on_children_prepainted(move |bounds, _, _| {
                if let Some(button) = bounds.first() {
                    button_bounds.set(*button);
                }
            })
            .relative()
            .ml_auto()
            .child(self.button(cx))
            .when(self.open, |root| root.child(self.list(cx)))
    }
}
