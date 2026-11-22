//! One key press in a 1 MB note, in release mode. It catches only big slowdowns.
//!
//! CI runs it with `cargo test --release -p holm-editor --test speed`.

mod common;

use std::time::{Duration, Instant};

use common::{app, one_mb_note, open};
use gpui_kit::px;

#[test]
#[cfg_attr(debug_assertions, ignore = "speed counts only in release mode")]
fn a_key_press_in_a_1_mb_note_takes_under_50_ms() {
    let mut app = app();
    let note = one_mb_note();
    let (editor, cx) = open(&mut app, &note, px(800.), px(600.));
    cx.simulate_keystrokes("down down right");

    let start = Instant::now();
    cx.simulate_input("a");
    let time = start.elapsed();

    assert_eq!(cx.read(|cx| editor.read(cx).text().len()), note.len() + 1);
    assert!(time < Duration::from_millis(50), "a key press took {time:?}");
}
