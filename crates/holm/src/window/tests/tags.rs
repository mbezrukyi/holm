use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Role, TestAppContext};

use super::{
    Fixture, autosave, bounds_of, chips, click, click_chip, editor_text, fields, focus, keys, keys_now, modified,
    name_field, open, options, popup_open, press, question, screen, set_time, settle, sidebar, tree, tree_field,
    tree_steps, type_new_name,
};
use crate::config::Config;

#[gpui_kit::test]
fn the_tag_popup_adds_and_removes_tags_and_pins(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    let a = notes_dir.join("A.md");
    fs::write(&a, "---\ntags:\n  - work\n  - home\n---\n# A\n").unwrap();
    fs::write(
        notes_dir.join("B.md"),
        "---\ntags:\n  - project/holm\n  - holiday\n---\n# B\n",
    )
    .unwrap();
    set_time(&a, 200);
    set_time(&notes_dir.join("B.md"), 100);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(editor_text(cx, handle), "# A\n");
    assert_eq!(chips(cx, handle), ["work", "home"]);
    assert!(!popup_open(cx, handle));

    keys(cx, handle, "ctrl-shift-t");
    assert!(popup_open(cx, handle));
    assert_eq!(focus(cx, handle), "tag field");
    assert_eq!(options(cx, handle), ["#work ✓", "#home ✓"]);
    cx.simulate_input(handle, "ho");
    settle(cx);
    assert_eq!(options(cx, handle), ["#home ✓", "#holiday", "#project/holm", "Add #ho"]);
    keys(cx, handle, "tab tab enter");
    assert_eq!(chips(cx, handle), ["work", "home", "holiday"]);
    cx.simulate_input(handle, "big plans");
    keys(cx, handle, "enter");
    assert_eq!(chips(cx, handle), ["work", "home", "holiday", "big-plans"]);
    assert_eq!(fields(&a).0, ["work", "home", "holiday", "big-plans"]);

    keys(cx, handle, "tab tab enter");
    assert_eq!(options(cx, handle), ["#work ✓", "#holiday ✓*", "#big-plans ✓"]);
    assert_eq!(chips(cx, handle), ["work", "holiday", "big-plans"]);

    cx.update_window(handle, |_, window, cx| {
        window.click(gpui_kit::ElementId::NamedInteger("tag-option".into(), 2), cx);
    })
    .unwrap();
    settle(cx);
    assert_eq!(chips(cx, handle), ["work", "holiday"]);
    assert_eq!(options(cx, handle), ["#work ✓", "#holiday ✓*"]);
    assert_eq!(focus(cx, handle), "tag field");
    assert_eq!(fields(&a).0, ["work", "holiday"]);

    keys(cx, handle, "escape");
    assert_eq!(focus(cx, handle), "editor");
    assert!(!popup_open(cx, handle));

    keys(cx, handle, "ctrl-shift-p");
    assert!(screen(cx, handle, |window| window.try_find("pin-mark").is_some()));
    assert_eq!(fields(&a), (vec!["work".to_owned(), "holiday".to_owned()], true));

    keys(cx, handle, "ctrl-z");
    autosave(cx);
    assert_eq!(fields(&a), (vec!["work".to_owned(), "holiday".to_owned()], true));
    assert_eq!(
        modified(&a),
        std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(200),
        "tags and pins keep the modification time"
    );

    keys(cx, handle, "up");
    assert_eq!(focus(cx, handle), "tag field");
    keys(cx, handle, "escape");
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(editor_text(cx, handle), "# A\n");
}

