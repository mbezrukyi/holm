use std::fs;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};

use super::{
    Fixture, autosave, backlink_count, backlinks, bounds_of, chips, editor_text, empty_pane, fields, files, focus,
    keys, message, new_note, new_note_open, note_path, open, popup_open, press, question, read, screen, set_time,
    settle, sidebar,
};
use crate::config::Config;
use crate::text;

#[gpui_kit::test]
fn the_delete_dialog_counts_and_deletes(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::create_dir(notes_dir.join("attachments")).unwrap();
    fs::write(notes_dir.join("attachments/x.png"), "png").unwrap();
    fs::write(notes_dir.join("A.md"), "# A\n![](attachments/x.png)\n").unwrap();
    fs::write(notes_dir.join("B.md"), "# B\n").unwrap();
    set_time(&notes_dir.join("A.md"), 200);
    set_time(&notes_dir.join("B.md"), 100);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(editor_text(cx, handle), "# A\n![](attachments/x.png)\n");

    keys(cx, handle, "ctrl-shift-l delete");
    assert_eq!(
        question(cx, handle).as_deref(),
        Some("Delete “A”? This deletes 1 note and 1 file forever.")
    );
    keys(cx, handle, "escape");
    assert_eq!(question(cx, handle), None);
    assert!(notes_dir.join("A.md").is_file());
    assert_eq!(focus(cx, handle), "sidebar");

    // The dialog opens after the count, so the keys go one step at a time.
    keys(cx, handle, "delete");
    keys(cx, handle, "space");
    let checked = screen(cx, handle, |window| window.find("do-not-ask").checked());
    assert_eq!(checked, Some(true));
    keys(cx, handle, "enter");
    assert_eq!(question(cx, handle), None);
    assert!(!notes_dir.join("A.md").exists());
    assert!(!notes_dir.join("attachments/x.png").exists());
    assert_eq!(editor_text(cx, handle), "# B\n");
    assert!(!Config::load(fixture.config()).settings().ask_before_delete);

    keys(cx, handle, "delete");
    assert_eq!(question(cx, handle), None, "no dialog after Do not show again");
    assert!(!notes_dir.join("B.md").exists());
    assert!(empty_pane(cx, handle));
    assert_eq!(files(&notes_dir), ["attachments"]);
    assert_eq!(focus(cx, handle), "sidebar");
    assert_eq!(Config::load(fixture.config()).settings().last_note, None);
}

#[gpui_kit::test]
fn the_delete_dialog_names_the_note_by_its_title(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    new_note(cx, handle, "Plans");
    cx.simulate_input(handle, "# Spring plans");
    assert_eq!(files(&fixture.notes()), ["Plans.md"]);

    keys(cx, handle, "ctrl-shift-l delete");
    assert_eq!(
        question(cx, handle).as_deref(),
        Some("Delete “Plans”? This deletes 1 note forever.")
    );
}

/// While it is open.
fn title_field(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    screen(cx, handle, |window| {
        window
            .try_find("title-input")
            .and_then(|field| field.value().map(str::to_owned))
    })
}

