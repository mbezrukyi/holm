use std::fs;
use std::path::PathBuf;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, Role, TestAppContext};

use super::{
    Fixture, bounds_of, click, empty_pane, focus, folder_screen, keys, label, labels, message, open, question, read,
    screen, state, state_menu,
};
use crate::text;

/// Gray, with the count of the problems.
fn problem_mark(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    label(cx, handle, "problem-mark")
}

/// The selected row has `*`.
fn problem_list(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    labels(cx, handle, "problem-list", Role::ListBoxOption)
}

#[gpui_kit::test]
fn a_broken_config_is_a_problem_in_the_message_line(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    fixture.write_config("version = [broken\n");
    let log = PathBuf::from("/state/holm/holm.log");
    let handle = open(cx, fixture.startup(Some(log.clone())));
    assert_eq!(folder_screen(cx, handle).as_deref(), Some(text::ASK_FOLDER));
    assert_eq!(message(cx, handle), Some(text::crashed(&log).to_string()));
    assert_eq!(problem_mark(cx, handle), Some(text::problem_mark(1)));

    keys(cx, handle, "ctrl-shift-e");
    assert_eq!(problem_list(cx, handle), [format!("{}*", text::CONFIG_WRITE)]);
    keys(cx, handle, "tab");
    assert_eq!(
        problem_list(cx, handle),
        [format!("{}*", text::CONFIG_WRITE)],
        "the one problem wraps to itself"
    );
    keys(cx, handle, "down shift-tab");
    assert_eq!(problem_list(cx, handle).len(), 1);
    keys(cx, handle, "escape");
    assert_eq!(problem_list(cx, handle), Vec::<String>::new());
    click(cx, handle, "problem-mark");
    assert_eq!(problem_list(cx, handle).len(), 1);
    click(cx, handle, "problem-mark");
    assert_eq!(problem_list(cx, handle), Vec::<String>::new());

    click(cx, handle, "use-default");
    assert!(empty_pane(cx, handle));
    assert_eq!(read(&fixture.config()), "version = [broken\n");
    assert_eq!(problem_mark(cx, handle), Some(text::problem_mark(1)));
}

#[gpui_kit::test]
fn the_message_line_does_not_move_the_folder_question(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let log = PathBuf::from("/state/holm/holm.log");
    let handle = open(cx, fixture.startup(Some(log.clone())));
    assert_eq!(message(cx, handle), Some(text::crashed(&log).to_string()));
    let with = bounds_of(cx, handle, "folder-question").unwrap();
    let line = bounds_of(cx, handle, "message-line").unwrap();
    let screen = bounds_of(cx, handle, "folder-screen").unwrap();
    assert_eq!(line.bottom(), screen.bottom(), "the line lies over the screen");

    keys(cx, handle, "escape");
    assert_eq!(message(cx, handle), None);
    assert_eq!(bounds_of(cx, handle, "folder-question"), Some(with));
}

#[gpui_kit::test]
fn an_answer_shows_under_the_note_until_the_next_key(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "[gone](holm:01X)\n").unwrap();
    let log = PathBuf::from("/state/holm/holm.log");
    let handle = open(cx, fixture.startup(Some(log.clone())));
    assert_eq!(message(cx, handle), Some(text::crashed(&log).to_string()));
    let (line, editor) = screen(cx, handle, |window| {
        (window.find("message-line").bounds(), window.find("editor").bounds())
    });
    assert!(line.top() >= editor.bottom(), "the line is under the note");

    keys(cx, handle, "right");
    assert_eq!(message(cx, handle), None);
    assert!(
        screen(cx, handle, |window| window.try_find("message-line")).is_none(),
        "an empty line does not show"
    );

    keys(cx, handle, "ctrl-enter");
    assert_eq!(message(cx, handle).as_deref(), Some(text::NOTE_GONE));
    keys(cx, handle, "escape");
    assert_eq!(message(cx, handle), None);
}

#[gpui_kit::test]
fn ctrl_shift_e_with_the_settings_open_opens_the_problem_list(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    fixture.write_config("version = [broken\n");
    let handle = open(cx, fixture.startup(None));
    click(cx, handle, "use-default");
    assert!(empty_pane(cx, handle));

    keys(cx, handle, "ctrl-shift-s");
    assert_eq!(question(cx, handle).as_deref(), Some(text::SETTINGS));
    keys(cx, handle, "ctrl-shift-e");
    assert_eq!(problem_list(cx, handle), [format!("{}*", text::CONFIG_WRITE)]);
}

#[gpui_kit::test]
fn ctrl_shift_d_with_the_tree_focused_opens_the_state_menu(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    cx.simulate_input(handle, "mine ");
    fixture.write_outside(cx, "Note.md", "theirs\n");
    assert_eq!(state(cx, handle).as_deref(), Some(text::CHANGED_ON_DISK));

    keys(cx, handle, "ctrl-shift-l");
    assert_eq!(focus(cx, handle), "sidebar");
    keys(cx, handle, "ctrl-shift-d");
    assert_eq!(state_menu(cx, handle), ["Keep mine*", "Reload"]);
}
