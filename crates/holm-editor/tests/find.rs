//! Find in the open note, through the editor interface.

mod common;

use common::{LINE_HEIGHT, app, cursor_y, open};
use gpui_kit::{ClipboardItem, Entity, Pixels, VisualTestContext, px};
use holm_editor::Editor;

const WIDTH: Pixels = px(600.);
const HEIGHT: Pixels = px(400.);

fn text(editor: &Entity<Editor>, cx: &VisualTestContext) -> String {
    cx.read(|cx| editor.read(cx).text().to_owned())
}

fn selected(editor: &Entity<Editor>, cx: &VisualTestContext) -> String {
    cx.read(|cx| editor.read(cx).selected_text().to_owned())
}

/// A note of 100 lines, `item 000` to `item 099`.
fn items() -> String {
    (0..100)
        .map(|n| format!("item {n:03}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The view of the scroll tests: ten lines high.
const VIEW: Pixels = px(200.);

fn assert_middle(y: Pixels) {
    assert!((y - VIEW / 2.).abs() < px(0.5), "{y:?} is not the middle");
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

#[test]
fn a_match_below_the_view_goes_to_the_middle() {
    let mut app = app();
    let (editor, cx) = open(&mut app, &items(), WIDTH, VIEW);
    cx.simulate_keystrokes("ctrl-f");
    cx.simulate_input("item 060");
    assert_eq!(selected(&editor, cx), "item 060");
    assert_middle(cursor_y(&editor, cx));
}

#[test]
fn a_match_above_the_view_goes_to_the_middle() {
    let mut app = app();
    let (editor, cx) = open(&mut app, &items(), WIDTH, VIEW);
    cx.simulate_keystrokes("ctrl-end ctrl-f");
    cx.simulate_input("item 040");
    assert_eq!(selected(&editor, cx), "item 040");
    assert_middle(cursor_y(&editor, cx));
}

#[test]
fn a_match_in_view_moves_nothing() {
    let mut app = app();
    let (editor, cx) = open(&mut app, &items(), WIDTH, VIEW);
    cx.simulate_keystrokes("down");
    let line_1 = cursor_y(&editor, cx);
    cx.simulate_keystrokes("ctrl-home ctrl-f");
    cx.simulate_input("item 008");
    assert_eq!(cursor_y(&editor, cx), line_1 + LINE_HEIGHT * 7.);
}

#[test]
fn a_match_in_the_last_lines_goes_to_the_middle_with_space_below() {
    let mut app = app();
    let (editor, cx) = open(&mut app, &items(), WIDTH, VIEW);
    cx.simulate_keystrokes("ctrl-f");
    cx.simulate_input("item 099");
    assert_middle(cursor_y(&editor, cx));
}

#[test]
fn a_match_near_the_start_leaves_the_first_line_at_the_top() {
    let mut app = app();
    let (editor, cx) = open(&mut app, &items(), WIDTH, VIEW);
    cx.simulate_keystrokes("down");
    let line_1 = cursor_y(&editor, cx);
    cx.simulate_keystrokes("ctrl-end ctrl-f");
    cx.simulate_input("item 001");
    assert_eq!(cursor_y(&editor, cx), line_1);
}

#[test]
fn the_row_with_the_match_goes_to_the_middle_of_a_wrapped_paragraph() {
    let mut app = app();
    let words = (0..300).map(|n| format!("w{n:03}")).collect::<Vec<_>>();
    let (editor, cx) = open(&mut app, &words.join(" "), WIDTH, VIEW);
    cx.simulate_keystrokes("ctrl-f");
    // A paste puts the whole query in at once. Typed, "w2" would scroll to "w200" first.
    cx.write_to_clipboard(ClipboardItem::new_string("w250".to_owned()));
    cx.simulate_keystrokes("ctrl-v");
    assert_eq!(selected(&editor, cx), "w250");
    assert_middle(cursor_y(&editor, cx));
}

#[test]
fn enter_and_shift_enter_put_each_match_in_the_middle_and_escape_keeps_the_view() {
    let mut app = app();
    let mut lines = (0..100).map(|n| format!("line {n}")).collect::<Vec<_>>();
    for n in [3, 50, 90] {
        lines[n] = format!("mark {n}");
    }
    let (editor, cx) = open(&mut app, &lines.join("\n"), WIDTH, VIEW);
    cx.simulate_keystrokes("down");
    let line_1 = cursor_y(&editor, cx);
    cx.simulate_keystrokes("ctrl-home ctrl-f");
    cx.simulate_input("mark");
    assert_eq!(cursor_y(&editor, cx), line_1 + LINE_HEIGHT * 2.);
    cx.simulate_keystrokes("enter");
    assert_middle(cursor_y(&editor, cx));
    cx.simulate_keystrokes("enter");
    assert_middle(cursor_y(&editor, cx));
    cx.simulate_keystrokes("shift-enter");
    assert_middle(cursor_y(&editor, cx));
    cx.simulate_keystrokes("escape");
    assert_middle(cursor_y(&editor, cx));
    cx.simulate_input("X");
    assert!(text(&editor, cx).contains("\nX 50\n"));
}

#[test]
fn arrows_after_find_scroll_to_the_edge_as_before() {
    let mut app = app();
    let (editor, cx) = open(&mut app, &items(), WIDTH, VIEW);
    cx.simulate_keystrokes("ctrl-f");
    cx.simulate_input("item 050");
    cx.simulate_keystrokes("escape");
    let middle = cursor_y(&editor, cx);
    // Four rows down is the last row in view whole. The next rows go to the bottom edge.
    cx.simulate_keystrokes("down down down down");
    assert_eq!(cursor_y(&editor, cx), middle + LINE_HEIGHT * 4.);
    cx.simulate_keystrokes("down");
    assert_eq!(cursor_y(&editor, cx), VIEW - LINE_HEIGHT / 2.);
    cx.simulate_keystrokes("down");
    assert_eq!(cursor_y(&editor, cx), VIEW - LINE_HEIGHT / 2.);
}
