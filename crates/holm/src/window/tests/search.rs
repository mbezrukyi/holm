use std::fs;

use gpui_kit::{AnyWindowHandle, Role, TestAppContext};

use super::{
    Fixture, editor_text, focus, folders, keys, labels, notes_list, open, press, question, set_time, settle, sidebar,
    three_notes,
};
use crate::config::{Config, SavedSidebarView, SidebarItem};
use crate::text;

#[gpui_kit::test]
fn search_lists_the_notes_that_match_and_esc_returns(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    three_notes(&fixture.notes());
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l escape");
    assert_eq!(sidebar(cx, handle), "tree All notes");

    keys(cx, handle, "ctrl-shift-f");
    assert_eq!(focus(cx, handle), "search");
    cx.simulate_input(handle, "read");
    settle(cx);
    assert_eq!(sidebar(cx, handle), "list Search: read");
    assert_eq!(notes_list(cx, handle), ["Goals"]);

    keys(cx, handle, "down");
    assert_eq!(focus(cx, handle), "search", "Down stays in the field");
    keys(cx, handle, "enter");
    assert_eq!(focus(cx, handle), "sidebar");
    assert_eq!(editor_text(cx, handle), "# Goals\nread more\n");
    assert_eq!(notes_list(cx, handle), ["Goals*"]);

    keys(cx, handle, "ctrl-shift-f");
    cx.simulate_input(handle, "work");
    settle(cx);
    assert_eq!(sidebar(cx, handle), "list Search: work");
    assert_eq!(notes_list(cx, handle), ["Goals*"]);

    keys(cx, handle, "escape");
    assert_eq!(sidebar(cx, handle), "tree All notes");
    assert_eq!(focus(cx, handle), "sidebar");

    keys(cx, handle, "ctrl-shift-f");
    cx.simulate_input(handle, "zebra");
    settle(cx);
    assert_eq!(notes_list(cx, handle), Vec::<String>::new());
    keys(cx, handle, "escape");
    assert_eq!(sidebar(cx, handle), "tree All notes");
    assert_eq!(focus(cx, handle), "sidebar");

    // The view is not part of the config.
    keys(cx, handle, "ctrl-shift-l");
    assert_eq!(sidebar(cx, handle), "hidden");
    keys(cx, handle, "ctrl-shift-f");
    cx.simulate_input(handle, "milk");
    settle(cx);
    assert_eq!(notes_list(cx, handle), ["Plan"]);
    let settings = Config::load(fixture.config()).settings().clone();
    assert_eq!(
        settings.sidebar.view,
        SavedSidebarView {
            list: false,
            item: SidebarItem::All,
        }
    );
    assert!(!settings.sidebar.hidden);

    keys(cx, handle, "enter enter");
    keys(cx, handle, "ctrl-f");
    assert_eq!(focus(cx, handle), "editor");
    keys(cx, handle, "ctrl-shift-f");
    assert_eq!(focus(cx, handle), "search");
}

fn list_steps(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<Vec<String>> {
    let mut steps = vec![notes_list(cx, handle)];
    while cx.executor().tick() {
        let now = notes_list(cx, handle);
        if steps.last() != Some(&now) {
            steps.push(now);
        }
    }
    steps
}

#[gpui_kit::test]
fn a_search_during_an_outside_rename_keeps_the_note_in_every_frame(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("Home.md"), "# Home\n").unwrap();
    fs::write(notes_dir.join("Trip.md"), "# Trip\n").unwrap();
    set_time(&notes_dir.join("Home.md"), 200);
    set_time(&notes_dir.join("Trip.md"), 100);
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-f");
    cx.simulate_input(handle, "tri");
    settle(cx);
    assert_eq!(notes_list(cx, handle), ["Trip"]);

    // The core has the rename, and the sidebar has not seen it yet.
    fs::rename(notes_dir.join("Trip.md"), notes_dir.join("Trips.md")).unwrap();
    let feed = fixture.feed.lock().unwrap();
    feed.as_ref().unwrap().changed(["Trip.md", "Trips.md"]).unwrap();
    drop(feed);
    press(cx, handle, "p");

    assert_eq!(list_steps(cx, handle), [vec!["Trip"], vec!["Trips"]]);
}

#[gpui_kit::test]
fn quick_open_opens_a_note_by_its_title(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    three_notes(&fixture.notes());
    let handle = open(cx, fixture.startup(None));
    let options = |cx: &mut TestAppContext| labels(cx, handle, "quick-open", Role::ListBoxOption);
    assert_eq!(editor_text(cx, handle), "# Plan\nbuy milk\n");

    keys(cx, handle, "ctrl-shift-o");
    assert_eq!(question(cx, handle).as_deref(), Some(text::QUICK_OPEN));
    assert_eq!(options(cx), ["Plan*", "Goals", "Top"]);

    // Titles only: "milk" is in the text of Plan.
    cx.simulate_input(handle, "milk");
    settle(cx);
    assert_eq!(options(cx), Vec::<String>::new());

    keys(cx, handle, "escape");
    assert_eq!(question(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "ctrl-shift-l ctrl-shift-o");
    cx.simulate_input(handle, "o");
    settle(cx);
    assert_eq!(options(cx), ["Goals*", "Top"]);
    keys(cx, handle, "down");
    assert_eq!(options(cx), ["Goals*", "Top"], "an arrow does nothing");
    keys(cx, handle, "shift-tab");
    assert_eq!(options(cx), ["Goals", "Top*"], "Shift+Tab wraps");
    keys(cx, handle, "tab");
    assert_eq!(options(cx), ["Goals*", "Top"], "Tab wraps");
    keys(cx, handle, "tab enter");
    assert_eq!(question(cx, handle), None);
    assert_eq!(editor_text(cx, handle), "# Top\n");
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(notes_list(cx, handle), ["Plan", "Goals", "Top*"]);

    keys(cx, handle, "ctrl-shift-l ctrl-shift-o escape");
    assert_eq!(focus(cx, handle), "sidebar");
}

#[gpui_kit::test]
fn quick_open_shows_the_folder_of_each_note(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::create_dir_all(notes.join("work/old")).unwrap();
    fs::write(notes.join("Plan.md"), "# Plan\n").unwrap();
    fs::write(notes.join("work/old/Beta.md"), "# Beta\n").unwrap();
    set_time(&notes.join("Plan.md"), 300);
    set_time(&notes.join("work/old/Beta.md"), 200);
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-o");
    assert_eq!(labels(cx, handle, "quick-open", Role::ListBoxOption), ["Plan*", "Beta"]);
    assert_eq!(folders(cx, handle, "quick-open"), [None, Some("work/old".to_owned())]);
}
