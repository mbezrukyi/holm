//! The end of a job as a step of a view in its window.

use gpui_kit::{Context, Window};

use crate::core_door::Then;

/// A step that runs `step` on the entity of `cx` with its window, while both still exist.
pub(crate) fn window_step<V: 'static, R: 'static>(
    window: &Window,
    cx: &Context<V>,
    step: impl FnOnce(&mut V, R, &mut Window, &mut Context<V>) + 'static,
) -> Then<R> {
    let entity = cx.weak_entity();
    let window = window.window_handle();
    Box::new(move |result, cx| {
        window
            .update(cx, |_, window, cx| {
                entity.update(cx, |entity, cx| step(entity, result, window, cx))
            })
            .ok();
    })
}
