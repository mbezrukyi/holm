use std::fs;

use gpui_kit::TestAppContext;

use super::{
    Fixture, autosave, bounds_of, click, close, editor_text, empty_pane, fields, files, focus, keys, message,
    nested_folders, note_path, notes_list, open, read, set_time, settle, sidebar, state, state_menu, state_reason,
    three_notes, tree,
};
use crate::text;

#[gpui_kit::test]
fn an_outside_change_reloads_quietly_with_no_unsaved_text(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    fixture.write_outside(cx, "Note.md", "new\n");

    assert_eq!(editor_text(cx, handle), "new\n");
    assert_eq!(message(cx, handle), None);
    assert_eq!(state(cx, handle), None);
}

#[gpui_kit::test]
fn an_outside_change_with_unsaved_text_asks_to_reload_or_keep_mine(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    cx.simulate_input(handle, "mine ");
    fixture.write_outside(cx, "Note.md", "theirs\n");
    assert_eq!(state(cx, handle).as_deref(), Some(text::CHANGED_ON_DISK));
    assert_eq!(message(cx, handle), None);
    autosave(cx);
    assert_eq!(read(&note), "theirs\n", "no autosave during the question");

    keys(cx, handle, "ctrl-shift-d");
    assert_eq!(state_menu(cx, handle), ["Keep mine*", "Reload"]);
    assert_eq!(state_reason(cx, handle).as_deref(), Some(text::CHANGED_REASON));
    let text = editor_text(cx, handle);
    keys(cx, handle, "x backspace");
    assert_eq!(editor_text(cx, handle), text);
    keys(cx, handle, "escape");
    assert_eq!(state_menu(cx, handle), Vec::<String>::new());
    assert_eq!(state(cx, handle).as_deref(), Some(text::CHANGED_ON_DISK));
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "ctrl-shift-d");
    click(cx, handle, "editor");
    assert_eq!(state_menu(cx, handle), Vec::<String>::new());
    assert_eq!(state(cx, handle).as_deref(), Some(text::CHANGED_ON_DISK));
    click(cx, handle, "message-state");
    assert_eq!(state_menu(cx, handle).len(), 2);
    click(cx, handle, "message-state");
    assert_eq!(state_menu(cx, handle), Vec::<String>::new());
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "ctrl-shift-d");
    let menu = bounds_of(cx, handle, "state-menu").unwrap();
    let opener = bounds_of(cx, handle, "message-state").unwrap();
    // The panel snaps to whole pixels.
    assert!((menu.right() - opener.right()).abs() <= gpui_kit::px(0.5));
    assert!(menu.bottom() <= opener.top());
    keys(cx, handle, "escape");

    keys(cx, handle, "ctrl-shift-d enter");
    assert!(read(&note).ends_with("\n---\nmine old\n"));
    assert_eq!(state(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");

    cx.simulate_input(handle, "x");
    fixture.write_outside(cx, "Note.md", "theirs again\n");
    click(cx, handle, "message-state");
    assert_eq!(state_menu(cx, handle), ["Keep mine*", "Reload"]);
    keys(cx, handle, "down shift-tab");
    assert_eq!(
        state_menu(cx, handle),
        ["Keep mine", "Reload*"],
        "an arrow does nothing, and Shift+Tab wraps"
    );
    keys(cx, handle, "enter");
    assert_eq!(editor_text(cx, handle), "theirs again\n");
    assert_eq!(state(cx, handle), None);
}

#[gpui_kit::test]
fn a_pin_or_a_tag_change_during_a_disk_question_keeps_the_outside_text(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    cx.simulate_input(handle, "mine ");
    fixture.write_outside(cx, "Note.md", "theirs\n");

    keys(cx, handle, "ctrl-shift-p");
    assert!(read(&note).contains("pinned: true"));
    assert!(read(&note).ends_with("\n---\ntheirs\n"));
    assert_eq!(state(cx, handle).as_deref(), Some(text::CHANGED_ON_DISK));

    keys(cx, handle, "ctrl-shift-t");
    cx.simulate_input(handle, "x");
    keys(cx, handle, "enter escape");
    assert_eq!(fields(&note).0, ["x"]);
    assert!(read(&note).ends_with("\n---\ntheirs\n"));
    assert_eq!(state(cx, handle).as_deref(), Some(text::CHANGED_ON_DISK));
    assert_eq!(editor_text(cx, handle), "mine old\n");

    keys(cx, handle, "ctrl-shift-d enter");
    assert_eq!(state(cx, handle), None);
    assert!(read(&note).ends_with("\n---\nmine old\n"));
    assert!(read(&note).contains("pinned: true"));
    assert_eq!(fields(&note).0, ["x"]);
}

#[gpui_kit::test]
fn a_failed_reload_keeps_the_text_and_the_question(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;

    let fixture = Fixture::new().with_folder();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    cx.simulate_input(handle, "mine ");
    fixture.write_outside(cx, "Note.md", "theirs\n");
    fs::set_permissions(&note, fs::Permissions::from_mode(0o000)).unwrap();

    keys(cx, handle, "ctrl-shift-d tab enter");
    assert_eq!(editor_text(cx, handle), "mine old\n");
    assert_eq!(state(cx, handle).as_deref(), Some(text::CHANGED_ON_DISK));
    fs::set_permissions(&note, fs::Permissions::from_mode(0o644)).unwrap();
    // The watcher sees the new permissions, so the core knows the new stamp.
    fixture.tell(cx, "Note.md");
    autosave(cx);
    assert_eq!(read(&note), "theirs\n", "the question still holds the text");

    keys(cx, handle, "ctrl-shift-d enter");
    assert_eq!(state(cx, handle), None);
    assert!(read(&note).ends_with("\n---\nmine old\n"));
}

/// The text that a disk question holds is not saved, so the pane stays on its note.
#[gpui_kit::test]
fn a_disk_question_keeps_the_pane_on_its_note(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    three_notes(&notes_dir);
    let plan = notes_dir.join("Plan.md");
    let handle = open(cx, fixture.startup(None));
    cx.simulate_input(handle, "mine ");
    fixture.write_outside(cx, "Plan.md", "theirs\n");

    keys(cx, handle, "ctrl-shift-l tab tab");
    assert_eq!(editor_text(cx, handle), "mine # Plan\nbuy milk\n");
    assert_eq!(state(cx, handle).as_deref(), Some(text::CHANGED_ON_DISK));
    assert_eq!(read(&plan), "theirs\n");

    keys(cx, handle, "escape");
    assert!(close(cx, handle), "the first close keeps the window");
    assert_eq!(message(cx, handle).as_deref(), Some(text::CLOSE_AGAIN));
    assert_eq!(read(&plan), "theirs\n");

    assert!(!close(cx, handle));
    assert!(
        read(&plan).ends_with("\n---\nmine # Plan\nbuy milk\n"),
        "no text is lost"
    );
}

#[gpui_kit::test]
fn an_outside_folder_rename_selects_the_parent(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    nested_folders(&fixture);
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape tab space tab tab space");
    assert_eq!(sidebar(cx, handle), "tree area/work");
    let notes_dir = fixture.notes();
    fs::rename(notes_dir.join("area/work"), notes_dir.join("area/job")).unwrap();
    let feed = fixture.feed.lock().unwrap();
    feed.as_ref().unwrap().changed(["area/work", "area/job"]).unwrap();
    drop(feed);
    settle(cx);
    assert_eq!(tree(cx, handle), ["All notes", "area*", "area/bebra", "area/job"]);
}

#[gpui_kit::test]
fn an_outside_rename_of_the_open_note_keeps_it_selected(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("A.md"), "---\nid: 01A\n---\n# A\n").unwrap();
    fs::write(notes_dir.join("B.md"), "---\nid: 01B\n---\n# B\n").unwrap();
    set_time(&notes_dir.join("A.md"), 200);
    set_time(&notes_dir.join("B.md"), 100);
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l");
    assert_eq!(notes_list(cx, handle), ["A*", "B"]);

    fs::rename(notes_dir.join("A.md"), notes_dir.join("C.md")).unwrap();
    let feed = fixture.feed.lock().unwrap();
    feed.as_ref().unwrap().changed(["A.md", "C.md"]).unwrap();
    drop(feed);
    settle(cx);

    assert_eq!(notes_list(cx, handle), ["C*", "B"]);
    assert_eq!(note_path(cx, handle).as_deref(), Some("C"));
}