#[gpui_kit::test]
fn tab_moves_through_the_tag_lines_and_wraps(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    two_tagged_notes(&fixture);
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-t shift-tab");
    assert_eq!(
        options(cx, handle),
        ["#home ✓", "#work/holm ✓*"],
        "no cursor: to the last"
    );
    keys(cx, handle, "tab");
    assert_eq!(options(cx, handle), ["#home ✓*", "#work/holm ✓"]);
    keys(cx, handle, "shift-tab");
    assert_eq!(options(cx, handle), ["#home ✓", "#work/holm ✓*"]);
    keys(cx, handle, "down up");
    assert_eq!(
        options(cx, handle),
        ["#home ✓", "#work/holm ✓*"],
        "an arrow does nothing"
    );

    cx.simulate_input(handle, "x");
    settle(cx);
    assert_eq!(options(cx, handle), ["Add #x"], "typing clears the cursor");
    keys(cx, handle, "tab tab");
    assert_eq!(options(cx, handle), ["Add #x*"]);

    keys(cx, handle, "ctrl-a backspace");
    assert_eq!(options(cx, handle), ["#home ✓", "#work/holm ✓"]);
    cx.simulate_input(handle, "zz");
    settle(cx);
    keys(cx, handle, "ctrl-a backspace tab");
    assert_eq!(
        options(cx, handle),
        ["#home ✓*", "#work/holm ✓"],
        "no cursor: to the first"
    );
    assert!(popup_open(cx, handle), "only Esc closes the popup");
}

fn tag_row_in_sight(cx: &mut TestAppContext, handle: AnyWindowHandle) -> bool {
    screen(cx, handle, |window| {
        window
            .try_find("tag-row")
            .is_some_and(|row| row.visible() && row.bounds().top() >= gpui_kit::px(0.))
    })
}

#[gpui_kit::test]
fn the_tag_row_scrolls_with_the_text_and_ctrl_shift_t_brings_it_back(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let lines: Vec<String> = (0..200).map(|line| format!("line {line}")).collect();
    fs::write(
        fixture.notes().join("Long.md"),
        format!(
            "---\ntags:\n  - work\n  - {}\n---\n# Long\n{}\n",
            "a".repeat(300),
            lines.join("\n")
        ),
    )
    .unwrap();
    let handle = open(cx, fixture.startup(None));
    assert!(tag_row_in_sight(cx, handle));
    let widest = screen(cx, handle, |window| {
        window
            .within("tag-row")
            .find_all_by_role(Role::ListItem)
            .iter()
            .map(|chip| chip.bounds().size.width)
            .fold(gpui_kit::px(0.), gpui_kit::Pixels::max)
    });
    assert!(widest <= crate::tags::CHIP_WIDTH, "a long tag ends with …");

    let row_shape = |cx: &mut TestAppContext| {
        screen(cx, handle, |window| {
            let mut shape = vec![window.find("tag-row").bounds()];
            shape.extend(
                window
                    .within("tag-row")
                    .find_all_by_role(Role::ListItem)
                    .iter()
                    .map(gpui_kit::test::ElementSnapshot::bounds),
            );
            shape
        })
    };
    let closed = row_shape(cx);
    keys(cx, handle, "ctrl-shift-t");
    assert!(popup_open(cx, handle));
    assert_eq!(row_shape(cx), closed);
    cx.simulate_input(handle, "x");
    settle(cx);
    assert_eq!(row_shape(cx), closed);
    keys(cx, handle, "escape");

    keys(cx, handle, "ctrl-end");
    assert!(!tag_row_in_sight(cx, handle));

    keys(cx, handle, "ctrl-shift-t");
    assert!(tag_row_in_sight(cx, handle));
    assert_eq!(focus(cx, handle), "tag field");
    assert_eq!(chips(cx, handle), ["work".to_owned(), "a".repeat(300)]);

    keys(cx, handle, "escape");
    assert_eq!(focus(cx, handle), "editor");
    assert!(!tag_row_in_sight(cx, handle));
    cx.simulate_input(handle, "end");
    settle(cx);
    assert!(editor_text(cx, handle).ends_with("line 199\nend"));
}

