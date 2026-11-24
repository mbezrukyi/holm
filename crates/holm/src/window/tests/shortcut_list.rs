use std::fs;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Role, TestAppContext};

use super::{Fixture, bounds_of, editor_text, focus, keys, labels, open, question, screen};
use crate::text;

#[gpui_kit::test]
fn the_shortcut_list_fits_a_low_window_at_a_big_scale(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fixture.write_config(&format!(
        "version = 1\nnotes_folder = {:?}\nscale = 200\n",
        fixture.notes().to_str().unwrap()
    ));
    let handle = open(cx, fixture.startup(None));
    let height = cx
        .update_window(handle, |_, window, _| window.viewport_size().height)
        .unwrap();
    assert!(height < gpui_kit::px(620.), "the window is low: {height:?}");

    keys(cx, handle, "ctrl-?");
    let rows = bounds_of(cx, handle, "shortcut-rows").unwrap();
    assert!(rows.top() > gpui_kit::px(0.), "{rows:?}");
    assert!(rows.bottom() < height, "{rows:?}");
}

/// As "label: keys".
fn shortcut_rows(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    labels(cx, handle, "shortcuts", Role::ListItem)
}

fn first_shortcut_top(cx: &mut TestAppContext, handle: AnyWindowHandle) -> gpui_kit::Pixels {
    screen(cx, handle, |window| {
        window.within("shortcuts").find_all_by_role(Role::ListItem)[0]
            .bounds()
            .top()
    })
}

#[gpui_kit::test]
fn ctrl_question_mark_opens_the_shortcut_list_and_esc_gives_the_focus_back(cx: &mut TestAppContext) {
    use crate::shortcuts;
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "a\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "ctrl-?");
    assert_eq!(question(cx, handle).as_deref(), Some(text::SHORTCUT_LIST));
    // The screen reads the two columns row by row, so the test compares the rows in any order.
    let mut rows = shortcut_rows(cx, handle);
    let mut want: Vec<_> = shortcuts::list()
        .iter()
        .map(|shortcut| {
            let keys = gpui_kit::component::kbd::Kbd::format(&shortcuts::keystroke(shortcut.keys));
            format!("{}: {keys}", shortcut.label)
        })
        .collect();
    rows.sort();
    want.sort();
    assert_eq!(rows, want, "one row for each row of both tables");
    for row in [
        "Bold: Ctrl+B",
        "Heading 6: Ctrl+6",
        "Word left: Ctrl+Left",
        "Next match: Enter",
        "Previous match: Shift+Enter",
        "New folder: Ctrl+Shift+N",
        "Shortcuts: Ctrl+?",
        "App menu: F10",
        "Close: Esc",
        "Next tag: Tab",
        "Previous tag: Shift+Tab",
        "Next row: Tab",
        "Previous row: Shift+Tab",
        "Go to the results: Enter",
    ] {
        assert!(rows.iter().any(|shown| shown == row), "{row} is missing");
    }

    keys(cx, handle, "escape");
    assert_eq!(question(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(editor_text(cx, handle), "a\n");
}

#[gpui_kit::test]
fn the_shortcut_list_names_each_place_once_and_no_label_runs_into_its_keys(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-?");

    let mut headings = labels(cx, handle, "shortcuts", Role::Heading);
    headings.sort();
    let mut want = [
        text::ANYWHERE,
        text::IN_SIDEBAR,
        text::IN_TREE,
        text::IN_NOTE_LIST,
        text::IN_TAG_POPUP,
        text::IN_SEARCH_FIELD,
        text::IN_PICKER,
        text::IN_QUESTION,
        text::IN_POPUP_LIST,
        text::IN_NAME_FIELD,
        text::IN_SETTINGS,
        text::IN_SHORTCUT_LIST,
        text::IN_ABOUT,
        text::IN_FIND,
        text::IN_COLOR_WINDOW,
        text::IN_HEX_FIELD,
    ];
    want.sort_unstable();
    assert_eq!(headings, want);

    let count = crate::shortcuts::list().len();
    let crowded: Vec<String> = screen(cx, handle, |window| {
        (0..count)
            .filter_map(|at| {
                let item = window.try_find(("shortcut", at))?;
                let (edge, name) = (item.bounds().right(), item.label().unwrap_or_default().to_owned());
                let row = window.within(("shortcut", at));
                let label = row.try_find("label")?.bounds();
                let keys = row.try_find("keys")?.bounds();
                (label.right() >= keys.left() || keys.right() > edge).then_some(name)
            })
            .collect()
    });
    assert!(crowded.is_empty(), "labels run into their keys: {crowded:?}");
}

#[gpui_kit::test]
fn tab_and_shift_tab_scroll_the_shortcut_list(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-?");
    let top = first_shortcut_top(cx, handle);

    keys(cx, handle, "shift-tab");
    assert_eq!(first_shortcut_top(cx, handle), top, "Shift+Tab stops at the top");
    keys(cx, handle, "down");
    assert_eq!(first_shortcut_top(cx, handle), top, "an arrow does nothing");
    keys(cx, handle, "tab");
    let down = first_shortcut_top(cx, handle);
    assert!(down < top, "Tab scrolls the list");
    keys(cx, handle, "tab");
    assert!(first_shortcut_top(cx, handle) < down);
    keys(cx, handle, "shift-tab shift-tab");
    assert_eq!(first_shortcut_top(cx, handle), top);
    assert_eq!(question(cx, handle).as_deref(), Some(text::SHORTCUT_LIST));
}
