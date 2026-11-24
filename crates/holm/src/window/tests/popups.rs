use std::fs;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext, Window};

use super::{
    Fixture, backlinks, click, click_chip, editor_text, focus, keys, name_field, open, popup_open, question, screen,
    set_time, settle, sidebar, three_notes,
};
use crate::text;

/// Whether the last drawn frame shows `id`. It draws no new frame, so it shows a frame that the window still owes.
fn drawn(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) -> bool {
    cx.update_window(handle, |_, window, _| shows(window, id)).unwrap()
}

fn shows(window: &Window, id: &'static str) -> bool {
    window.try_find(id).is_some_and(|element| element.visible())
}

/// Work that waits for the end of the draw has not run yet.
fn first_frame_has(cx: &mut TestAppContext, handle: AnyWindowHandle, key: &'static str, id: &'static str) -> bool {
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_keystroke(gpui_kit::Keystroke::parse(key).unwrap(), cx);
        window.render_frame(cx);
        shows(window, id)
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_popup_closes_in_the_frame_of_the_focus_move(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    let target = "---\nid: 01T\ntags:\n  - work\n---\n# Target\n";
    fs::write(notes.join("Target.md"), target).unwrap();
    fs::write(notes.join("Other.md"), "# Other\n[t](holm:01T)\n").unwrap();
    set_time(&notes.join("Target.md"), 300);
    set_time(&notes.join("Other.md"), 100);
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l ctrl-shift-l ctrl-shift-t");
    assert_eq!(sidebar(cx, handle), "hidden");
    assert!(!first_frame_has(cx, handle, "ctrl-shift-l", "tag-popup"));
    settle(cx);
    assert!(!drawn(cx, handle, "tag-popup"));
    assert_eq!(focus(cx, handle), "sidebar");

    keys(cx, handle, "escape ctrl-shift-b");
    assert!(drawn(cx, handle, "backlinks"));
    assert!(!first_frame_has(cx, handle, "ctrl-shift-l", "backlinks"));
    settle(cx);
    assert!(!drawn(cx, handle, "backlinks"));

    // F2 moves the focus in an event after the key, so `first_frame_has` cannot see it.
    keys(cx, handle, "escape ctrl-shift-t tab f2");
    assert!(!drawn(cx, handle, "tag-popup"));
    assert_eq!(name_field(cx, handle).as_deref(), Some("work"));
}

/// Clicks near the bottom left of the editor, where no popup panel lies.
fn click_below(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    let bottom = screen(cx, handle, |window| window.find("editor").bounds().size.height);
    cx.update_window(handle, |_, window, cx| {
        window.click_at(
            "editor",
            gpui_kit::point(gpui_kit::px(20.), bottom - gpui_kit::px(20.)),
            cx,
        );
    })
    .unwrap();
    settle(cx);
}

#[gpui_kit::test]
fn a_click_outside_a_popup_never_reaches_the_parts_under_it(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    three_notes(&fixture.notes());
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l ctrl-shift-o");
    click_below(cx, handle);
    assert_eq!(question(cx, handle), None);
    assert_eq!(focus(cx, handle), "sidebar");
    keys(cx, handle, "ctrl-shift-o");
    assert_eq!(question(cx, handle).as_deref(), Some(text::QUICK_OPEN));

    keys(cx, handle, "escape ctrl-shift-l ctrl-k");
    assert_eq!(question(cx, handle).as_deref(), Some(text::LINK_TO_NOTE));
    click_below(cx, handle);
    assert_eq!(question(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(editor_text(cx, handle), "# Plan\nbuy milk\n");

    keys(cx, handle, "ctrl-shift-l delete");
    let delete = question(cx, handle);
    assert!(delete.is_some());
    click_below(cx, handle);
    assert_eq!(question(cx, handle), delete);
    assert_eq!(focus(cx, handle), "other");
}

#[gpui_kit::test]
fn the_tag_popup_and_the_backlinks_list_take_turns(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    let target = "---\nid: 01T\ntags:\n  - work\n---\n# Target\n";
    fs::write(notes.join("Target.md"), target).unwrap();
    fs::write(notes.join("Other.md"), "# Other\n[t](holm:01T)\n").unwrap();
    set_time(&notes.join("Target.md"), 300);
    set_time(&notes.join("Other.md"), 100);
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-b");
    assert_eq!(backlinks(cx, handle), ["Other*"]);
    click_chip(cx, handle, 0);
    assert_eq!(backlinks(cx, handle), Vec::<String>::new());
    assert!(popup_open(cx, handle));
    assert_eq!(focus(cx, handle), "tag field");

    click(cx, handle, "backlinks-button");
    settle(cx);
    assert_eq!(backlinks(cx, handle), ["Other*"]);
    assert!(!popup_open(cx, handle));

    keys(cx, handle, "escape");
    assert_eq!(backlinks(cx, handle), Vec::<String>::new());
    assert!(!popup_open(cx, handle));
    assert_eq!(focus(cx, handle), "editor");
}

#[gpui_kit::test]
fn a_click_in_the_sidebar_keeps_the_focus_after_the_tag_popup_closes(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("A.md"), "---\ntags:\n  - work\n---\n# A\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    click_chip(cx, handle, 0);
    assert_eq!(focus(cx, handle), "tag field");
    cx.update_window(handle, |_, window, cx| {
        window.click(gpui_kit::ElementId::NamedInteger("note-row".into(), 0), cx);
    })
    .unwrap();
    settle(cx);
    assert!(!popup_open(cx, handle));
    assert_eq!(focus(cx, handle), "sidebar");
}

#[gpui_kit::test]
fn a_popup_over_the_tag_field_gives_the_focus_home_when_it_closes(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "a\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    for popup in ["ctrl-?", "ctrl-shift-s"] {
        keys(cx, handle, "ctrl-shift-t");
        assert_eq!(focus(cx, handle), "tag field");
        // The popup takes the focus, so the tag field closes.
        keys(cx, handle, popup);
        keys(cx, handle, "escape");
        assert_eq!(question(cx, handle), None);
        assert_eq!(focus(cx, handle), "editor", "after {popup}");
    }
    keys(cx, handle, "ctrl-?");
    assert_eq!(question(cx, handle).as_deref(), Some(text::SHORTCUT_LIST));
}