#[gpui_kit::test]
fn f2_renames_the_note_and_esc_keeps_the_old_title(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::create_dir(notes.join("work")).unwrap();
    fs::write(notes.join("work/Plans.md"), "# Spring plans\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    assert_eq!(note_path(cx, handle).as_deref(), Some("work › Plans"));

    keys(cx, handle, "f2");
    assert_eq!(focus(cx, handle), "title field");
    assert_eq!(title_field(cx, handle).as_deref(), Some("Plans"));
    cx.simulate_input(handle, "Goals");
    keys(cx, handle, "escape");
    assert_eq!(title_field(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(files(&notes.join("work")), ["Plans.md"]);

    keys(cx, handle, "f2");
    cx.simulate_input(handle, "Goals");
    keys(cx, handle, "enter");
    assert_eq!(title_field(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(files(&notes.join("work")), ["Goals.md"]);
    assert_eq!(read(&notes.join("work/Goals.md")), "# Spring plans\n");
    assert_eq!(note_path(cx, handle).as_deref(), Some("work › Goals"));

    cx.simulate_input(handle, "x");
    autosave(cx);
    assert!(read(&notes.join("work/Goals.md")).ends_with("---\nx# Spring plans\n"));
    assert_eq!(files(&notes.join("work")), ["Goals.md"]);
}

#[gpui_kit::test]
fn f2_with_a_taken_title_shows_the_answer_and_keeps_the_field(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("Plans.md"), "").unwrap();
    fs::write(notes.join("Ideas.md"), "").unwrap();
    set_time(&notes.join("Plans.md"), 200);
    set_time(&notes.join("Ideas.md"), 100);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(note_path(cx, handle).as_deref(), Some("Plans"));

    keys(cx, handle, "f2");
    cx.simulate_input(handle, "ideas");
    keys(cx, handle, "enter");
    assert_eq!(message(cx, handle).as_deref(), Some(text::NAME_TAKEN));
    assert_eq!(title_field(cx, handle).as_deref(), Some("ideas"));
    assert_eq!(focus(cx, handle), "title field");
    assert_eq!(files(&notes), ["Ideas.md", "Plans.md"]);

    keys(cx, handle, "escape ctrl-shift-l f2");
    assert_eq!(focus(cx, handle), "title field");
    assert_eq!(title_field(cx, handle).as_deref(), Some("Plans"));
}

#[gpui_kit::test]
fn ctrl_n_names_the_note_first_and_esc_leaves_nothing(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("Old.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-n");
    assert_eq!(focus(cx, handle), "title field");
    assert_eq!(title_field(cx, handle).as_deref(), Some(""));
    assert!(new_note_open(cx, handle));
    keys(cx, handle, "escape");
    assert!(!new_note_open(cx, handle));
    assert_eq!(files(&notes), ["Old.md"]);
    assert_eq!(editor_text(cx, handle), "old\n");
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "ctrl-n");
    cx.simulate_input(handle, "old");
    keys(cx, handle, "enter");
    assert_eq!(message(cx, handle).as_deref(), Some(text::NAME_TAKEN));
    assert_eq!(title_field(cx, handle).as_deref(), Some("old"));
    keys(cx, handle, "ctrl-a backspace");
    cx.simulate_input(handle, "Plans");
    keys(cx, handle, "enter");
    assert_eq!(files(&notes), ["Old.md", "Plans.md"]);
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(note_path(cx, handle).as_deref(), Some("Plans"));
    cx.simulate_input(handle, "text");
    autosave(cx);
    assert!(read(&notes.join("Plans.md")).ends_with("---\ntext\n"));
}

#[gpui_kit::test]
fn enter_on_an_empty_title_shows_the_answer_with_no_wait(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Old.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    press(cx, handle, "ctrl-n");
    press(cx, handle, "enter");
    assert_eq!(title_field(cx, handle).as_deref(), Some(""));
    assert_eq!(focus(cx, handle), "title field");
    assert_eq!(message(cx, handle).as_deref(), Some(text::NAME_EMPTY));
}

/// Whether a frame between the tasks that `key` starts shows the empty pane.
fn empty_pane_in_a_step(cx: &mut TestAppContext, handle: AnyWindowHandle, key: &'static str) -> bool {
    press(cx, handle, key);
    let mut shown = empty_pane(cx, handle);
    while cx.executor().tick() {
        shown |= empty_pane(cx, handle);
    }
    shown
}

#[gpui_kit::test]
fn esc_on_a_new_note_keeps_the_note_from_before_with_its_undo(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("Old.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    cx.simulate_input(handle, "new ");
    assert_eq!(editor_text(cx, handle), "new old\n");

    keys(cx, handle, "ctrl-n");
    assert!(!empty_pane_in_a_step(cx, handle, "escape"));
    assert_eq!(focus(cx, handle), "editor");
    keys(cx, handle, "ctrl-z");
    assert_eq!(editor_text(cx, handle), "old\n");
}

#[gpui_kit::test]
fn enter_on_a_new_note_shows_it_with_no_empty_pane(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("Old.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-n");
    cx.simulate_input(handle, "Plans");
    assert!(!empty_pane_in_a_step(cx, handle, "enter"));
    assert_eq!(note_path(cx, handle).as_deref(), Some("Plans"));
    assert_eq!(focus(cx, handle), "editor");
    cx.simulate_input(handle, "text");
    assert_eq!(editor_text(cx, handle), "text");
}

#[gpui_kit::test]
fn ctrl_n_in_a_tag_view_gives_the_note_that_tag(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("A.md"), "---\ntags: [work]\n---\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l escape tab tab tab enter");
    assert_eq!(sidebar(cx, handle), "list #work");

    keys(cx, handle, "ctrl-n");
    assert_eq!(chips(cx, handle), ["work"]);
    cx.simulate_input(handle, "Plans");
    keys(cx, handle, "enter");
    assert_eq!(fields(&notes.join("Plans.md")).0, ["work"]);
}

#[gpui_kit::test]
fn a_click_under_the_title_field_of_a_new_note_does_nothing(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-n");
    let bottom = screen(cx, handle, |window| window.find("new-note").bounds().size.height);
    cx.update_window(handle, |_, window, cx| {
        window.click_at(
            "new-note",
            gpui_kit::point(gpui_kit::px(200.), bottom - gpui_kit::px(20.)),
            cx,
        );
    })
    .unwrap();
    settle(cx);
    assert_eq!(focus(cx, handle), "title field");
    cx.simulate_input(handle, "x");
    assert_eq!(title_field(cx, handle).as_deref(), Some("x"));
    assert_eq!(files(&fixture.notes()), Vec::<String>::new());
}

#[gpui_kit::test]
fn a_click_between_the_title_and_the_backlinks_opens_no_popup(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("Target.md"), "---\nid: 01T\n---\n").unwrap();
    fs::write(notes.join("Old.md"), "[t](holm:01T)\n").unwrap();
    set_time(&notes.join("Target.md"), 200);
    set_time(&notes.join("Old.md"), 100);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(backlink_count(cx, handle).as_deref(), Some("← 1 note"));

    let row = bounds_of(cx, handle, "tag-row").unwrap();
    let title = bounds_of(cx, handle, "note-title").unwrap();
    let dot = bounds_of(cx, handle, "path-dot").unwrap();
    let y = title.center().y - row.top();
    for x in [(title.right() + dot.left()) / 2., dot.center().x] {
        cx.update_window(handle, |_, window, cx| {
            window.click_at("tag-row", gpui_kit::point(x - row.left(), y), cx);
        })
        .unwrap();
        settle(cx);
        assert!(!popup_open(cx, handle));
    }
}

#[gpui_kit::test]
fn the_title_field_opens_in_the_place_of_the_title(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::create_dir(notes.join("work")).unwrap();
    fs::write(notes.join("work/Plans.md"), "").unwrap();
    let handle = open(cx, fixture.startup(None));

    let before = bounds_of(cx, handle, "note-path").unwrap();
    keys(cx, handle, "f2");
    assert_eq!(focus(cx, handle), "title field");
    assert_eq!(bounds_of(cx, handle, "note-path"), Some(before));
}

#[gpui_kit::test]
fn a_click_on_the_dot_or_the_count_does_not_reach_the_open_title_field(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("Target.md"), "---\nid: 01T\n---\n").unwrap();
    fs::write(notes.join("Old.md"), "[t](holm:01T)\n").unwrap();
    set_time(&notes.join("Target.md"), 200);
    set_time(&notes.join("Old.md"), 100);
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "f2");

    let row = bounds_of(cx, handle, "tag-row").unwrap();
    let click = |cx: &mut TestAppContext, id: &'static str| {
        let at = bounds_of(cx, handle, id).unwrap();
        let y = at.center().y - row.top();
        cx.update_window(handle, |_, window, cx| {
            window.click_at("tag-row", gpui_kit::point(at.center().x - row.left(), y), cx);
        })
        .unwrap();
        settle(cx);
    };
    click(cx, "path-dot");
    assert!(!popup_open(cx, handle));
    assert_eq!(focus(cx, handle), "title field");
    // The click put no caret in the field, so the title is still all selected.
    cx.simulate_input(handle, "Goal");
    assert_eq!(title_field(cx, handle).as_deref(), Some("Goal"));

    click(cx, "backlinks-button");
    assert_eq!(backlinks(cx, handle), ["Old*"]);
    assert_eq!(title_field(cx, handle), None);
    assert_eq!(files(&notes), ["Old.md", "Target.md"]);
}

#[gpui_kit::test]
fn ctrl_n_after_a_refused_title_starts_clean(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("Target.md"), "---\nid: 01T\n---\n").unwrap();
    fs::write(notes.join("Old.md"), "[t](holm:01T)\n").unwrap();
    set_time(&notes.join("Target.md"), 200);
    set_time(&notes.join("Old.md"), 100);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(backlink_count(cx, handle).as_deref(), Some("← 1 note"));

    keys(cx, handle, "f2");
    cx.simulate_input(handle, "old");
    keys(cx, handle, "enter");
    assert_eq!(title_field(cx, handle).as_deref(), Some("old"));

    keys(cx, handle, "ctrl-n");
    assert_eq!(title_field(cx, handle).as_deref(), Some(""));
    assert_eq!(backlink_count(cx, handle), None);
}
