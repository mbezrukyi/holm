use std::fs;

use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;

use super::{
    Fixture, chips, editor_text, empty_pane, files, focus, keys, new_note_open, open, screen, sidebar, three_notes,
};
use crate::config::Config;

#[gpui_kit::test]
fn an_empty_folder_shows_the_empty_pane_and_makes_no_note(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));

    assert!(empty_pane(cx, handle));
    assert_eq!(files(&fixture.notes()), Vec::<String>::new());
    assert_eq!(focus(cx, handle), "sidebar");
    assert_eq!(chips(cx, handle), Vec::<String>::new());

    keys(cx, handle, "ctrl-shift-p ctrl-shift-t ctrl-shift-b ctrl-k");
    assert_eq!(files(&fixture.notes()), Vec::<String>::new());
    assert!(empty_pane(cx, handle));

    keys(cx, handle, "ctrl-shift-l");
    assert_eq!(sidebar(cx, handle), "hidden");
    keys(cx, handle, "ctrl-shift-l");
    assert_eq!(focus(cx, handle), "sidebar");

    keys(cx, handle, "ctrl-n");
    assert!(!empty_pane(cx, handle));
    assert!(new_note_open(cx, handle));
    assert_eq!(focus(cx, handle), "title field");
    assert_eq!(files(&fixture.notes()), Vec::<String>::new());
    cx.simulate_input(handle, "Plans");
    keys(cx, handle, "enter");
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(files(&fixture.notes()), ["Plans.md"]);
    let settings = Config::load(fixture.config()).settings().clone();
    assert_eq!(settings.last_note, holm_core::NotePath::new("Plans.md"));
}

#[gpui_kit::test]
fn the_link_keys_do_nothing_on_the_empty_pane_with_no_sidebar(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l");
    assert_eq!(sidebar(cx, handle), "hidden");
    assert!(empty_pane(cx, handle));

    keys(cx, handle, "ctrl-k ctrl-o ctrl-j");
    assert!(!cx.did_prompt_for_paths());
    let picker = screen(cx, handle, |window| window.try_find("link-picker").is_some());
    assert!(!picker);
    assert!(empty_pane(cx, handle));
}

#[gpui_kit::test]
fn the_start_with_no_last_note_opens_the_first_note_of_the_note_order(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let config = fs::read_to_string(fixture.config()).unwrap();
    fixture.write_config(&format!("{config}note_order = \"title\"\n"));
    three_notes(&fixture.notes());
    let handle = open(cx, fixture.startup(None));
    assert_eq!(editor_text(cx, handle), "# Goals\nread more\n");
}

#[gpui_kit::test]
fn a_delete_of_the_open_note_opens_its_neighbor(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    three_notes(&fixture.notes());
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l tab");
    assert_eq!(editor_text(cx, handle), "# Goals\nread more\n");

    keys(cx, handle, "delete");
    keys(cx, handle, "enter");
    assert_eq!(editor_text(cx, handle), "# Top\n");
    assert_eq!(files(&fixture.notes()), ["Plan.md", "Top.md"]);
}