#[gpui_kit::test]
fn an_outside_add_on_the_empty_pane_only_lists_the_note(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));

    fixture.write_outside(cx, "Plans.md", "# Plans\n");

    assert!(empty_pane(cx, handle));
    keys(cx, handle, "enter");
    assert_eq!(notes_list(cx, handle), ["Plans"]);
    assert!(empty_pane(cx, handle));
}

#[gpui_kit::test]
fn an_outside_delete_opens_the_neighbor_or_the_empty_pane(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    three_notes(&notes_dir);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(editor_text(cx, handle), "# Plan\nbuy milk\n");

    fixture.delete_outside(cx, "Plan.md");
    assert_eq!(editor_text(cx, handle), "# Goals\nread more\n");

    fixture.delete_outside(cx, "Goals.md");
    fixture.delete_outside(cx, "Top.md");
    assert!(empty_pane(cx, handle));
    assert_eq!(files(&notes_dir), Vec::<String>::new());
    assert_eq!(state(cx, handle), None);
}

#[gpui_kit::test]
fn an_outside_delete_with_unsaved_text_asks_to_keep_or_close(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    three_notes(&notes_dir);
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-end");
    cx.simulate_input(handle, "mine");

    fixture.delete_outside(cx, "Plan.md");
    assert_eq!(state(cx, handle).as_deref(), Some(text::DELETED_ON_DISK));
    autosave(cx);
    assert!(!notes_dir.join("Plan.md").exists(), "no autosave");

    keys(cx, handle, "ctrl-shift-d");
    assert_eq!(state_menu(cx, handle), ["Keep mine*", "Close"]);
    assert_eq!(state_reason(cx, handle).as_deref(), Some(text::DELETED_REASON));
    keys(cx, handle, "escape");
    assert_eq!(state(cx, handle).as_deref(), Some(text::DELETED_ON_DISK));
    keys(cx, handle, "ctrl-shift-d enter");
    assert_eq!(state(cx, handle), None);
    assert!(read(&notes_dir.join("Plan.md")).ends_with("# Plan\nbuy milk\nmine\n"));

    cx.simulate_input(handle, "!");
    fixture.delete_outside(cx, "Plan.md");
    assert_eq!(state(cx, handle).as_deref(), Some(text::DELETED_ON_DISK));
    keys(cx, handle, "ctrl-shift-d tab enter");
    assert_eq!(editor_text(cx, handle), "# Goals\nread more\n");
    assert!(!notes_dir.join("Plan.md").exists());
    assert_eq!(state(cx, handle), None);
}

#[gpui_kit::test]
fn an_outside_delete_after_a_view_change_opens_the_first_note(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    three_notes(&fixture.notes());
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l tab tab");
    assert_eq!(editor_text(cx, handle), "# Top\n");

    // The tag work holds only Goals.
    keys(cx, handle, "escape tab");
    assert_eq!(sidebar(cx, handle), "tree #work");

    fixture.delete_outside(cx, "Top.md");
    assert_eq!(editor_text(cx, handle), "# Plan\nbuy milk\n");
}
