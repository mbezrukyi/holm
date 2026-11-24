use std::fs;
use std::path::Path;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Role, TestAppContext};

use super::{
    Fixture, backlink_count, backlinks, chips, click, editor_text, focus, folders, keys, labels, message, open,
    question, read, screen, set_time, settle,
};
use crate::text;

fn picker_text(cx: &mut TestAppContext, handle: AnyWindowHandle) -> String {
    screen(cx, handle, |window| {
        window
            .within("link-picker")
            .find_all_by_role(Role::TextInput)
            .first()
            .and_then(|field| field.value().map(str::to_owned))
            .unwrap_or_default()
    })
}

fn id_of(path: &Path) -> String {
    holm_markdown::NoteFile::parse(&read(path))
        .id()
        .expect("the note has an id")
}

#[gpui_kit::test]
fn the_link_picker_links_notes_headings_and_new_notes(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::create_dir(notes.join("work")).unwrap();
    fs::write(notes.join("Alpha.md"), "# Alpha\n").unwrap();
    fs::write(notes.join("work/Beta.md"), "# Beta\n\n## Bug list\n\n## Ideas\n").unwrap();
    set_time(&notes.join("Alpha.md"), 300);
    set_time(&notes.join("work/Beta.md"), 200);
    let handle = open(cx, fixture.startup(None));
    let options = |cx: &mut TestAppContext| labels(cx, handle, "link-picker", Role::ListBoxOption);
    assert_eq!(editor_text(cx, handle), "# Alpha\n");
    keys(cx, handle, "ctrl-end");

    // The note had no id, so it gets one.
    keys(cx, handle, "ctrl-k");
    assert_eq!(question(cx, handle).as_deref(), Some(text::LINK_TO_NOTE));
    assert_eq!(options(cx), ["Alpha*", "Beta"]);
    assert_eq!(folders(cx, handle, "link-picker"), [None, Some("work".to_owned())]);
    cx.simulate_input(handle, "bet");
    settle(cx);
    assert_eq!(options(cx), ["Beta*", "Create “bet”"]);
    keys(cx, handle, "enter");
    assert_eq!(question(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");
    let beta = id_of(&notes.join("work/Beta.md"));
    assert_eq!(editor_text(cx, handle), format!("# Alpha\n[Beta](holm:{beta})"));

    cx.simulate_input(handle, " ");
    keys(cx, handle, "ctrl-k");
    cx.simulate_input(handle, "be");
    settle(cx);
    keys(cx, handle, "tab tab");
    assert_eq!(options(cx), ["Beta*", "Create “be”"], "Tab wraps");
    cx.simulate_input(handle, "#");
    settle(cx);
    assert_eq!(options(cx), ["Beta*", "Bug list", "Ideas"]);
    assert_eq!(picker_text(cx, handle), "Beta#");
    cx.simulate_input(handle, "bug");
    settle(cx);
    keys(cx, handle, "enter");
    assert_eq!(
        editor_text(cx, handle),
        format!("# Alpha\n[Beta](holm:{beta}) [Bug list](<holm:{beta}#Bug list>)")
    );

    cx.simulate_input(handle, " ");
    keys(cx, handle, "ctrl-k");
    cx.simulate_input(handle, "Gamma");
    settle(cx);
    assert_eq!(options(cx), ["Create “Gamma”*"]);
    keys(cx, handle, "enter");
    let gamma_path = notes.join("Gamma.md");
    assert_eq!(holm_markdown::NoteFile::parse(&read(&gamma_path)).body(), "");
    let gamma = id_of(&gamma_path);
    let text = editor_text(cx, handle);
    assert!(text.starts_with("# Alpha\n"));
    assert!(text.ends_with(&format!(" [Gamma](holm:{gamma})")));

    keys(cx, handle, "ctrl-k");
    assert_eq!(options(cx), ["Gamma*"]);
    keys(cx, handle, "ctrl-a backspace");
    cx.simulate_input(handle, "Beta");
    settle(cx);
    keys(cx, handle, "enter");
    assert!(
        editor_text(cx, handle).ends_with(&format!(" [Gamma](holm:{beta})")),
        "{}",
        editor_text(cx, handle)
    );

    let before = editor_text(cx, handle);
    keys(cx, handle, "ctrl-k");
    cx.simulate_input(handle, "Alpha");
    keys(cx, handle, "escape");
    assert_eq!(question(cx, handle), None);
    assert_eq!(editor_text(cx, handle), before);
}

#[gpui_kit::test]
fn links_open_notes_at_headings_and_backlinks_lead_back(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(
        notes.join("Alpha.md"),
        "---\nid: 01A\n---\n# Alpha\n[ideas](holm:01B#Ideas)\n[gone](holm:01X)\n",
    )
    .unwrap();
    fs::write(notes.join("Beta.md"), "---\nid: 01B\n---\n# Beta\n\ntext\n\n## Ideas\n").unwrap();
    set_time(&notes.join("Alpha.md"), 300);
    set_time(&notes.join("Beta.md"), 200);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(backlink_count(cx, handle), None);

    keys(cx, handle, "down ctrl-enter");
    assert_eq!(editor_text(cx, handle), "# Beta\n\ntext\n\n## Ideas\n");
    cx.simulate_input(handle, "X");
    assert_eq!(editor_text(cx, handle), "# Beta\n\ntext\n\nX## Ideas\n");

    assert_eq!(backlink_count(cx, handle).as_deref(), Some("← 1 note"));
    // The heading is at the top of the view, so the tag row is out of sight.
    keys(cx, handle, "ctrl-home");
    click(cx, handle, "backlinks-button");
    assert_eq!(backlinks(cx, handle), ["Alpha*"]);
    cx.update_window(handle, |_, window, cx| {
        window.click(gpui_kit::ElementId::NamedInteger("backlink".into(), 0), cx);
    })
    .unwrap();
    settle(cx);
    assert_eq!(focus(cx, handle), "editor");
    assert!(editor_text(cx, handle).starts_with("# Alpha\n"));
    assert_eq!(read(&notes.join("Beta.md")).lines().last(), Some("X## Ideas"));

    keys(cx, handle, "down down ctrl-enter");
    assert_eq!(message(cx, handle).as_deref(), Some(text::NOTE_GONE));
}

#[gpui_kit::test]
fn ctrl_shift_b_opens_the_backlinks_of_the_note(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("Target.md"), "---\nid: 01T\n---\n# Target\n").unwrap();
    fs::write(notes.join("Old.md"), "# Old\n[t](holm:01T)\n").unwrap();
    fs::write(notes.join("New.md"), "# New\n[t](holm:01T)\n").unwrap();
    set_time(&notes.join("Target.md"), 300);
    set_time(&notes.join("New.md"), 200);
    set_time(&notes.join("Old.md"), 100);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(editor_text(cx, handle), "# Target\n");
    assert_eq!(chips(cx, handle), Vec::<String>::new());
    assert_eq!(backlink_count(cx, handle).as_deref(), Some("← 2 notes"));
    assert!(screen(cx, handle, |window| window.find("tag-row").bounds().size.height) > gpui_kit::px(0.));

    keys(cx, handle, "ctrl-shift-b");
    assert_eq!(backlinks(cx, handle), ["New*", "Old"]);
    keys(cx, handle, "escape");
    assert_eq!(backlinks(cx, handle), Vec::<String>::new());
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "ctrl-shift-b shift-tab");
    assert_eq!(backlinks(cx, handle), ["New", "Old*"]);
    keys(cx, handle, "tab");
    assert_eq!(backlinks(cx, handle), ["New*", "Old"]);
    keys(cx, handle, "down tab");
    assert_eq!(backlinks(cx, handle), ["New", "Old*"]);
    keys(cx, handle, "enter");
    assert_eq!(editor_text(cx, handle), "# Old\n[t](holm:01T)\n");
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(backlinks(cx, handle), Vec::<String>::new());
}

