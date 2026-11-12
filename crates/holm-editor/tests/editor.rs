//! The editor through its interface: keys, text input and clicks.

mod common;

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use common::{LINE_HEIGHT, MARGIN, advance, app, column_x, data_folder, open};
use gpui_kit::{
    Bounds, ClipboardItem, Entity, EntityInputHandler, Modifiers, MouseButton, MouseDownEvent,
    MouseUpEvent, Pixels, VisualTestContext, point, px,
};
use holm_editor::{Changed, Editor, UpFromStart, WritingHelp};

const WIDTH: Pixels = px(600.);
const HEIGHT: Pixels = px(400.);

fn text(editor: &Entity<Editor>, cx: &VisualTestContext) -> String {
    cx.read(|cx| editor.read(cx).text().to_owned())
}

/// Clicks at a char column and a pixel height, from the top left of the editor.
fn click(cx: &mut VisualTestContext, column: f32, y: Pixels) {
    let x = column_x(cx, column);
    cx.simulate_click(point(x, y), Modifiers::none());
}

/// Clicks `count` times in a row at a char column and a pixel height.
fn multi_click(cx: &mut VisualTestContext, column: f32, y: Pixels, count: usize) {
    let position = point(column_x(cx, column), y);
    for click_count in 1..=count {
        cx.simulate_event(MouseDownEvent {
            position,
            modifiers: Modifiers::none(),
            button: MouseButton::Left,
            click_count,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            position,
            modifiers: Modifiers::none(),
            button: MouseButton::Left,
            click_count,
        });
    }
}

fn clipboard(cx: &VisualTestContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

fn set_help(editor: &Entity<Editor>, cx: &mut VisualTestContext, help: WritingHelp) {
    cx.update(|_, cx| editor.update(cx, |editor, _| editor.set_writing_help(help)));
}

/// The x of the cursor in the window, as the input handler gives it to the system.
fn cursor_x(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Option<Pixels> {
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.bounds_for_range(0..0, Bounds::default(), window, cx)
        })
    })
    .map(|bounds| bounds.origin.x)
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
fn up_at_the_very_start_sends_up_from_start() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "one\ntwo", WIDTH, HEIGHT);
    let ups = Rc::new(Cell::new(0));
    let counter = ups.clone();
    cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, _: &UpFromStart, _| {
            counter.set(counter.get() + 1);
        })
        .detach();
    });
    cx.simulate_keystrokes("down right up shift-left up");
    assert_eq!(ups.get(), 0, "only with no selection at the start");
    cx.simulate_keystrokes("left up");
    assert_eq!(ups.get(), 1);
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
        advance(cx) * 20.5 + MARGIN * 2.
    };
    let (editor, cx) = open(&mut app, "aaaa bbbb cccc dddd eeee ffff", width, HEIGHT);
    // The second row starts at "eeee".
    click(cx, 0.2, LINE_HEIGHT * 1.5);
    cx.simulate_input("X");
    assert_eq!(text(&editor, cx), "aaaa bbbb cccc dddd Xeeee ffff");
}

#[test]
fn up_and_down_pass_a_wrap_inside_a_word() {
    let mut app = app();
    let width = {
        let (_, cx) = open(&mut app, "", WIDTH, HEIGHT);
        advance(cx) * 20.5 + MARGIN * 2.
    };
    let word = "a".repeat(30);
    let (editor, cx) = open(&mut app, &format!("{word}\nend"), width, HEIGHT);
    cx.simulate_keystrokes("down down");
    cx.simulate_input("X");
    assert_eq!(text(&editor, cx), format!("{word}\nXend"));
    cx.simulate_keystrokes("left up up");
    cx.simulate_input("Y");
    assert_eq!(text(&editor, cx), format!("Y{word}\nXend"));
    // Home on the second row stays on that row.
    cx.simulate_keystrokes("backspace down home");
    cx.simulate_input("Z");
    assert_eq!(
        text(&editor, cx),
        format!("{}Z{}\nXend", &word[..20], &word[20..])
    );
}

