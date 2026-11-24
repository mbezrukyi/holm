use std::fs;

use gpui_kit::{AppContext as _, TestAppContext};

use super::{
    Fixture, autosave, click, close, editor_text, files, keys, message, modified, new_note, open, question, read,
    read_only, state, state_reason, three_notes, window_open,
};
use crate::config::Config;
use crate::text;

#[gpui_kit::test]
fn a_close_with_a_saved_note_quits_at_once(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    cx.simulate_input(handle, "a");
    autosave(cx);
    assert!(!close(cx, handle));
}

#[gpui_kit::test]
fn a_close_saves_the_typed_text_first(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    cx.simulate_input(handle, "a");
    assert!(!close(cx, handle));
    assert!(read(&note).ends_with("aold\n"));
}

#[gpui_kit::test]
fn a_click_between_two_closes_still_lets_the_second_close_close(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("Note.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    read_only(&notes_dir, true);
    cx.simulate_input(handle, "a");
    assert!(close(cx, handle));
    assert_eq!(message(cx, handle).as_deref(), Some(text::CLOSE_AGAIN));
    click(cx, handle, "message-state");
    assert_eq!(state_reason(cx, handle).as_deref(), Some(text::WRITE));

    assert!(!close(cx, handle), "the second close closes");
    read_only(&notes_dir, false);
}

#[gpui_kit::test]
fn a_close_with_a_note_that_cannot_be_saved_asks_for_a_second_close(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("Note.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    read_only(&notes_dir, true);
    cx.simulate_input(handle, "a");
    assert!(close(cx, handle));
    assert_eq!(message(cx, handle).as_deref(), Some(text::CLOSE_AGAIN));
    assert_eq!(state(cx, handle).as_deref(), Some(text::NOT_SAVED));

    assert!(!close(cx, handle), "the second close closes");
    read_only(&notes_dir, false);
}

/// A key that changes no text keeps the second close.
#[gpui_kit::test]
fn a_failed_close_then_an_arrow_key_then_a_close_quits(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("Note.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    read_only(&notes_dir, true);
    cx.simulate_input(handle, "a");
    assert!(close(cx, handle));
    keys(cx, handle, "right");
    assert_eq!(message(cx, handle), None);
    assert!(!close(cx, handle));
    read_only(&notes_dir, false);
}

/// Typed text after a failed close stops the second close, so the text is never lost.
#[gpui_kit::test]
fn a_failed_close_then_a_typed_letter_then_a_close_does_not_quit(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("Note.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    read_only(&notes_dir, true);
    cx.simulate_input(handle, "a");
    assert!(close(cx, handle));
    cx.simulate_input(handle, "b");
    assert!(close(cx, handle), "the close tries to save again");
    assert_eq!(message(cx, handle).as_deref(), Some(text::CLOSE_AGAIN));

    assert!(!close(cx, handle));
    read_only(&notes_dir, false);
}

#[gpui_kit::test]
fn close_after_an_outside_delete_writes_no_file(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    three_notes(&notes_dir);
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-end");
    cx.simulate_input(handle, "mine");
    let times = || ["Goals.md", "Top.md"].map(|note| modified(&notes_dir.join(note)));
    let before = times();

    fixture.delete_outside(cx, "Plan.md");
    keys(cx, handle, "ctrl-shift-d tab enter");
    autosave(cx);
    assert_eq!(editor_text(cx, handle), "# Goals\nread more\n");
    assert_eq!(files(&notes_dir), ["Goals.md", "Top.md"]);
    assert_eq!(times(), before);
}

#[gpui_kit::test]
fn a_quit_keeps_the_name_of_the_open_note(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    new_note(cx, handle, "Plans");
    cx.simulate_input(handle, "# Spring plans");

    cx.quit();
    assert_eq!(files(&fixture.notes()), ["Plans.md"]);
    assert!(read(&fixture.notes().join("Plans.md")).ends_with("\n# Spring plans\n"));
    let settings = Config::load(fixture.config()).settings().clone();
    assert_eq!(settings.last_note, holm_core::NotePath::new("Plans.md"));
}

#[gpui_kit::test]
fn a_window_close_keeps_the_name_of_the_open_note(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    new_note(cx, handle, "Plans");
    cx.simulate_input(handle, "# Spring plans");

    cx.update_window(handle, |_, window, _| window.remove_window()).unwrap();
    cx.run_until_parked();
    assert_eq!(files(&fixture.notes()), ["Plans.md"]);
    assert!(read(&fixture.notes().join("Plans.md")).ends_with("\n# Spring plans\n"));
    let settings = Config::load(fixture.config()).settings().clone();
    assert_eq!(settings.last_note, holm_core::NotePath::new("Plans.md"));
}

#[gpui_kit::test]
fn ctrl_shift_q_with_a_dialog_open_closes_the_window_by_the_close_rule(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    cx.simulate_input(handle, "a");
    keys(cx, handle, "ctrl-shift-l delete");
    assert!(question(cx, handle).is_some());
    keys(cx, handle, "ctrl-shift-q");
    assert!(!window_open(cx, handle));
    assert!(read(&note).ends_with("\naold\n"), "the close saves the text");
}

#[gpui_kit::test]
fn ctrl_shift_q_with_text_that_cannot_be_saved_needs_a_second_press(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("Note.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    read_only(&notes_dir, true);
    cx.simulate_input(handle, "a");
    keys(cx, handle, "ctrl-shift-q");
    assert!(window_open(cx, handle));
    assert_eq!(message(cx, handle).as_deref(), Some(text::CLOSE_AGAIN));

    keys(cx, handle, "ctrl-shift-q");
    assert!(!window_open(cx, handle));
    read_only(&notes_dir, false);
}

#[gpui_kit::test]
fn ctrl_shift_q_saves_the_typed_text_first(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    cx.simulate_input(handle, "a");
    keys(cx, handle, "ctrl-shift-q");
    assert!(!window_open(cx, handle));
    assert!(read(&note).ends_with("\naold\n"));
}
