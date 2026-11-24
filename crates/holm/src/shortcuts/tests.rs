use super::*;

/// Whether one key can reach both places.
fn overlap(one: Place, other: Place) -> bool {
    use editor::Place as E;
    let one_way = |a: Place, b: Place| {
        matches!(
            (a, b),
            (Place::Anywhere | Place::Window, _)
                | (Place::Sidebar, Place::Tree | Place::NoteList)
                | (Place::Note, Place::Editor(_))
                | (
                    Place::Editor(E::Text),
                    Place::Editor(E::Editor | E::Find) | Place::TextField
                )
        )
    };
    one == other || one_way(one, other) || one_way(other, one)
}

#[test]
fn each_row_of_the_app_binds_its_keys_in_its_place() {
    let cx = gpui_kit::TestAppContext::build(gpui_kit::TestDispatcher::new(0), None);
    cx.update(|cx| {
        for row in ROWS {
            let context = row.place.key_context();
            let bindings = row.keys.bindings(context, || Run(row.command), cx);
            let count = if row.keys.shift == Shift::Either { 2 } else { 1 };
            assert_eq!(bindings.len(), count, "{:?} {:?}", row.place, row.keys);
        }
    });
}

#[test]
fn the_shortcut_list_shows_the_scale_keys_as_ctrl_plus_and_ctrl_minus() {
    let shown = |command| {
        let keys = window_keys(command).unwrap();
        gpui_kit::component::kbd::Kbd::format(&keystroke(keys))
    };
    assert_eq!(shown(Command::ScaleUp), "Ctrl++");
    assert_eq!(shown(Command::ScaleDown), "Ctrl+-");
}

#[test]
fn each_second_key_has_the_place_of_its_command() {
    let second: Vec<_> = second_keys().collect();
    assert_eq!(second.len(), SECOND_KEYS.len());
    for (place, keys, _) in second {
        assert_eq!(place, Place::Window, "{keys:?}");
    }
}

#[test]
fn no_key_with_no_case_carries_shift() {
    // GPUI on Linux drops Shift for a key whose char has no case, so a binding with Shift never matches it.
    let keys = ROWS
        .iter()
        .map(|row| (row.place, row.keys))
        .chain(second_keys().map(|(place, keys, _)| (place, keys)));
    for (place, keys) in keys {
        let mut chars = keys.key.chars();
        let no_case = matches!(
            (chars.next(), chars.next()),
            (Some(char), None) if !char.is_lowercase() && !char.is_uppercase()
        );
        assert!(!no_case || keys.shift == Shift::Up, "{place:?} {keys:?}");
    }
}

#[test]
fn text_inputs_keep_space_and_text_fields_keep_the_plain_keys_of_other_places() {
    let typed = typed_keys();
    assert!(typed.iter().all(|keys| !keys.ctrl), "{typed:?}");
    assert!(typed.iter().any(|keys| keys.key == "space"), "{typed:?}");
    assert!(typed.iter().any(|keys| keys.key == "1"), "a color of the color window");
    assert!(!typed.iter().any(|keys| keys.key == "enter"));

    let kept = kept_by_text_fields();
    let has = |keys: Keys| kept.iter().any(|kept| kept.overlaps(&keys));
    assert!(has(Keys::plain("tab")), "the tab of the settings");
    assert!(has(Keys::plain("space")));
    assert!(has(Keys::plain("delete")), "the delete of the tree");
    assert!(has(Keys::plain("f2")), "the rename of the tree");
    assert!(!has(Keys::plain("enter")), "the text field names it");
    assert!(!has(Keys::plain("escape")), "the text field names it");
    assert!(!has(Keys::plain("f10")), "a window key passes");
    assert!(kept.iter().all(|keys| !keys.ctrl));
}

#[test]
fn no_two_shortcuts_share_a_keystroke_in_one_place() {
    let mut list = list();
    list.extend(second_keys().map(|(place, keys, _)| Shortcut {
        place,
        keys,
        group: Group::Window,
        label: "second keys",
    }));
    for (at, one) in list.iter().enumerate() {
        for other in &list[at + 1..] {
            assert!(
                !(overlap(one.place, other.place) && one.keys.overlaps(&other.keys)),
                "{one:?} and {other:?} share a keystroke",
            );
        }
    }
}