#[gpui_kit::test]
fn clicks_open_and_close_the_tag_popup(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(
        fixture.notes().join("A.md"),
        "---\ntags:\n  - work\n  - home\n---\n# A\n",
    )
    .unwrap();
    let handle = open(cx, fixture.startup(None));

    click_chip(cx, handle, 0);
    assert_eq!(focus(cx, handle), "tag field");
    assert_eq!(options(cx, handle), ["#work ✓*", "#home ✓"]);
    click_chip(cx, handle, 1);
    assert!(popup_open(cx, handle), "a click on the row keeps the popup open");
    assert_eq!(options(cx, handle), ["#work ✓", "#home ✓*"]);
    keys(cx, handle, "escape");
    let right = screen(cx, handle, |window| window.find("tag-row").bounds().size.width);
    cx.update_window(handle, |_, window, cx| {
        window.click_at("tag-row", gpui_kit::point(right / 2., gpui_kit::px(16.)), cx);
    })
    .unwrap();
    settle(cx);
    assert_eq!(focus(cx, handle), "tag field");
    assert_eq!(options(cx, handle), ["#work ✓", "#home ✓"]);

    let bottom = screen(cx, handle, |window| window.find("editor").bounds().size.height);
    cx.update_window(handle, |_, window, cx| {
        window.click_at(
            "editor",
            gpui_kit::point(gpui_kit::px(200.), bottom - gpui_kit::px(20.)),
            cx,
        );
    })
    .unwrap();
    settle(cx);
    assert!(!popup_open(cx, handle));
    assert_eq!(focus(cx, handle), "editor");
}

#[gpui_kit::test]
fn a_note_with_no_tags_shows_the_add_chip(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let a = fixture.notes().join("A.md");
    fs::write(&a, "# A\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    let row = bounds_of(cx, handle, "tag-row").unwrap();
    assert!(row.size.height > gpui_kit::px(0.));
    assert!(bounds_of(cx, handle, "tag-add").is_some());
    assert_eq!(chips(cx, handle), Vec::<String>::new());
    let popup_under_row = |cx: &mut TestAppContext| bounds_of(cx, handle, "tag-popup").unwrap().top() >= row.bottom();

    click(cx, handle, "tag-add");
    assert_eq!(focus(cx, handle), "tag field");
    assert!(popup_under_row(cx));
    keys(cx, handle, "escape");
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "ctrl-shift-t");
    assert_eq!(focus(cx, handle), "tag field");
    assert!(popup_under_row(cx));

    cx.simulate_input(handle, "work");
    keys(cx, handle, "enter");
    assert_eq!(chips(cx, handle), ["work"]);
    assert_eq!(bounds_of(cx, handle, "tag-add"), None);
    assert_eq!(bounds_of(cx, handle, "tag-row").unwrap(), row);
    assert_eq!(fields(&a).0, ["work"]);
}

#[gpui_kit::test]
fn the_tag_row_stays_on_one_line_and_folds_the_rest_into_a_chip(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("A.md"), "---\nid: 01A\ntags:\n  - work\n---\n# A\n").unwrap();
    fs::write(notes.join("B.md"), "# B\n[a](holm:01A)\n").unwrap();
    set_time(&notes.join("A.md"), 200);
    set_time(&notes.join("B.md"), 100);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(editor_text(cx, handle), "# A\n");
    assert_eq!(chips(cx, handle), ["work"]);
    assert_eq!(bounds_of(cx, handle, "tag-more"), None);
    let one_tag = bounds_of(cx, handle, "tag-row").unwrap().size.height;

    let tags: Vec<String> = (0..12)
        .map(|at| format!("a-long-tag-number-{at:02}-with-words"))
        .collect();
    let lines = tags.join("\n  - ");
    fixture.write_outside(cx, "A.md", &format!("---\nid: 01A\ntags:\n  - {lines}\n---\n# A\n"));
    let shown = chips(cx, handle);
    assert!(!shown.is_empty() && shown.len() < tags.len(), "{shown:?}");
    assert_eq!(shown, tags[..shown.len()]);
    let row = bounds_of(cx, handle, "tag-row").unwrap();
    assert_eq!(row.size.height, one_tag);
    let count = bounds_of(cx, handle, "backlinks-button").unwrap();
    let more = bounds_of(cx, handle, "tag-more").unwrap();
    assert_eq!(
        count.center().y,
        more.center().y,
        "the count is on the line of the chips"
    );
    assert!(more.right() <= count.left());
    assert!(count.right() <= row.right());

    cx.simulate_window_resize(handle, gpui_kit::size(gpui_kit::px(560.), gpui_kit::px(600.)));
    cx.run_until_parked();
    let narrow = chips(cx, handle);
    assert!(narrow.len() < shown.len(), "{narrow:?}");
    assert_eq!(bounds_of(cx, handle, "tag-row").unwrap().size.height, one_tag);

    cx.simulate_window_resize(handle, gpui_kit::size(gpui_kit::px(100.), gpui_kit::px(600.)));
    cx.run_until_parked();
    assert_eq!(chips(cx, handle), Vec::<String>::new());
    let more = bounds_of(cx, handle, "tag-more").unwrap();
    let count = bounds_of(cx, handle, "backlinks-button").unwrap();
    assert!(more.right() <= count.left(), "{more:?} {count:?}");
    cx.simulate_window_resize(handle, gpui_kit::size(gpui_kit::px(560.), gpui_kit::px(600.)));
    cx.run_until_parked();

    click(cx, handle, "tag-more");
    assert_eq!(focus(cx, handle), "tag field");
    let cursor = options(cx, handle).iter().position(|line| line.ends_with('*'));
    assert_eq!(cursor, Some(narrow.len()));
}

