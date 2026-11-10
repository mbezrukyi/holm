//! The outside changes of the core, for an entity on the UI thread.

use std::time::Duration;

use gpui_kit::{Context, Task};
use holm_core::Core;
use holm_core::model::Change;

/// How often the UI looks for new changes. The core already waits up to 500 ms to batch them.
pub(crate) const POLL: Duration = Duration::from_millis(100);

/// Calls `on_change` for each change that comes from outside Holm, until the entity is gone.
pub(crate) fn watch<T: 'static>(
    core: &Core,
    cx: &mut Context<T>,
    on_change: impl Fn(&mut T, Change, &mut Context<T>) + 'static,
) -> Task<()> {
    let changes = core.changes();
    cx.spawn(async move |entity, cx| {
        loop {
            cx.background_executor().timer(POLL).await;
            let gone = entity.update(cx, |entity, cx| {
                while let Some(change) = changes.recv_timeout(Duration::ZERO) {
                    on_change(entity, change, cx);
                }
            });
            if gone.is_err() {
                break;
            }
        }
    })
}
