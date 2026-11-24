use std::fs;
use std::time::Duration;

use gpui_kit::{AnyWindowHandle, Role, TestAppContext};

use super::{
    Fixture, autosave, editor_text, expanded, files, focus, keys, keys_now, labels, message, nested_folders, open,
    press, question, read, set_time, settle, sidebar, state, tree, tree_field, tree_steps, type_new_name,
};
use crate::config::{Config, SavedSidebarView, SidebarItem};
use crate::text;
use crate::window::Startup;

#[gpui_kit::test]
fn keys_make_rename_and_move_notes_and_folders(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("Top.md"), "# Top\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-n");
    assert_eq!(tree(cx, handle), ["All notes*", text::NEW_FOLDER]);
    assert_eq!(tree_field(cx, handle).as_deref(), Some(""));
    assert_eq!(files(&notes_dir), ["Top.md"]);
    cx.simulate_input(handle, "work");
    keys(cx, handle, "enter");
    assert!(notes_dir.join("work").is_dir());
    assert_eq!(sidebar(cx, handle), "tree work");
    assert_eq!(tree(cx, handle), ["All notes", "work*"]);

    keys(cx, handle, "enter ctrl-n");
    cx.simulate_input(handle, "Plans");
    keys(cx, handle, "enter");
    assert!(notes_dir.join("work/Plans.md").is_file());
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "ctrl-shift-l escape shift-tab enter tab");
    assert_eq!(editor_text(cx, handle), "# Top\n");
    keys(cx, handle, "ctrl-shift-m");
    keys(cx, handle, "tab enter");
    assert!(notes_dir.join("work/Top.md").is_file());
    assert!(!notes_dir.join("Top.md").exists());

    keys(cx, handle, "enter");
    cx.simulate_input(handle, "x");
    autosave(cx);
    assert!(read(&notes_dir.join("work/Top.md")).contains('x'));
    assert!(!notes_dir.join("Top.md").exists());
}

#[gpui_kit::test]
fn a_late_folder_rename_keeps_the_field_and_shows_writing(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::create_dir(notes_dir.join("work")).unwrap();
    fs::create_dir(notes_dir.join("work/sub")).unwrap();
    fs::write(notes_dir.join("Top.md"), "# Top\n").unwrap();
    let startup = Startup {
        write_budget: Duration::ZERO,
        ..fixture.startup(None)
    };
    let handle = open(cx, startup);

    keys(cx, handle, "ctrl-shift-l escape tab space");
    type_new_name(cx, handle, "job");
    press(cx, handle, "enter");
    assert_eq!(tree_field(cx, handle).as_deref(), Some("job"));
    assert_eq!(tree(cx, handle), ["All notes", "work*", "work/sub"]);
    assert_eq!(state(cx, handle).as_deref(), Some(text::WRITING));

    press(cx, handle, "escape");
    press(cx, handle, "shift-tab");
    press(cx, handle, "backspace");
    assert_eq!(tree_field(cx, handle).as_deref(), Some("job"));
    assert_eq!(tree(cx, handle), ["All notes", "work*", "work/sub"]);

    cx.run_until_parked();
    assert_eq!(tree_field(cx, handle), None);
    assert_eq!(tree(cx, handle), ["All notes", "job*", "job/sub"]);
    assert_eq!(state(cx, handle), None);
    assert!(notes_dir.join("job/sub").is_dir());
}

#[gpui_kit::test]
fn a_folder_rename_to_a_taken_name_keeps_the_field_and_shows_the_answer(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::create_dir(notes_dir.join("job")).unwrap();
    fs::create_dir(notes_dir.join("work")).unwrap();
    fs::write(notes_dir.join("Top.md"), "# Top\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape tab tab");
    assert_eq!(tree(cx, handle), ["All notes", "job", "work*"]);
    type_new_name(cx, handle, "Job");
    keys(cx, handle, "enter");
    assert_eq!(tree(cx, handle), ["All notes", "job", "work*"]);
    assert_eq!(tree_field(cx, handle).as_deref(), Some("Job"));
    assert_eq!(message(cx, handle).as_deref(), Some(text::NAME_TAKEN));
    assert_eq!(files(&notes_dir), ["Top.md", "job", "work"]);

    keys(cx, handle, "ctrl-a");
    cx.simulate_input(handle, "Work");
    keys(cx, handle, "enter");
    assert_eq!(tree_field(cx, handle), None);
    assert_eq!(tree(cx, handle), ["All notes", "Work*", "job"]);
    assert_eq!(files(&notes_dir), ["Top.md", "Work", "job"]);
}

