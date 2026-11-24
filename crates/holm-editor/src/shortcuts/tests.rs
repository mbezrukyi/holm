use gpui_kit::{TestAppContext, TestDispatcher};

use super::*;

#[test]
fn no_key_with_no_case_carries_shift() {
    // GPUI on Linux drops Shift for a key whose char has no case, so a binding with Shift never matches it.
    for shortcut in SHORTCUTS {
        let mut chars = shortcut.keys.key.chars();
        let no_case = matches!(
            (chars.next(), chars.next()),
            (Some(char), None) if !char.is_lowercase() && !char.is_uppercase()
        );
        assert!(!no_case || shortcut.keys.shift == Shift::Up, "{shortcut:?}");
    }
}

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