#[gpui_kit::test]
fn tags_rename_and_delete_in_all_notes_after_the_dialog(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    let a = notes_dir.join("A.md");
    let b = notes_dir.join("B.md");
    fs::write(&a, "---\ntags:\n  - work\n---\n# A\n").unwrap();
    fs::write(&b, "---\ntags:\n  - work/holm\n  - home\n---\n# B\n").unwrap();
    set_time(&a, 200);
    set_time(&b, 100);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(chips(cx, handle), ["work"]);

    keys(cx, handle, "ctrl-shift-l escape tab tab");
    assert_eq!(sidebar(cx, handle), "tree #work");
    keys(cx, handle, "f2");
    keys(cx, handle, "ctrl-a");
    cx.simulate_input(handle, "job");
    keys(cx, handle, "enter");
    assert_eq!(
        question(cx, handle).as_deref(),
        Some("Rename “#work” to “#job”? This changes 2 notes.")
    );
    keys(cx, handle, "escape");
    assert_eq!(question(cx, handle), None);
    assert_eq!(fields(&a).0, ["work"]);

    keys(cx, handle, "f2");
    keys(cx, handle, "ctrl-a");
    cx.simulate_input(handle, "job");
    keys(cx, handle, "enter");
    keys(cx, handle, "enter");
    assert_eq!(question(cx, handle), None);
    assert_eq!(fields(&a).0, ["job"]);
    assert_eq!(fields(&b).0, ["job/holm", "home"]);
    assert_eq!(sidebar(cx, handle), "tree #job");
    assert_eq!(chips(cx, handle), ["job"], "the open note shows the new name");

    // The tag dialogs have no "Do not show again", so Space does nothing.
    keys(cx, handle, "delete");
    assert_eq!(
        question(cx, handle).as_deref(),
        Some("Remove “#job” from all notes? This changes 2 notes.")
    );
    assert!(screen(cx, handle, |window| window.try_find("do-not-ask").is_none()));
    keys(cx, handle, "space enter");
    assert_eq!(fields(&a).0, Vec::<String>::new());
    assert_eq!(fields(&b).0, ["home"]);
    assert_eq!(chips(cx, handle), Vec::<String>::new());
    assert!(Config::load(fixture.config()).settings().ask_before_delete);

    assert_eq!(sidebar(cx, handle), "tree All notes");
    keys(cx, handle, "tab");
    assert_eq!(sidebar(cx, handle), "tree #home");
    keys(cx, handle, "delete");
    assert_eq!(
        question(cx, handle).as_deref(),
        Some("Remove “#home” from all notes? This changes 1 note.")
    );
    keys(cx, handle, "enter");
    assert_eq!(fields(&b).0, Vec::<String>::new());

    keys(cx, handle, "ctrl-shift-l");
    cx.simulate_input(handle, "x");
    autosave(cx);
    assert_eq!(fields(&a).0, Vec::<String>::new());
    assert_eq!(
        modified(&b),
        std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(100)
    );
}

