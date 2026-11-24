use std::fs;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, TestAppContext};

use super::{
    Fixture, editor_text, expanded, focus, keys, notes_list, open, read, screen, set_time, sidebar, three_notes, tree,
    tree_field,
};
use crate::config::{Config, SavedSidebarView, SidebarItem};

#[gpui_kit::test]
fn the_keyboard_moves_through_tree_list_and_editor(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::create_dir(notes_dir.join("work")).unwrap();
    fs::write(notes_dir.join("work/Plan.md"), "# Plan\n").unwrap();
    fs::write(notes_dir.join("Top.md"), "# Top\n").unwrap();
    set_time(&notes_dir.join("work/Plan.md"), 100);
    set_time(&notes_dir.join("Top.md"), 200);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(sidebar(cx, handle), "list All notes");
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "ctrl-shift-l");
    assert_eq!(focus(cx, handle), "sidebar");

    keys(cx, handle, "escape tab enter");
    assert_eq!(sidebar(cx, handle), "list work");

    keys(cx, handle, "tab");
    assert_eq!(editor_text(cx, handle), "# Plan\n");
    keys(cx, handle, "f2");
    assert_eq!(focus(cx, handle), "title field");
    cx.simulate_input(handle, "Goals");
    keys(cx, handle, "enter");
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "ctrl-shift-l");
    assert_eq!(focus(cx, handle), "sidebar");
    keys(cx, handle, "escape");
    assert_eq!(sidebar(cx, handle), "tree work");
    // The rename keeps the time of the note, so it stays below "Top".
    keys(cx, handle, "shift-tab enter shift-tab");
    assert_eq!(editor_text(cx, handle), "# Top\n");
    assert_eq!(read(&notes_dir.join("work/Goals.md")), "# Plan\n");
    assert!(!notes_dir.join("work/Plan.md").exists());

    keys(cx, handle, "enter");
    assert_eq!(focus(cx, handle), "editor");
    keys(cx, handle, "ctrl-shift-l ctrl-shift-l");
    assert_eq!(sidebar(cx, handle), "hidden");
    assert_eq!(focus(cx, handle), "editor");
    keys(cx, handle, "ctrl-shift-l");
    assert_eq!(sidebar(cx, handle), "list All notes");
    assert_eq!(focus(cx, handle), "sidebar");

    let settings = Config::load(fixture.config()).settings().clone();
    assert_eq!(
        settings.sidebar.view,
        SavedSidebarView {
            list: true,
            item: SidebarItem::All,
        }
    );
    assert_eq!(settings.last_note, holm_core::NotePath::new("Top.md"));
}

#[gpui_kit::test]
fn tab_and_shift_tab_stay_in_the_editor(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("A.md"), "- a\n- b\n\nend").unwrap();
    let handle = open(cx, fixture.startup(None));
    assert_eq!(sidebar(cx, handle), "list All notes");
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "down tab");
    assert_eq!(editor_text(cx, handle), "- a\n  - b\n\nend");
    assert_eq!(focus(cx, handle), "editor");
    keys(cx, handle, "shift-tab");
    assert_eq!(editor_text(cx, handle), "- a\n- b\n\nend");
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "ctrl-end tab");
    assert_eq!(editor_text(cx, handle), "- a\n- b\n\nend    ");
    assert_eq!(focus(cx, handle), "editor");
    keys(cx, handle, "shift-tab");
    assert_eq!(editor_text(cx, handle), "- a\n- b\n\nend    ");
    assert_eq!(focus(cx, handle), "editor");
}

fn open_on_work(cx: &mut TestAppContext) -> AnyWindowHandle {
    let fixture = Fixture::new().with_folder();
    fs::create_dir(fixture.notes().join("work")).unwrap();
    fs::write(fixture.notes().join("Top.md"), "# Top\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l escape tab");
    assert_eq!(sidebar(cx, handle), "tree work");
    handle
}

#[gpui_kit::test]
fn space_types_a_space_in_the_rename_field_of_the_tree(cx: &mut TestAppContext) {
    let handle = open_on_work(cx);
    // Ticket 155 gives Space to the tree. This binding stands for its row.
    cx.update(|cx| {
        let bindings = holm_editor::shortcuts::Keys::plain("space").bindings(
            crate::shortcuts::Place::Tree.key_context(),
            || crate::shortcuts::Run(crate::shortcuts::Command::ShowNotes),
            cx,
        );
        cx.bind_keys(bindings);
    });
    keys(cx, handle, "f2 end space");
    assert_eq!(tree_field(cx, handle).as_deref(), Some("work "));
    assert_eq!(sidebar(cx, handle), "tree work");
}

