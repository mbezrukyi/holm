use std::fs;
use std::time::Duration;

use gpui_kit::{Role, TestAppContext};

use super::{
    Fixture, autosave, editor_text, fields, focus, keys, keys_now, labels, message, new_note_open, open, press,
    question, read, read_only, set_time, settle, sidebar, state, state_menu, state_reason, three_notes, tree,
    type_unsaved,
};
use crate::text;
use crate::window::Startup;

#[gpui_kit::test]
fn typing_saves_the_note_with_its_line_breaks(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "---\nid: 01A\n---\r\none\r\ntwo\r\n").unwrap();
    let fixture = fixture.with_folder();
    let handle = open(cx, fixture.startup(None));
    assert_eq!(editor_text(cx, handle), "one\ntwo\n");

    cx.simulate_input(handle, "zero ");
    settle(cx);
    assert!(!read(&note).contains("zero"), "the autosave waits for a pause");
    autosave(cx);

    assert_eq!(read(&note), "---\nid: 01A\n---\r\nzero one\r\ntwo\r\n");
}

/// The save asks the disk question instead.
#[gpui_kit::test]
fn a_save_behind_a_late_job_keeps_an_outside_change(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "old\n").unwrap();
    let startup = Startup {
        write_budget: Duration::ZERO,
        ..fixture.startup(None)
    };
    let handle = open(cx, startup);

    press(cx, handle, "ctrl-shift-p");
    press(cx, handle, "x");
    press(cx, handle, "ctrl-shift-p");
    fs::write(&note, "theirs\n").unwrap();
    let feed = fixture.feed.lock().unwrap();
    feed.as_ref().unwrap().changed(["Note.md"]).unwrap();
    drop(feed);
    cx.run_until_parked();

    assert!(read(&note).ends_with("\n---\ntheirs\n"), "{}", read(&note));
    assert_eq!(state(cx, handle).as_deref(), Some(text::CHANGED_ON_DISK));
    assert_eq!(editor_text(cx, handle), "xold\n");

    keys(cx, handle, "ctrl-shift-d enter");
    assert_eq!(state(cx, handle), None);
    assert!(read(&note).ends_with("\n---\nxold\n"), "{}", read(&note));
}

#[gpui_kit::test]
fn a_late_pin_of_the_open_note_asks_nothing_about_the_disk(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "one\n").unwrap();
    let startup = Startup {
        write_budget: Duration::ZERO,
        ..fixture.startup(None)
    };
    let handle = open(cx, startup);

    press(cx, handle, "ctrl-shift-p");
    press(cx, handle, "x");
    cx.run_until_parked();

    assert!(read(&note).contains("pinned: true"));
    assert_eq!(editor_text(cx, handle), "xone\n");
    assert_eq!(state(cx, handle), None);
}

#[gpui_kit::test]
fn a_failed_save_shows_not_saved_until_a_save_works(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    let note = notes_dir.join("Note.md");
    fs::write(&note, "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    read_only(&notes_dir, true);
    cx.simulate_input(handle, "a");
    autosave(cx);
    assert_eq!(message(cx, handle).as_deref(), Some(text::WRITE));
    assert_eq!(state(cx, handle).as_deref(), Some(text::NOT_SAVED));

    keys(cx, handle, "right");
    cx.simulate_input(handle, "b");
    autosave(cx);
    assert_eq!(message(cx, handle), None);
    assert_eq!(state(cx, handle).as_deref(), Some(text::NOT_SAVED));
    keys(cx, handle, "ctrl-shift-d");
    assert_eq!(state_reason(cx, handle).as_deref(), Some(text::WRITE));
    assert_eq!(message(cx, handle), None);
    assert_eq!(editor_text(cx, handle), "aobld\n", "Ctrl+Shift+D types nothing");

    fixture.write_outside(cx, "Note.md", "theirs\n");
    assert_eq!(state(cx, handle).as_deref(), Some(text::CHANGED_ON_DISK));
    assert_eq!(state_reason(cx, handle), None);

    read_only(&notes_dir, false);
    keys(cx, handle, "ctrl-shift-d enter");
    assert_eq!(state(cx, handle), None);
    assert!(read(&note).ends_with("aobld\n"));
}

#[gpui_kit::test]
fn each_pane_action_sees_the_typed_text(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("A.md"), "# A\n").unwrap();
    fs::write(notes.join("B.md"), "# B\n").unwrap();
    set_time(&notes.join("A.md"), 200);
    set_time(&notes.join("B.md"), 100);
    let handle = open(cx, fixture.startup(None));
    let alpha = notes.join("Alpha.md");

    type_unsaved(cx, handle, "one ");
    keys_now(cx, handle, "f2");
    cx.simulate_input(handle, "Alpha");
    keys_now(cx, handle, "enter");
    assert!(read(&alpha).ends_with("one # A\n"), "the rename");

    type_unsaved(cx, handle, "two ");
    keys_now(cx, handle, "ctrl-n");
    assert!(new_note_open(cx, handle));
    assert!(read(&alpha).ends_with("one two # A\n"), "Ctrl+N");
    keys_now(cx, handle, "escape");

    keys_now(cx, handle, "ctrl-end");
    type_unsaved(cx, handle, "## Fresh");
    keys_now(cx, handle, "ctrl-k");
    cx.simulate_input(handle, "#fr");
    cx.run_until_parked();
    let options = labels(cx, handle, "link-picker", Role::ListBoxOption);
    assert_eq!(options, ["Fresh*"], "Ctrl+K");
    assert!(read(&alpha).ends_with("## Fresh\n"));
    keys_now(cx, handle, "escape");

    type_unsaved(cx, handle, " three");
    keys_now(cx, handle, "ctrl-k");
    cx.simulate_input(handle, "B");
    cx.run_until_parked();
    keys_now(cx, handle, "enter");
    assert!(read(&alpha).ends_with("## Fresh three\n"), "a link");

    type_unsaved(cx, handle, " four");
    keys_now(cx, handle, "ctrl-shift-o");
    cx.simulate_input(handle, "B");
    cx.run_until_parked();
    keys_now(cx, handle, "enter");
    assert_eq!(editor_text(cx, handle), "# B\n");
    assert!(read(&alpha).ends_with(" four\n"), "an open of another note");
}