#[gpui_kit::test]
fn a_failed_folder_rename_shows_the_old_name_and_the_error(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::create_dir(notes_dir.join("work")).unwrap();
    fs::write(notes_dir.join("Top.md"), "# Top\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape tab");
    fs::remove_dir(notes_dir.join("work")).unwrap();
    type_new_name(cx, handle, "job");
    keys_now(cx, handle, "enter");
    assert_eq!(tree_field(cx, handle), None);
    assert_eq!(tree(cx, handle), ["All notes", "work*"]);
    assert!(message(cx, handle).is_some());

    keys(cx, handle, "escape");
    assert_eq!(message(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");
}

fn rename_steps(cx: &mut TestAppContext, handle: AnyWindowHandle, name: &str) -> Vec<Vec<String>> {
    type_new_name(cx, handle, name);
    press(cx, handle, "enter");
    tree_steps(cx, handle)
}

#[gpui_kit::test]
fn a_folder_rename_keeps_the_row_selected_in_every_frame(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    nested_folders(&fixture);
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape tab space tab tab space");
    let before = ["All notes", "area", "area/bebra", "area/work*", "area/work/sub"];
    assert_eq!(tree(cx, handle), before);
    assert_eq!(
        rename_steps(cx, handle, "job"),
        [vec!["All notes", "area", "area/bebra", "area/job*", "area/job/sub"],]
    );

    assert_eq!(editor_text(cx, handle), "# A\n");
    keys(cx, handle, "ctrl-shift-l");
    cx.simulate_input(handle, "x");
    autosave(cx);
    let notes_dir = fixture.notes();
    assert!(read(&notes_dir.join("area/job/A.md")).contains('x'));
    assert!(!notes_dir.join("area/work").exists());
}

#[gpui_kit::test]
fn a_folder_rename_scrolls_to_the_new_place_of_the_row(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    for folder in 0..60 {
        fs::create_dir(fixture.notes().join(format!("f{folder:02}"))).unwrap();
    }
    fs::write(fixture.notes().join("Top.md"), "# Top\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape tab");
    assert_eq!(sidebar(cx, handle), "tree f00");
    rename_steps(cx, handle, "zz");
    assert_eq!(sidebar(cx, handle), "tree zz");
}

#[gpui_kit::test]
fn a_folder_rename_to_a_taken_name_keeps_the_row_selected_in_every_frame(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    nested_folders(&fixture);
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape tab space tab tab space");
    assert_eq!(
        rename_steps(cx, handle, "bebra"),
        [vec!["All notes", "area", "area/bebra", "area/work*", "area/work/sub"],]
    );
    assert_eq!(tree_field(cx, handle).as_deref(), Some("bebra"));
}

#[gpui_kit::test]
fn a_new_folder_with_a_taken_name_keeps_the_field_and_esc_leaves_nothing(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    nested_folders(&fixture);
    let notes_dir = fixture.notes();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape tab");
    assert_eq!(sidebar(cx, handle), "tree area");
    keys(cx, handle, "ctrl-shift-n");
    assert_eq!(
        tree(cx, handle),
        ["All notes", "area*", text::NEW_FOLDER, "area/bebra", "area/work"]
    );
    cx.simulate_input(handle, "Bebra");
    keys(cx, handle, "enter");
    assert_eq!(tree_field(cx, handle).as_deref(), Some("Bebra"));
    assert_eq!(message(cx, handle).as_deref(), Some(text::NAME_TAKEN));
    cx.simulate_input(handle, " 2");
    assert_eq!(tree_field(cx, handle).as_deref(), Some("Bebra 2"));

    keys(cx, handle, "escape");
    assert_eq!(tree_field(cx, handle), None);
    assert_eq!(tree(cx, handle), ["All notes", "area*", "area/bebra", "area/work"]);
    assert_eq!(files(&notes_dir.join("area")), ["bebra", "work"]);

    keys(cx, handle, "ctrl-shift-n enter");
    assert_eq!(tree_field(cx, handle).as_deref(), Some(""));
    assert_eq!(message(cx, handle).as_deref(), Some(text::NAME_EMPTY));
}

#[gpui_kit::test]
fn a_move_into_a_clash_shows_the_answer_and_moves_nothing(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::create_dir(notes_dir.join("work")).unwrap();
    fs::write(notes_dir.join("Top.md"), "# Top\n").unwrap();
    fs::write(notes_dir.join("work/top.md"), "# Other\n").unwrap();
    set_time(&notes_dir.join("Top.md"), 200);
    set_time(&notes_dir.join("work/top.md"), 100);
    let handle = open(cx, fixture.startup(None));

    assert_eq!(editor_text(cx, handle), "# Top\n");
    keys(cx, handle, "ctrl-shift-l");
    keys(cx, handle, "ctrl-shift-m");
    keys(cx, handle, "tab enter");
    assert_eq!(message(cx, handle).as_deref(), Some(text::NAME_TAKEN));
    assert_eq!(read(&notes_dir.join("Top.md")), "# Top\n");
    assert_eq!(read(&notes_dir.join("work/top.md")), "# Other\n");
}

#[gpui_kit::test]
fn the_move_picker_finds_a_folder_by_its_path(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::create_dir_all(notes_dir.join("work/old")).unwrap();
    fs::create_dir(notes_dir.join("home")).unwrap();
    fs::write(notes_dir.join("Top.md"), "# Top\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    let options = |cx: &mut TestAppContext| labels(cx, handle, "folder-picker", Role::ListBoxOption);

    keys(cx, handle, "ctrl-shift-l ctrl-shift-m");
    assert_eq!(question(cx, handle).as_deref(), Some(text::MOVE_TO));
    // The note is in the root folder already, so the root folder is not a choice.
    assert_eq!(options(cx), ["home*", "work", "work/old"]);

    cx.simulate_input(handle, "OLD");
    settle(cx);
    assert_eq!(options(cx), ["work/old*"]);
    keys(cx, handle, "enter");
    assert_eq!(question(cx, handle), None);
    assert!(notes_dir.join("work/old/Top.md").is_file());
    assert!(!notes_dir.join("Top.md").exists());
}

#[gpui_kit::test]
fn a_folder_move_takes_the_open_note_with_it(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    nested_folders(&fixture);
    let handle = open(cx, fixture.startup(None));

    // The picker lists the root folder first, then `area`.
    keys(cx, handle, "ctrl-shift-l escape tab space tab tab");
    assert_eq!(sidebar(cx, handle), "tree area/work");
    keys(cx, handle, "ctrl-shift-m");
    keys(cx, handle, "enter");
    let notes_dir = fixture.notes();
    assert!(notes_dir.join("work/A.md").is_file());
    assert_eq!(sidebar(cx, handle), "tree work");

    keys(cx, handle, "ctrl-shift-l");
    cx.simulate_input(handle, "x");
    autosave(cx);
    assert!(read(&notes_dir.join("work/A.md")).contains('x'));
    assert!(!notes_dir.join("area/work").exists());
}

#[gpui_kit::test]
fn a_folder_delete_selects_the_parent_folder(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::create_dir_all(notes_dir.join("work/plans")).unwrap();
    fs::write(notes_dir.join("work/Goals.md"), "# Goals\n").unwrap();
    fs::write(notes_dir.join("work/plans/Plan.md"), "# Plan\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape tab space tab");
    assert_eq!(sidebar(cx, handle), "tree work/plans");
    keys(cx, handle, "delete");
    assert!(question(cx, handle).is_some());
    keys(cx, handle, "enter");
    assert!(!notes_dir.join("work/plans").exists());
    assert_eq!(sidebar(cx, handle), "tree work");

    keys(cx, handle, "delete");
    assert!(question(cx, handle).is_some());
    keys(cx, handle, "enter");
    assert!(!notes_dir.join("work").exists());
    assert_eq!(sidebar(cx, handle), "tree All notes");
    assert_eq!(
        Config::load(fixture.config()).settings().sidebar.view,
        SavedSidebarView {
            list: false,
            item: SidebarItem::All,
        }
    );
}

/// A note and the folders `area/work/plans`, with `area/work/plans` deleted in the app.
fn delete_the_last_child_folder(cx: &mut TestAppContext, fixture: &Fixture) -> AnyWindowHandle {
    fs::create_dir_all(fixture.notes().join("area/work/plans")).unwrap();
    fs::write(fixture.notes().join("A.md"), "# A\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l escape tab space tab space");
    assert_eq!(expanded(cx, handle, "area/work"), Some(true));
    keys(cx, handle, "tab delete enter");
    assert_eq!(sidebar(cx, handle), "tree area/work");
    handle
}

#[gpui_kit::test]
fn a_folder_shows_closed_after_its_last_child_folder_is_deleted(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = delete_the_last_child_folder(cx, &fixture);

    assert_eq!(tree(cx, handle), ["All notes", "area", "area/work*"]);
    assert_eq!(expanded(cx, handle, "area/work"), None);
    keys(cx, handle, "space");
    assert_eq!(sidebar(cx, handle), "tree area/work");
    assert_eq!(expanded(cx, handle, "area/work"), None);
}

#[gpui_kit::test]
fn a_new_child_folder_after_the_delete_shows_at_once(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = delete_the_last_child_folder(cx, &fixture);

    fs::create_dir(fixture.notes().join("area/work/later")).unwrap();
    fixture.tell(cx, "area/work/later");
    assert_eq!(tree(cx, handle), ["All notes", "area", "area/work*", "area/work/later"]);
    assert_eq!(expanded(cx, handle, "area/work"), Some(true));
}

#[gpui_kit::test]
fn esc_on_a_new_folder_in_a_folder_without_children_shows_it_closed(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::create_dir(fixture.notes().join("home")).unwrap();
    fs::write(fixture.notes().join("A.md"), "# A\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape tab");
    assert_eq!(expanded(cx, handle, "home"), None);
    keys(cx, handle, "ctrl-shift-n");
    assert_eq!(tree(cx, handle), ["All notes", "home*", text::NEW_FOLDER]);
    assert_eq!(expanded(cx, handle, "home"), Some(true));

    keys(cx, handle, "escape");
    assert_eq!(tree(cx, handle), ["All notes", "home*"]);
    assert_eq!(expanded(cx, handle, "home"), None);
}
