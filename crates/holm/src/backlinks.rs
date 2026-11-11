//! The quiet list at the end of a note: the other notes that link to it. A click opens one.

use std::path::PathBuf;

use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    ClickEvent, Context, CursorStyle, ElementId, EventEmitter, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement, Render, Role, StatefulInteractiveElement as _, Styled,
    TestSupportExt as _, Window, div, px,
};
use holm_core::model::NoteInfo;

use crate::text;

/// A click on a note in the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OpenBacklink(pub(crate) PathBuf);

#[derive(Default)]
pub(crate) struct Backlinks {
    notes: Vec<NoteInfo>,
}

impl Backlinks {
    pub(crate) fn show(&mut self, notes: Vec<NoteInfo>, cx: &mut Context<Self>) {
        if notes != self.notes {
            self.notes = notes;
            cx.notify();
        }
    }
}

impl EventEmitter<OpenBacklink> for Backlinks {}

impl Render for Backlinks {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let rows = self.notes.iter().enumerate().map(|(at, note)| {
            let path = note.path.clone();
            div()
                .id(ElementId::NamedInteger("backlink".into(), at as u64))
                .role(Role::ListItem)
                .aria_label(note.title.clone())
                .test_support()
                .cursor(CursorStyle::PointingHand)
                .hover(|row| row.text_color(theme.foreground))
                .truncate()
                .child(note.title.clone())
                // A click here is not a click in the text.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    cx.emit(OpenBacklink(path.clone()));
                }))
        });
        v_flex()
            .id("backlinks")
            .role(Role::List)
            .aria_label(text::LINKED_FROM)
            .test_support()
            .w_full()
            .when(!self.notes.is_empty(), |list| {
                list.pt(px(48.))
                    .pb(px(24.))
                    .gap_1()
                    .text_size(px(13.))
                    .text_color(theme.muted_foreground)
                    .child(
                        div()
                            .pb_1()
                            .text_size(px(11.))
                            .child(text::LINKED_FROM.to_uppercase()),
                    )
                    .children(rows)
            })
    }
}
