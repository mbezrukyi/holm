//! Find in the open note, through the editor interface.

mod common;

use common::{app, open};
use gpui_kit::{Entity, Pixels, VisualTestContext, px};
use holm_editor::Editor;

const WIDTH: Pixels = px(600.);
const HEIGHT: Pixels = px(400.);

fn text(editor: &Entity<Editor>, cx: &VisualTestContext) -> String {
    cx.read(|cx| editor.read(cx).text().to_owned())
}

fn selected(editor: &Entity<Editor>, cx: &VisualTestContext) -> String {
    cx.read(|cx| editor.read(cx).selected_text().to_owned())
}

/// The text with `[` and `]` around the selection. It types them in, then undoes it.
fn marked(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
    let part = selected(editor, cx);
    cx.simulate_input(&format!("[{part}]"));
    let with = text(editor, cx);
    cx.simulate_keystrokes("ctrl-z");
    with
}

#[test]
fn typing_in_find_selects_the_first_match_and_leaves_the_note() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "One two\n\none TWO one", WIDTH, HEIGHT);
    cx.simulate_keystrokes("ctrl-f");
    cx.simulate_input("tw");
    assert_eq!(text(&editor, cx), "One two\n\none TWO one");
    assert_eq!(selected(&editor, cx), "tw");
    cx.simulate_input("o");
    assert_eq!(selected(&editor, cx), "two");
    cx.simulate_keystrokes("backspace backspace backspace");
    cx.simulate_input("one");
    assert_eq!(selected(&editor, cx), "One");
}

#[test]
fn enter_and_shift_enter_move_between_matches() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "a1 a2 a3", WIDTH, HEIGHT);
    cx.simulate_keystrokes("ctrl-f");
    cx.simulate_input("a");
    cx.simulate_keystrokes("enter");
    cx.simulate_keystrokes("escape");
    assert_eq!(marked(&editor, cx), "a1 [a]2 a3");
    cx.simulate_keystrokes("ctrl-f enter enter escape");
    // The selection fills the query, and Enter goes round after the last match.
    assert_eq!(marked(&editor, cx), "[a]1 a2 a3");
    cx.simulate_keystrokes("ctrl-f shift-enter escape");
    assert_eq!(marked(&editor, cx), "a1 a2 [a]3");
}

#[test]
fn escape_closes_find_and_typing_edits_the_note_again() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "find me", WIDTH, HEIGHT);
    cx.simulate_keystrokes("ctrl-f");
    cx.simulate_input("me");
    cx.simulate_keystrokes("escape");
    cx.simulate_input("you");
    assert_eq!(text(&editor, cx), "find you");
}

#[test]
fn a_query_with_no_match_keeps_the_selection() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "abc", WIDTH, HEIGHT);
    cx.simulate_keystrokes("right ctrl-f");
    cx.simulate_input("zz");
    cx.simulate_keystrokes("enter escape");
    cx.simulate_input("X");
    assert_eq!(text(&editor, cx), "aXbc");
}

#[test]
fn a_format_key_in_find_changes_nothing() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "one two", WIDTH, HEIGHT);
    cx.simulate_keystrokes("ctrl-f");
    cx.simulate_input("two");
    cx.simulate_keystrokes("ctrl-b escape");
    assert_eq!(text(&editor, cx), "one two");
}

#[test]
fn ctrl_backspace_in_find_clears_the_query() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "one two", WIDTH, HEIGHT);
    cx.simulate_keystrokes("ctrl-f");
    cx.simulate_input("two");
    cx.simulate_keystrokes("ctrl-backspace");
    cx.simulate_input("one");
    assert_eq!(selected(&editor, cx), "one");
}
