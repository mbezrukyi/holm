//! The opens of the pane. Each open of a note starts a new open, and each drop or stop of
//! an open cancels the steps of the opens before. A step made from an [`Open`] runs only
//! while no newer open started, so a late read never shows over a newer note.

use gpui_kit::{Context, Window};

use super::{NotePane, step};
use crate::core_door::{Then, window_step};

/// The opens of the pane.
#[derive(Default)]
pub(super) struct Opens {
    last: u64,
}

/// One open of the pane. Its steps do nothing once a newer open starts.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Open(u64);

impl Opens {
    /// Starts a new open. The steps of all older opens do nothing from now on.
    pub(super) fn start(&mut self) -> Open {
        self.last += 1;
        Open(self.last)
    }

    /// Cancels the steps of all opens so far.
    pub(super) fn cancel(&mut self) {
        self.last += 1;
    }

    /// The open of the note in the pane now. A reload belongs to it.
    pub(super) fn current(&self) -> Open {
        Open(self.last)
    }
}

impl Open {
    /// A `done` step that runs `step` on the pane while this open is the current one.
    pub(super) fn step<R: 'static>(
        self,
        cx: &Context<NotePane>,
        step_fn: impl FnOnce(&mut NotePane, R, &mut Context<NotePane>) + 'static,
    ) -> Then<R> {
        step(cx, move |pane, result, cx| {
            if pane.opens.current() == self {
                step_fn(pane, result, cx);
            }
        })
    }

    /// As [`Open::step`], with the window of the pane.
    pub(super) fn window_step<R: 'static>(
        self,
        window: &Window,
        cx: &Context<NotePane>,
        step_fn: impl FnOnce(&mut NotePane, R, &mut Window, &mut Context<NotePane>) + 'static,
    ) -> Then<R> {
        window_step(
            window,
            cx,
            move |pane: &mut NotePane, result, window, cx| {
                if pane.opens.current() == self {
                    step_fn(pane, result, window, cx);
                }
            },
        )
    }
}
