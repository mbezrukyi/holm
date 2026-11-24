use gpui_kit::{TestAppContext, TestDispatcher};

use super::*;

#[test]
fn each_row_binds_its_keys_in_its_place() {
    let cx = TestAppContext::build(TestDispatcher::new(0), None);
    cx.update(|cx| {
        for shortcut in SHORTCUTS {
            let command = shortcut.command;
            let bindings = shortcut.keys.bindings(
                shortcut.place.binding_context(),
                || Run {
                    command,
                    selection: Selection::Collapse,
                },
                cx,
            );
            let count = if shortcut.keys.shift == Shift::Either { 2 } else { 1 };
            assert_eq!(bindings.len(), count, "{shortcut:?}");
            assert!(
                bindings.iter().all(|binding| binding.predicate().is_some()),
                "{shortcut:?}"
            );
        }
    });
}
