//! The delete dialog. It says how many notes and files a delete removes.
//!
//! `Enter` deletes and `Esc` cancels. `Space` turns "Do not show again" on and off.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    App, ClickEvent, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement, Render, Role, StatefulInteractiveElement as _,
    Styled, TestSupportExt as _, Toggled, Window, div, px,
};
use holm_core::model::DeleteCount;

use crate::sidebar::Target;
use crate::{popup, text};

/// The answer of the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DialogEvent {
    Delete { target: Target, do_not_ask: bool },
    Cancel,
}

pub(crate) struct DeleteDialog {
    target: Target,
    count: DeleteCount,
    do_not_ask: bool,
    focus: FocusHandle,
}

impl DeleteDialog {
    pub(crate) fn new(
        target: Target,
        count: DeleteCount,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            target,
            count,
            do_not_ask: false,
            focus,
        }
    }

    /// "Delete “Plans”? This deletes 1 note and 2 files forever."
    fn question(&self) -> String {
        let path = match &self.target {
            Target::Note(path) | Target::Folder(path) => path,
        };
        let name = path
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        text::delete_question(&name, self.count.notes, self.count.attachments)
    }

    fn answer(&mut self, delete: bool, cx: &mut Context<Self>) {
        cx.emit(if delete {
            DialogEvent::Delete {
                target: self.target.clone(),
                do_not_ask: self.do_not_ask,
            }
        } else {
            DialogEvent::Cancel
        });
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        self.do_not_ask = !self.do_not_ask;
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.modified() {
            return;
        }
        match event.keystroke.key.as_str() {
            "enter" => self.answer(true, cx),
            "escape" => self.answer(false, cx),
            "space" => self.toggle(cx),
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl EventEmitter<DialogEvent> for DeleteDialog {}

impl Focusable for DeleteDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for DeleteDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let question = self.question();
        let check = if self.do_not_ask { "☑" } else { "☐" };
        let dialog = popup::panel(px(420.), cx)
            .p_4()
            .gap_3()
            .child(div().child(question.clone()))
            .child(
                div()
                    .id("do-not-ask")
                    .role(Role::CheckBox)
                    .aria_label(text::DO_NOT_ASK)
                    .aria_toggled(if self.do_not_ask {
                        Toggled::True
                    } else {
                        Toggled::False
                    })
                    .test_support()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(format!("{check} {}", text::DO_NOT_ASK))
                    .on_click(cx.listener(|dialog, _: &ClickEvent, _, cx| dialog.toggle(cx))),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("delete-cancel")
                            .label(text::CANCEL)
                            .ghost()
                            .on_click(cx.listener(|dialog, _, _, cx| dialog.answer(false, cx))),
                    )
                    .child(
                        Button::new("delete-confirm")
                            .label(text::DELETE)
                            .danger()
                            .on_click(cx.listener(|dialog, _, _, cx| dialog.answer(true, cx))),
                    ),
            );
        popup::layer(cx)
            .id("delete-dialog")
            .role(Role::Dialog)
            .aria_label(question)
            .test_support()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .child(dialog)
    }
}