#[gpui_kit::test]
fn a_note_with_no_backlinks_has_no_count(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("Target.md"), "---\nid: 01T\n---\n# Target\n").unwrap();
    fs::write(notes.join("Other.md"), "# Other\n").unwrap();
    set_time(&notes.join("Target.md"), 300);
    set_time(&notes.join("Other.md"), 100);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(backlink_count(cx, handle), None);
    keys(cx, handle, "ctrl-shift-b");
    assert_eq!(backlinks(cx, handle), Vec::<String>::new());
    assert_eq!(focus(cx, handle), "editor");

    fixture.write_outside(cx, "Other.md", "# Other\n[t](holm:01T)\n");
    assert_eq!(backlink_count(cx, handle).as_deref(), Some("← 1 note"));
}

#[gpui_kit::test]
fn ctrl_o_embeds_a_file_where_it_is(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::create_dir(notes.join("work")).unwrap();
    fs::create_dir(notes.join("docs")).unwrap();
    fs::write(notes.join("work/a.md"), "# A\n").unwrap();
    fs::write(notes.join("docs/plan.pdf"), "pdf").unwrap();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-end ctrl-o");
    assert!(cx.did_prompt_for_paths());
    let picked = notes.join("docs/plan.pdf");
    cx.simulate_path_prompt_response(move |_| Some(vec![picked]));
    settle(cx);
    assert_eq!(editor_text(cx, handle), "# A\n![plan.pdf](../docs/plan.pdf)");
    assert_eq!(fs::read_dir(notes.join("work")).unwrap().count(), 1);
}

#[gpui_kit::test]
fn ctrl_o_asks_for_files_and_ctrl_j_for_folders(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("a.md"), "# A\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    for (key, folders) in [("ctrl-o", false), ("ctrl-j", true)] {
        keys(cx, handle, key);
        cx.simulate_path_prompt_response(|options| {
            assert_eq!(options.files, !folders, "{key}");
            assert_eq!(options.directories, folders, "{key}");
            assert!(options.multiple, "{key}");
            assert_eq!(options.prompt.as_deref(), Some(text::LINK_FILE), "{key}");
            None
        });
        settle(cx);
    }
    assert_eq!(editor_text(cx, handle), "# A\n");
}

#[gpui_kit::test]
fn ctrl_j_embeds_a_folder_at_the_cursor(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::create_dir(notes.join("work")).unwrap();
    fs::create_dir(notes.join("docs")).unwrap();
    fs::write(notes.join("work/a.md"), "# A\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-end ctrl-j");
    let picked = notes.join("docs");
    cx.simulate_path_prompt_response(move |_| Some(vec![picked]));
    settle(cx);
    cx.simulate_input(handle, "!");
    assert_eq!(editor_text(cx, handle), "# A\n![docs](../docs)!");
}