#[gpui_kit::test]
fn each_sidebar_action_sees_the_typed_text(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::create_dir_all(notes.join("attachments")).unwrap();
    fs::write(notes.join("attachments/x.png"), "png").unwrap();
    fs::create_dir(notes.join("work")).unwrap();
    fs::write(notes.join("work/A.md"), "---\ntags:\n  - old\n---\n# A\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    keys_now(cx, handle, "ctrl-end");

    type_unsaved(cx, handle, "one");
    keys_now(cx, handle, "ctrl-shift-l escape tab");
    assert_eq!(tree(cx, handle), ["All notes", "work*", "#old"]);
    keys_now(cx, handle, "f2 ctrl-a");
    cx.simulate_input(handle, "area");
    keys_now(cx, handle, "enter");
    let note = notes.join("area/A.md");
    assert!(read(&note).ends_with("# A\none\n"), "a folder rename");

    keys_now(cx, handle, "escape");
    type_unsaved(cx, handle, " two");
    keys_now(cx, handle, "ctrl-shift-n");
    cx.simulate_input(handle, "new");
    keys_now(cx, handle, "enter");
    assert!(read(&note).ends_with(" two\n"), "a new folder");

    keys_now(cx, handle, "escape");
    type_unsaved(cx, handle, " three");
    keys_now(cx, handle, "ctrl-shift-l tab");
    assert_eq!(tree(cx, handle), ["All notes", "area", "area/new", "#old*"]);
    keys_now(cx, handle, "f2 ctrl-a");
    cx.simulate_input(handle, "job");
    keys_now(cx, handle, "enter");
    assert!(read(&note).ends_with(" three\n"), "the count of a tag rename");
    keys_now(cx, handle, "enter");
    assert_eq!(fields(&note).0, ["job"]);

    keys_now(cx, handle, "escape");
    type_unsaved(cx, handle, " ![](../attachments/x.png)");
    keys_now(cx, handle, "ctrl-shift-l shift-tab shift-tab enter");
    assert_eq!(sidebar(cx, handle), "list area");
    keys_now(cx, handle, "delete");
    assert_eq!(
        question(cx, handle).as_deref(),
        Some("Delete “A”? This deletes 1 note and 1 file forever."),
        "the count of a delete"
    );
    keys_now(cx, handle, "escape");

    keys_now(cx, handle, "enter");
    type_unsaved(cx, handle, " four");
    keys_now(cx, handle, "ctrl-shift-l ctrl-shift-m enter");
    let note = notes.join("A.md");
    assert!(read(&note).ends_with(" four\n"), "a note move");

    keys_now(cx, handle, "enter");
    type_unsaved(cx, handle, " five");
    keys_now(cx, handle, "ctrl-shift-l escape tab tab");
    assert_eq!(sidebar(cx, handle), "tree #job");
    keys_now(cx, handle, "delete");
    assert!(read(&note).ends_with(" five\n"), "the count of a tag delete");
    keys_now(cx, handle, "enter");
    assert_eq!(fields(&note).0, Vec::<String>::new());
}

#[gpui_kit::test]
fn ctrl_shift_d_on_not_saved_shows_its_reason(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("Note.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-d");
    assert_eq!(state_reason(cx, handle), None, "no state, no menu");

    read_only(&notes_dir, true);
    cx.simulate_input(handle, "a");
    autosave(cx);
    assert_eq!(state(cx, handle).as_deref(), Some(text::NOT_SAVED));
    keys(cx, handle, "ctrl-shift-d");
    assert_eq!(state_reason(cx, handle).as_deref(), Some(text::WRITE));
    assert_eq!(state_menu(cx, handle), Vec::<String>::new());
    assert_eq!(message(cx, handle), None);
    keys(cx, handle, "enter");
    assert_eq!(state_reason(cx, handle).as_deref(), Some(text::WRITE));
    keys(cx, handle, "escape");
    assert_eq!(state_reason(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");
    read_only(&notes_dir, false);
}

/// Also after keys in the new note, while it opens.
#[gpui_kit::test]
fn text_typed_while_another_note_opens_reaches_its_file(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    three_notes(&fixture.notes());
    let startup = Startup {
        write_budget: Duration::ZERO,
        ..fixture.startup(None)
    };
    let handle = open(cx, startup);
    keys(cx, handle, "ctrl-shift-o");
    cx.simulate_input(handle, "Top");
    settle(cx);

    press(cx, handle, "enter");
    let mut typed = 0;
    while editor_text(cx, handle).contains("# Plan") {
        press(cx, handle, "m");
        typed += 1;
        if !cx.executor().tick() {
            break;
        }
    }
    assert!(typed > 1, "the open took {typed} keys");
    assert_eq!(editor_text(cx, handle), "# Top\n");
    cx.simulate_input(handle, "xy");
    autosave(cx);

    let plan = read(&fixture.notes().join("Plan.md"));
    assert!(
        plan.ends_with(&format!("{}# Plan\nbuy milk\n", "m".repeat(typed))),
        "{typed}: {plan}"
    );
    assert!(read(&fixture.notes().join("Top.md")).ends_with("xy# Top\n"));
}