#[test]
fn up_and_down_keep_the_column_into_a_line_with_only_a_card() {
    let note = "abcdef\n![x](x.pdf)\nabcdef";
    for (keys, expected) in [
        ("right right right down", "abcdef\n![xX](x.pdf)\nabcdef"),
        (
            "down down right right right up",
            "abcdef\n![xX](x.pdf)\nabcdef",
        ),
    ] {
        let mut app = app();
        let (editor, cx) = open(&mut app, note, WIDTH, HEIGHT);
        cx.simulate_keystrokes(keys);
        cx.simulate_input("X");
        assert_eq!(text(&editor, cx), expected, "{keys}");
    }
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

#[test]
fn format_keys_add_and_remove_markdown() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "one two", WIDTH, HEIGHT);
    cx.simulate_keystrokes("ctrl-right right shift-ctrl-right ctrl-b");
    assert_eq!(text(&editor, cx), "one **two**");
    cx.simulate_keystrokes("ctrl-i");
    assert_eq!(text(&editor, cx), "one ***two***");
    cx.simulate_keystrokes("ctrl-b ctrl-i");
    assert_eq!(text(&editor, cx), "one two");
    cx.simulate_keystrokes("ctrl-u ctrl-d ctrl-h ctrl-e");
    assert_eq!(text(&editor, cx), "one <u>~~==`two`==~~</u>");
}

#[test]
fn a_format_key_with_no_selection_types_inside_new_markers() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "", WIDTH, HEIGHT);
    cx.simulate_keystrokes("ctrl-b");
    cx.simulate_input("bold");
    assert_eq!(text(&editor, cx), "**bold**");
}

#[test]
fn heading_keys_set_the_level() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "Plans", WIDTH, HEIGHT);
    cx.simulate_keystrokes("ctrl-2");
    assert_eq!(text(&editor, cx), "## Plans");
    cx.simulate_keystrokes("ctrl-0");
    assert_eq!(text(&editor, cx), "Plans");
}

#[test]
fn format_keys_can_be_turned_off() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "word", WIDTH, HEIGHT);
    set_help(
        &editor,
        cx,
        WritingHelp {
            format_keys: false,
            ..WritingHelp::default()
        },
    );
    cx.simulate_keystrokes("ctrl-a ctrl-b ctrl-1");
    assert_eq!(text(&editor, cx), "word");
}

#[test]
fn enter_continues_a_list_and_tab_indents_it() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "- one", WIDTH, HEIGHT);
    cx.simulate_keystrokes("end enter");
    cx.simulate_input("two");
    assert_eq!(text(&editor, cx), "- one\n- two");
    cx.simulate_keystrokes("tab");
    assert_eq!(text(&editor, cx), "- one\n  - two");
    cx.simulate_keystrokes("shift-tab");
    assert_eq!(text(&editor, cx), "- one\n- two");
    cx.simulate_keystrokes("enter enter");
    assert_eq!(text(&editor, cx), "- one\n- two\n\n");
}

#[test]
fn enter_twice_ends_the_list() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "- a", WIDTH, HEIGHT);
    cx.simulate_keystrokes("end enter enter");
    assert_eq!(text(&editor, cx), "- a\n\n");
    assert_eq!(cursor_x(&editor, cx), Some(MARGIN));
}

#[test]
fn the_cursor_on_an_empty_line_after_an_item_shows_where_text_goes() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "- a\n", WIDTH, HEIGHT);
    cx.simulate_keystrokes("end left");
    let item_x = cursor_x(&editor, cx);
    assert!(item_x > Some(MARGIN));
    cx.simulate_keystrokes("ctrl-end");
    assert_eq!(cursor_x(&editor, cx), item_x);
    cx.simulate_input("b");
    cx.simulate_keystrokes("left");
    assert_eq!(cursor_x(&editor, cx), item_x);
}

#[test]
fn the_cursor_on_an_empty_line_after_a_nested_item_shows_where_text_goes() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "- a\n  - b\n", WIDTH, HEIGHT);
    cx.simulate_keystrokes("down end left");
    let item_x = cursor_x(&editor, cx);
    assert!(item_x > Some(MARGIN));
    cx.simulate_keystrokes("ctrl-end");
    assert_eq!(cursor_x(&editor, cx), item_x);
    assert_eq!(text(&editor, cx), "- a\n  - b\n");
}

#[test]
fn a_click_on_the_empty_line_after_an_item_places_the_cursor_there() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "- a\n", WIDTH, HEIGHT);
    click(cx, 0., LINE_HEIGHT * 1.5);
    cx.simulate_input("b");
    assert_eq!(text(&editor, cx), "- a\nb");
}

#[test]
fn list_keys_can_be_turned_off() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "- one", WIDTH, HEIGHT);
    set_help(
        &editor,
        cx,
        WritingHelp {
            continue_lists: false,
            indent_lists: false,
            ..WritingHelp::default()
        },
    );
    cx.simulate_keystrokes("end enter tab");
    assert_eq!(text(&editor, cx), "- one\n    ");
}

#[test]
fn no_char_closes_itself() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "", WIDTH, HEIGHT);
    cx.simulate_input("*(\"'[");
    assert_eq!(text(&editor, cx), "*(\"'[");
}