#[gpui_kit::test]
fn a_tag_rename_shows_the_new_name_after_the_confirm(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("A.md"), "---\ntags:\n  - old\n---\n# A\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape tab");
    assert_eq!(tree(cx, handle), ["All notes", "#old*"]);
    type_new_name(cx, handle, "new");
    keys(cx, handle, "enter");
    assert!(question(cx, handle).is_some());
    assert_eq!(tree_field(cx, handle).as_deref(), Some("new"));
    assert_eq!(tree(cx, handle), ["All notes", "#old*"]);
    keys(cx, handle, "escape");
    assert_eq!(tree_field(cx, handle), None);
    assert_eq!(tree(cx, handle), ["All notes", "#old*"]);

    type_new_name(cx, handle, "new");
    keys(cx, handle, "enter");
    press(cx, handle, "enter");
    assert_eq!(question(cx, handle), None);
    assert_eq!(tree_field(cx, handle), None);
    assert_eq!(tree(cx, handle), ["All notes", "#new*"]);
    assert_eq!(tree_steps(cx, handle), [["All notes", "#new*"]]);
    settle(cx);
    assert_eq!(tree(cx, handle), ["All notes", "#new*"]);
    assert_eq!(fields(&notes_dir.join("A.md")).0, ["new"]);
}

#[gpui_kit::test]
fn a_tag_rename_onto_a_tag_merges_into_its_row(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("A.md"), "---\ntags:\n  - old\n---\n# A\n").unwrap();
    fs::write(notes_dir.join("B.md"), "---\ntags:\n  - new\n---\n# B\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape tab tab");
    assert_eq!(tree(cx, handle), ["All notes", "#new", "#old*"]);
    type_new_name(cx, handle, "new");
    keys(cx, handle, "enter");
    keys_now(cx, handle, "enter");
    assert_eq!(tree(cx, handle), ["All notes", "#new*"]);
    settle(cx);
    assert_eq!(tree(cx, handle), ["All notes", "#new*"]);
    assert_eq!(fields(&notes_dir.join("A.md")).0, ["new"]);
}

#[gpui_kit::test]
fn a_tag_rename_keeps_the_row_selected_in_every_frame(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(
        notes_dir.join("A.md"),
        "---\ntags:\n  - area/old/x\n  - area/new\n---\n# A\n",
    )
    .unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape tab space tab tab space");
    assert_eq!(
        tree(cx, handle),
        ["All notes", "#area", "#area/new", "#area/old*", "#area/old/x"]
    );
    type_new_name(cx, handle, "area/fresh");
    keys(cx, handle, "enter");
    press(cx, handle, "enter");
    assert_eq!(
        tree_steps(cx, handle),
        [vec!["All notes", "#area", "#area/fresh*", "#area/fresh/x", "#area/new"],]
    );

    keys(cx, handle, "f2");
    keys(cx, handle, "ctrl-a");
    cx.simulate_input(handle, "area/new");
    keys(cx, handle, "enter");
    press(cx, handle, "enter");
    let steps = tree_steps(cx, handle);
    assert!(
        steps.iter().all(|step| !step.contains(&"#area*".to_owned())),
        "{steps:?}"
    );
    assert_eq!(
        steps.last().unwrap(),
        &["All notes", "#area", "#area/new*", "#area/new/x"]
    );
}

/// A is open.
fn two_tagged_notes(fixture: &Fixture) -> (PathBuf, PathBuf) {
    let notes_dir = fixture.notes();
    let a = notes_dir.join("A.md");
    let b = notes_dir.join("B.md");
    fs::write(&a, "---\ntags:\n  - home\n  - work/holm\n---\n# A\n").unwrap();
    fs::write(&b, "---\ntags:\n  - work/holm\n---\n# B\n").unwrap();
    set_time(&a, 200);
    set_time(&b, 100);
    (a, b)
}

#[gpui_kit::test]
fn f2_in_the_tag_popup_renames_the_tag_in_the_sidebar(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let (a, b) = two_tagged_notes(&fixture);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(chips(cx, handle), ["home", "work/holm"]);

    keys(cx, handle, "ctrl-shift-t tab tab f2");
    assert!(!popup_open(cx, handle));
    assert_eq!(sidebar(cx, handle), "tree #work/holm");
    assert_eq!(name_field(cx, handle).as_deref(), Some("work/holm"));

    keys(cx, handle, "ctrl-a");
    cx.simulate_input(handle, "job");
    keys(cx, handle, "enter");
    assert_eq!(
        question(cx, handle).as_deref(),
        Some("Rename “#work/holm” to “#job”? This changes 2 notes.")
    );
    keys(cx, handle, "enter");
    assert_eq!(fields(&a).0, ["home", "job"]);
    assert_eq!(fields(&b).0, ["job"]);
    assert_eq!(chips(cx, handle), ["home", "job"]);
}

