//! The dialog before a change that has no undo. It says how many notes the change touches.
//!
//! `Enter` confirms and `Esc` cancels. Before a delete of notes, `Space` turns the
//! "Do not show again" switch on and off. The tag dialogs always show, because their count of
//! notes is the point of the dialog.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex};
use gpui_kit::{
    App, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement, Render, Role, StatefulInteractiveElement as _, Styled,
    TestSupportExt as _, Window, div, px,
};
use holm_core::model::DeleteCount;

use crate::sidebar::Target;
use crate::{popup, text};

/// The change that the dialog asks about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ask {
    /// Delete a note or a folder.
    Delete { target: Target, count: DeleteCount },
    /// Rename a tag in all notes.
    RenameTag {
        from: String,
        to: String,
        notes: usize,
    },
    /// Remove a tag from all notes.
    DeleteTag { tag: String, notes: usize },
}

/// The answer of the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DialogEvent {
    Confirm { ask: Ask, do_not_ask: bool },
    Cancel,
}

pub(crate) struct Dialog {
    ask: Ask,
    do_not_ask: bool,
    focus: FocusHandle,
}

impl Dialog {
    pub(crate) fn new(ask: Ask, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            ask,
            do_not_ask: false,
            focus,
        }
    }

    /// For example: "Delete “Plans”? This deletes 1 note and 2 files forever."
    fn question(&self) -> String {
        match &self.ask {
            Ask::Delete { target, count } => {
                let path = match target {
                    Target::Note(path) | Target::Folder(path) => path,
                };
                let name = path
                    .file_stem()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                text::delete_question(&name, count.notes, count.attachments)
            }
            Ask::RenameTag { from, to, notes } => text::rename_tag_question(from, to, *notes),
            Ask::DeleteTag { tag, notes } => text::delete_tag_question(tag, *notes),
        }
    }

    fn answer(&mut self, confirm: bool, cx: &mut Context<Self>) {
        cx.emit(if confirm {
            DialogEvent::Confirm {
                ask: self.ask.clone(),
                do_not_ask: self.do_not_ask,
            }
        } else {
            DialogEvent::Cancel
        });
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.ask.can_skip() {
            self.do_not_ask = !self.do_not_ask;
            cx.notify();
        }
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

impl Ask {
    /// Whether the dialog has "Do not show again".
    pub(crate) fn can_skip(&self) -> bool {
        matches!(self, Self::Delete { .. })
    }
}

impl EventEmitter<DialogEvent> for Dialog {}

impl Focusable for Dialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Dialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let question = self.question();
        let confirm = match self.ask {
            Ask::RenameTag { .. } => text::RENAME,
            Ask::Delete { .. } | Ask::DeleteTag { .. } => text::DELETE,
        };
        let switch = self.ask.can_skip().then(|| {
            h_flex()
                .gap_2()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(
                    Switch::new("do-not-ask")
                        .checked(self.do_not_ask)
                        .label(text::DO_NOT_ASK)
                        .small()
                        .tab_stop(false)
                        .on_click(cx.listener(|dialog, _: &bool, _, cx| dialog.toggle(cx))),
                )
        });
        let dialog = popup::panel(px(420.), cx)
            .p_4()
            .gap_3()
            .child(div().child(question.clone()))
            .children(switch)
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel")
                            .label(text::CANCEL)
                            .ghost()
                            .on_click(cx.listener(|dialog, _, _, cx| dialog.answer(false, cx))),
                    )
                    .child(
                        Button::new("confirm")
                            .label(confirm)
                            .danger()
                            .on_click(cx.listener(|dialog, _, _, cx| dialog.answer(true, cx))),
                    ),
            );
        popup::layer(cx)
            .id("dialog")
            .role(Role::Dialog)
            .aria_label(question)
            .test_support()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .child(dialog)
    }
}
