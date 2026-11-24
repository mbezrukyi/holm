use holm_editor::shortcuts::{Shift, shortcuts};

#[test]
fn no_key_with_no_case_carries_shift() {
    // GPUI on Linux drops Shift for a key whose char has no case, so a binding with Shift never matches it.
    for shortcut in shortcuts() {
        let mut chars = shortcut.keys.key.chars();
        let no_case = matches!(
            (chars.next(), chars.next()),
            (Some(char), None) if !char.is_lowercase() && !char.is_uppercase()
        );
        assert!(!no_case || shortcut.keys.shift == Shift::Up, "{shortcut:?}");
    }
}
