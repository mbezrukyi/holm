//! The editor through its interface: keys, text input and clicks.

mod common;

use std::cell::Cell;
use std::rc::Rc;

use common::{LINE_HEIGHT, advance, app, data_folder, open};
use gpui_kit::{Entity, Modifiers, Pixels, VisualTestContext, point, px};
use holm_editor::{Changed, Editor};

const WIDTH: Pixels = px(600.);
const HEIGHT: Pixels = px(400.);

fn text(editor: &Entity<Editor>, cx: &VisualTestContext) -> String {
    cx.read(|cx| editor.read(cx).text().to_owned())
}

/// Clicks at a char column and a pixel height, from the top left of the editor.
fn click(cx: &mut VisualTestContext, column: f32, y: Pixels) {
    let x = advance(cx) * column;
    cx.simulate_click(point(x, y), Modifiers::none());
}

fn count_changes(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Rc<Cell<usize>> {
    let count = Rc::new(Cell::new(0));
    let counter = count.clone();
    cx.update(|_, cx| {
        cx.subscribe(editor, move |_, _: &Changed, _| {
            counter.set(counter.get() + 1);
        })
        .detach();
    });
    count
}

#[test]
fn typing_and_deleting_change_the_text() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "", WIDTH, HEIGHT);
    let changes = count_changes(&editor, cx);

    cx.simulate_input("hello");
    assert!(changes.get() > 0);
    let typed = changes.get();
    cx.simulate_keystrokes("left left backspace");
    assert_eq!(text(&editor, cx), "helo");
    cx.simulate_keystrokes("delete enter");
    assert_eq!(text(&editor, cx), "he\no");
    cx.simulate_keystrokes("right right delete");
    assert_eq!(text(&editor, cx), "he\no");
    // The last delete at the end of the text changes nothing.
    assert_eq!(changes.get(), typed + 3);
}

#[test]
fn set_text_sends_no_change() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "one", WIDTH, HEIGHT);
    let changes = count_changes(&editor, cx);
    cx.update(|_, cx| editor.update(cx, |editor, cx| editor.set_text("two", cx)));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "two");
    assert_eq!(changes.get(), 0);
}

#[test]
fn a_selection_is_replaced_by_typing() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "abcdef", WIDTH, HEIGHT);
    cx.simulate_keystrokes("right shift-right shift-right");
    cx.simulate_input("X");
    assert_eq!(text(&editor, cx), "aXdef");
}

#[test]
fn a_click_lands_on_the_right_byte() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "start\n\nsome text", WIDTH, HEIGHT);
    click(cx, 6.2, LINE_HEIGHT * 2.5);
    cx.simulate_input("X");
    assert_eq!(text(&editor, cx), "start\n\nsome tXext");
    click(cx, 0.2, LINE_HEIGHT * 0.5);
    cx.simulate_input("Y");
    assert_eq!(text(&editor, cx), "Ystart\n\nsome tXext");
}

#[test]
fn syntax_shows_only_on_the_cursor_line() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "start\n\nsome **bold** text", WIDTH, HEIGHT);
    // Away from the cursor the line shows "some bold text", so column 6 is the "o".
    click(cx, 6.2, LINE_HEIGHT * 2.5);
    cx.simulate_input("X");
    assert_eq!(text(&editor, cx), "start\n\nsome **bXold** text");
    // Now the line has the cursor and shows its source, so column 6 is in the "**".
    click(cx, 6.2, LINE_HEIGHT * 2.5);
    cx.simulate_input("Y");
    assert_eq!(text(&editor, cx), "start\n\nsome *Y*bXold** text");
}

#[test]
fn long_lines_wrap_at_the_editor_width() {
    let mut app = app();
    let width = {
        let (_, cx) = open(&mut app, "", WIDTH, HEIGHT);
        advance(cx) * 20.5
    };
    let (editor, cx) = open(&mut app, "aaaa bbbb cccc dddd eeee ffff", width, HEIGHT);
    // The second row starts at "eeee".
    click(cx, 0.2, LINE_HEIGHT * 1.5);
    cx.simulate_input("X");
    assert_eq!(text(&editor, cx), "aaaa bbbb cccc dddd Xeeee ffff");
}

#[test]
fn up_and_down_keep_the_column() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "abcdef\nxy\nabcdef", WIDTH, HEIGHT);
    cx.simulate_keystrokes("right right right right down down");
    cx.simulate_input("X");
    assert_eq!(text(&editor, cx), "abcdef\nxy\nabcdXef");
    cx.simulate_keystrokes("up");
    cx.simulate_input("Y");
    assert_eq!(text(&editor, cx), "abcdef\nxyY\nabcdXef");
}

/// The test image is 64 by 40 pixels. Images have 4 px of space above and below.
#[test]
fn an_image_shows_as_a_block_below_its_line() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "first\n![cat](cat.png)\nafter", WIDTH, HEIGHT);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| editor.set_folder(Some(data_folder()), cx));
    });
    cx.run_until_parked();

    // The image line shows no text, so the image starts right below "first".
    let after = LINE_HEIGHT + px(4. + 40. + 4.);
    click(cx, 0.2, after + LINE_HEIGHT * 0.5);
    cx.simulate_input("X");
    assert_eq!(text(&editor, cx), "first\n![cat](cat.png)\nXafter");

    // A click on the image puts the cursor on its line. Without the image, "after" would be here.
    click(cx, 1., LINE_HEIGHT * 2.5);
    cx.simulate_input("Y");
    assert_eq!(text(&editor, cx), "first\n![cat](cat.png)Y\nXafter");
}

#[test]
fn the_cursor_moves_onto_and_past_an_image_line() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "first\n![cat](cat.png)\nafter", WIDTH, HEIGHT);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| editor.set_folder(Some(data_folder()), cx));
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("down");
    cx.simulate_input("Y");
    assert_eq!(text(&editor, cx), "first\nY![cat](cat.png)\nafter");
    // The column stays 1.
    cx.simulate_keystrokes("down");
    cx.simulate_input("Z");
    assert_eq!(text(&editor, cx), "first\nY![cat](cat.png)\naZfter");
}

#[test]
fn the_view_follows_the_cursor() {
    let mut app = app();
    let note = (0..100).fold(String::new(), |note, n| note + &format!("line {n}\n"));
    let (editor, cx) = open(&mut app, &note, WIDTH, LINE_HEIGHT * 10.);
    for _ in 0..50 {
        cx.simulate_keystrokes("down");
    }
    // Line 50 is now the last row in view.
    click(cx, 0.2, LINE_HEIGHT * 9.5);
    cx.simulate_input("X");
    assert!(text(&editor, cx).contains("\nXline 50\n"));
}
