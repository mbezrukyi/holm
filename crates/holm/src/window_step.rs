//! The end of a job as a step of a view in its window.

use gpui_kit::{Context, Window};

use crate::core_door::Then;

/// Runs `step` only while the entity of `cx` and its window still exist.
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