#[test]
fn copy_and_cut_give_the_markdown_source() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "a **b** c\n\nnext", WIDTH, HEIGHT);
    cx.simulate_keystrokes("shift-end ctrl-c");
    assert_eq!(clipboard(cx).as_deref(), Some("a **b** c"));
    cx.simulate_keystrokes("ctrl-x");
    assert_eq!(text(&editor, cx), "\n\nnext");
    assert_eq!(clipboard(cx).as_deref(), Some("a **b** c"));
}

#[test]
fn paste_puts_in_plain_text_as_it_is() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "ab", WIDTH, HEIGHT);
    cx.write_to_clipboard(ClipboardItem::new_string("**x**\r\n- y".into()));
    cx.simulate_keystrokes("right ctrl-v");
    assert_eq!(text(&editor, cx), "a**x**\n- yb");
    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&editor, cx), "ab");
}

#[test]
fn undo_and_redo_go_by_words_and_pauses() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "", WIDTH, HEIGHT);
    cx.simulate_input("one two");
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.simulate_input("three");
    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&editor, cx), "one two");
    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&editor, cx), "one ");
    cx.simulate_keystrokes("ctrl-y ctrl-y");
    assert_eq!(text(&editor, cx), "one twothree");
}

#[test]
fn a_reload_is_one_undo_step() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "old", WIDTH, HEIGHT);
    let changes = count_changes(&editor, cx);
    cx.update(|_, cx| editor.update(cx, |editor, cx| editor.reload_text("new", cx)));
    assert_eq!(text(&editor, cx), "new");
    assert_eq!(changes.get(), 0);
    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&editor, cx), "old");
    assert_eq!(changes.get(), 1);
}

#[test]
fn home_end_and_word_keys_move_and_select() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "one two\nthree", WIDTH, HEIGHT);
    cx.simulate_keystrokes("end");
    cx.simulate_input("X");
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input("Y");
    cx.simulate_keystrokes("ctrl-home ctrl-shift-right backspace");
    assert_eq!(text(&editor, cx), " twoX\nthreeY");
    cx.simulate_keystrokes("ctrl-delete");
    assert_eq!(text(&editor, cx), "\nthreeY");
}

#[test]
fn page_down_moves_by_one_view() {
    let mut app = app();
    let note = (0..100).fold(String::new(), |note, n| note + &format!("line {n}\n"));
    let (editor, cx) = open(&mut app, &note, WIDTH, LINE_HEIGHT * 10.);
    cx.simulate_keystrokes("pagedown");
    cx.simulate_input("X");
    assert!(text(&editor, cx).contains("\nXline 10\n"));
}

#[test]
fn a_double_click_selects_a_word_and_a_triple_click_a_line() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "one two\nnext", WIDTH, HEIGHT);
    multi_click(cx, 5.2, LINE_HEIGHT * 0.5, 2);
    cx.simulate_input("X");
    assert_eq!(text(&editor, cx), "one X\nnext");
    multi_click(cx, 1.2, LINE_HEIGHT * 0.5, 3);
    cx.simulate_input("Y");
    assert_eq!(text(&editor, cx), "Ynext");
}

#[test]
fn a_drag_selects_text() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "one two", WIDTH, HEIGHT);
    let x = advance(cx);
    let y = LINE_HEIGHT * 0.5;
    cx.simulate_mouse_down(
        point(MARGIN + x * 1.2, y),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.simulate_mouse_move(
        point(MARGIN + x * 5.2, y),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.simulate_mouse_up(
        point(MARGIN + x * 5.2, y),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.simulate_input("X");
    assert_eq!(text(&editor, cx), "oXwo");
}

#[test]
fn a_click_selects_nothing_when_its_line_shows_the_syntax() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "start\n\nsome **bold** text", WIDTH, HEIGHT);
    // The click shows the "**", so the same point now lands on another byte.
    let position = point(column_x(cx, 6.2), LINE_HEIGHT * 2.5);
    cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(position, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::none());
    cx.simulate_input("X");
    assert_eq!(text(&editor, cx), "start\n\nsome **bXold** text");
}

#[test]
fn a_drag_starts_only_a_few_pixels_from_the_click() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "one two", WIDTH, HEIGHT);
    let x = advance(cx);
    let y = LINE_HEIGHT * 0.5;
    cx.simulate_mouse_down(
        point(MARGIN + x * 1.2, y),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.simulate_mouse_move(
        point(MARGIN + x * 1.2 + px(2.), y),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.simulate_mouse_move(
        point(MARGIN + x * 5.2, y),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.simulate_mouse_up(
        point(MARGIN + x * 5.2, y),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.simulate_input("X");
    assert_eq!(text(&editor, cx), "oXwo");
}