#[gpui_kit::test]
fn a_new_tag_renames_in_all_notes_with_no_wait(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let a = fixture.notes().join("A.md");
    fs::write(&a, "# A\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys_now(cx, handle, "ctrl-shift-t");
    cx.simulate_input(handle, "fresh");
    // F2 comes before the write of the new tag ends.
    press(cx, handle, "enter");
    press(cx, handle, "tab");
    press(cx, handle, "f2");
    cx.run_until_parked();
    assert_eq!(name_field(cx, handle).as_deref(), Some("fresh"));
    assert_eq!(tree(cx, handle), ["All notes", "#fresh*"]);
    assert_eq!(tree_field(cx, handle).as_deref(), Some("fresh"));
    keys(cx, handle, "ctrl-a");
    cx.simulate_input(handle, "job");
    keys_now(cx, handle, "enter");
    assert_eq!(
        question(cx, handle).as_deref(),
        Some("Rename “#fresh” to “#job”? This changes 1 note.")
    );
    keys_now(cx, handle, "enter");
    assert_eq!(fields(&a).0, ["job"]);
}

#[gpui_kit::test]
fn f2_in_the_tag_popup_shows_a_hidden_sidebar(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    two_tagged_notes(&fixture);
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l ctrl-shift-l");
    assert_eq!(sidebar(cx, handle), "hidden");

    keys(cx, handle, "ctrl-shift-t tab f2");
    assert!(!popup_open(cx, handle));
    assert_eq!(sidebar(cx, handle), "tree #home");
    assert_eq!(name_field(cx, handle).as_deref(), Some("home"));
    keys(cx, handle, "escape");
    assert_eq!(focus(cx, handle), "sidebar");
    assert_eq!(name_field(cx, handle), None);
}

#[gpui_kit::test]
fn f2_in_the_tag_popup_needs_a_tag_line(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let (a, _) = two_tagged_notes(&fixture);
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l ctrl-shift-l");
    assert_eq!(sidebar(cx, handle), "hidden");

    keys(cx, handle, "ctrl-shift-t f2");
    assert!(popup_open(cx, handle), "with no cursor, F2 does nothing");
    cx.simulate_input(handle, "new");
    settle(cx);
    assert_eq!(options(cx, handle), ["Add #new"]);
    keys(cx, handle, "tab f2");
    assert!(popup_open(cx, handle));
    assert_eq!(options(cx, handle), ["Add #new*"]);
    assert_eq!(focus(cx, handle), "tag field");
    assert_eq!(sidebar(cx, handle), "hidden");
    assert_eq!(fields(&a).0, ["home", "work/holm"]);
}

#[gpui_kit::test]
fn a_child_tag_delete_selects_the_parent_tag_while_it_exists(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    let a = notes_dir.join("A.md");
    let b = notes_dir.join("B.md");
    fs::write(&a, "---\ntags:\n  - work\n  - work/holm\n---\n# A\n").unwrap();
    fs::write(&b, "---\ntags:\n  - home/garden\n---\n# B\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape tab tab space tab");
    assert_eq!(sidebar(cx, handle), "tree #work/holm");
    keys(cx, handle, "delete");
    keys(cx, handle, "enter");
    assert_eq!(fields(&a).0, ["work"]);
    assert_eq!(sidebar(cx, handle), "tree #work");

    // No note keeps the tag home, so it goes with its child.
    keys(cx, handle, "shift-tab space tab");
    assert_eq!(sidebar(cx, handle), "tree #home/garden");
    keys(cx, handle, "delete");
    keys(cx, handle, "enter");
    assert_eq!(fields(&b).0, Vec::<String>::new());
    assert_eq!(sidebar(cx, handle), "tree All notes");
}
