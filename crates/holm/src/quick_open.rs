//! Quick open: a small window that finds a note by its title.
//!
//! It matches every title that holds the typed text. `↑` and `↓` pick a note, `Enter` opens
//! it, and `Esc` closes the window.

use std::path::PathBuf;

use gpui_kit::base::actions::Cancel;
use gpui_kit::component::IndexPath;
use gpui_kit::component::command::{Command, CommandItem, CommandState};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement, Render, Role, StatefulInteractiveElement as _, TestSupportExt as _, Window, px,
};
use holm_core::model::NoteInfo;

use crate::{popup, text};

/// The answer of the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum QuickOpenEvent {
    Open(PathBuf),
    Close,
}

pub(crate) struct QuickOpen {
    notes: Vec<NoteInfo>,
    state: Entity<CommandState>,
}

impl QuickOpen {
    /// Quick open over `notes`, in their order. The field gets the focus.
    pub(crate) fn new(notes: Vec<NoteInfo>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|cx| CommandState::new(window, cx));
        state.update(cx, |state, cx| state.focus(window, cx));
        Self { notes, state }
    }

    fn open(&mut self, at: IndexPath, cx: &mut Context<Self>) {
        if let Some(note) = self.notes.get(at.row) {
            cx.emit(QuickOpenEvent::Open(note.path.clone()));
        }
    }
}

impl EventEmitter<QuickOpenEvent> for QuickOpen {}

impl Render for QuickOpen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let items = self
            .notes
            .iter()
            .map(|note| CommandItem::new().label(note.title.clone()));
        let command = Command::new(&self.state)
            .items(items.collect::<Vec<_>>())
            .placeholder(text::QUICK_OPEN)
            .bordered(false)
            .empty(|_, _, cx| popup::empty(text::NO_MATCH, cx))
            .on_confirm(move |at, _, cx| {
                this.update(cx, |this, cx| this.open(at, cx)).ok();
            });
        popup::layer(cx)
            .id("quick-open")
            .role(Role::Dialog)
            .aria_label(text::QUICK_OPEN)
            .test_support()
            // The palette clears its text on the first `Esc`. The window closes at once.
            .capture_action(cx.listener(|_, _: &Cancel, _, cx| {
                cx.emit(QuickOpenEvent::Close);
                cx.stop_propagation();
            }))
            .child(popup::panel(px(480.), cx).child(command))
    }
}