#[gpui_kit::test]
fn space_types_a_space_in_an_input_inside_a_place_that_binds_it(cx: &mut TestAppContext) {
    let handle = open_on_work(cx);
    // A Space row of the notes view would take the space from the search field.
    cx.update(|cx| {
        let bindings = holm_editor::shortcuts::Keys::plain("space").bindings(
            crate::shortcuts::Place::Window.key_context(),
            || crate::shortcuts::Run(crate::shortcuts::Command::NewNote),
            cx,
        );
        cx.bind_keys(bindings);
    });
    keys(cx, handle, "ctrl-shift-f a space b");
    let search = screen(cx, handle, |window| {
        window
            .try_find("search")
            .and_then(|field| field.value().map(str::to_owned))
    });
    assert_eq!(search.as_deref(), Some("a b"));
}

#[gpui_kit::test]
fn tab_in_the_rename_field_leaves_the_tree_cursor(cx: &mut TestAppContext) {
    let handle = open_on_work(cx);
    keys(cx, handle, "f2 tab shift-tab");
    assert_eq!(tree_field(cx, handle).as_deref(), Some("work"));
    assert_eq!(sidebar(cx, handle), "tree work");
    keys(cx, handle, "end x");
    assert_eq!(
        tree_field(cx, handle).as_deref(),
        Some("workx"),
        "the field keeps the focus"
    );
}

#[gpui_kit::test]
fn tab_where_no_place_names_it_leaves_the_focus(cx: &mut TestAppContext) {
    let handle = open_on_work(cx);
    keys(cx, handle, "ctrl-shift-f");
    assert_eq!(focus(cx, handle), "search");
    keys(cx, handle, "tab");
    assert_eq!(focus(cx, handle), "search");
    keys(cx, handle, "shift-tab");
    assert_eq!(focus(cx, handle), "search");
    assert_eq!(sidebar(cx, handle), "tree work");
}

#[gpui_kit::test]
fn tab_moves_through_the_tree_and_wraps_and_space_opens_a_folder(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::create_dir_all(fixture.notes().join("area/work")).unwrap();
    fs::write(
        fixture.notes().join("A.md"),
        "# A
",
    )
    .unwrap();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l escape");
    assert_eq!(tree(cx, handle), ["All notes*", "area"]);

    keys(cx, handle, "shift-tab");
    assert_eq!(tree(cx, handle), ["All notes", "area*"], "Shift+Tab wraps");
    keys(cx, handle, "tab");
    assert_eq!(tree(cx, handle), ["All notes*", "area"], "Tab wraps");
    keys(cx, handle, "down right");
    assert_eq!(tree(cx, handle), ["All notes*", "area"], "an arrow does nothing");

    keys(cx, handle, "tab space");
    assert_eq!(tree(cx, handle), ["All notes", "area*", "area/work"]);
    assert_eq!(expanded(cx, handle, "area"), Some(true));
    keys(cx, handle, "tab");
    assert_eq!(tree(cx, handle), ["All notes", "area", "area/work*"]);
    keys(cx, handle, "left");
    assert_eq!(
        sidebar(cx, handle),
        "tree area/work",
        "Left no longer selects the parent"
    );
    keys(cx, handle, "shift-tab space");
    assert_eq!(tree(cx, handle), ["All notes", "area*"]);
    assert_eq!(expanded(cx, handle, "area"), Some(false));
}

#[gpui_kit::test]
fn tab_in_the_note_list_opens_each_note_and_wraps(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    three_notes(&fixture.notes());
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l");
    assert_eq!(notes_list(cx, handle), ["Plan*", "Goals", "Top"]);

    keys(cx, handle, "tab");
    assert_eq!(notes_list(cx, handle), ["Plan", "Goals*", "Top"]);
    assert_eq!(editor_text(cx, handle), "# Goals\nread more\n");
    keys(cx, handle, "tab tab");
    assert_eq!(notes_list(cx, handle), ["Plan*", "Goals", "Top"], "Tab wraps");
    keys(cx, handle, "shift-tab");
    assert_eq!(notes_list(cx, handle), ["Plan", "Goals", "Top*"], "Shift+Tab wraps");
    assert_eq!(editor_text(cx, handle), "# Top\n");
    keys(cx, handle, "up down");
    assert_eq!(
        notes_list(cx, handle),
        ["Plan", "Goals", "Top*"],
        "an arrow does nothing"
    );
    assert_eq!(focus(cx, handle), "sidebar");
}
