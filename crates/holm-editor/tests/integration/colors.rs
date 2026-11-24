use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{ClipboardItem, Entity, Pixels, VisualTestContext, px};
use holm_editor::testing::{app, open};
use holm_editor::{CustomColors, Editor, WritingHelp};
use holm_markdown::Color;

const WIDTH: Pixels = px(600.);
const HEIGHT: Pixels = px(400.);

fn text(editor: &Entity<Editor>, cx: &VisualTestContext) -> String {
    cx.read(|cx| editor.read(cx).text().to_owned())
}

fn color(value: &str) -> Color {
    Color::from_hex(value).unwrap_or_else(|| panic!("{value} is a color"))
}

/// Collects the custom colors that the editor sends.
fn sent(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Rc<RefCell<Vec<Vec<Color>>>> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(editor, move |_, event: &CustomColors, _| {
            sink.borrow_mut().push(event.colors.clone());
        })
        .detach();
    });
    events
}

/// An editor with "a word" and "word" selected.
fn word(app: &mut gpui_kit::TestAppContext) -> (Entity<Editor>, &mut VisualTestContext) {
    let (editor, cx) = open(app, "a word", WIDTH, HEIGHT);
    cx.simulate_keystrokes("end shift-ctrl-left");
    (editor, cx)
}

fn with_custom(editor: &Entity<Editor>, cx: &mut VisualTestContext, colors: &[Color]) {
    cx.update(|_, cx| editor.update(cx, |editor, cx| editor.set_custom_colors(colors, cx)));
}

#[test]
fn keys_one_to_six_set_a_palette_text_color() {
    for (key, hex) in [("1", "#b5442c"), ("6", "#8250a0")] {
        let mut app = app();
        let (editor, cx) = word(&mut app);
        cx.simulate_keystrokes(&format!("ctrl-t {key}"));
        assert_eq!(text(&editor, cx), format!("a <span style=\"color: {hex}\">word</span>"));
    }
}

#[test]
fn ctrl_m_sets_a_palette_highlight_color() {
    let mut app = app();
    let (editor, cx) = word(&mut app);
    cx.simulate_keystrokes("ctrl-m 5");
    assert_eq!(text(&editor, cx), "a <mark style=\"background: #c6dbf2\">word</mark>");
}

#[test]
fn a_key_closes_the_window_so_typing_edits_the_text_again() {
    let mut app = app();
    let (editor, cx) = word(&mut app);
    cx.simulate_keystrokes("ctrl-t 2 end");
    cx.simulate_input("!");
    assert_eq!(text(&editor, cx), "a <span style=\"color: #a85d12\">word</span>!");
}

#[test]
fn key_zero_removes_the_color() {
    let mut app = app();
    let (editor, cx) = open(
        &mut app,
        "a <mark style=\"background: #fde68a\">word</mark> ==b==",
        WIDTH,
        HEIGHT,
    );
    cx.simulate_keystrokes("ctrl-a ctrl-m 0");
    assert_eq!(text(&editor, cx), "a word b");
}

#[test]
fn keys_seven_to_nine_take_the_custom_colors() {
    let mut app = app();
    let (editor, cx) = word(&mut app);
    let custom = [color("#111111"), color("#222222"), color("#333333")];
    with_custom(&editor, cx, &custom);
    let sent = sent(&editor, cx);
    cx.simulate_keystrokes("ctrl-t 8");
    assert_eq!(text(&editor, cx), "a <span style=\"color: #222222\">word</span>");
    assert_eq!(*sent.borrow(), [vec![custom[1], custom[0], custom[2]]]);
}

#[test]
fn a_free_custom_key_does_nothing_and_escape_closes() {
    let mut app = app();
    let (editor, cx) = word(&mut app);
    with_custom(&editor, cx, &[color("#111111")]);
    cx.simulate_keystrokes("ctrl-t 9");
    assert_eq!(text(&editor, cx), "a word");
    cx.simulate_keystrokes("escape");
    cx.simulate_input("X");
    assert_eq!(text(&editor, cx), "a X");
}

#[test]
fn plus_opens_the_hex_field_and_enter_takes_the_value() {
    let mut app = app();
    let (editor, cx) = word(&mut app);
    let sent = sent(&editor, cx);
    cx.simulate_keystrokes("ctrl-t +");
    cx.simulate_input("#12Ab3");
    cx.simulate_keystrokes("backspace");
    cx.simulate_input("C4");
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "a <span style=\"color: #12abc4\">word</span>");
    assert_eq!(*sent.borrow(), [vec![color("#12abc4")]]);
}

#[test]
fn a_digit_key_in_the_hex_field_types_the_digit() {
    let mut app = app();
    let (editor, cx) = word(&mut app);
    cx.simulate_keystrokes("ctrl-t + 1 2 3 enter");
    assert_eq!(text(&editor, cx), "a <span style=\"color: #112233\">word</span>");
}

#[test]
fn the_hex_field_takes_three_digits_and_a_paste() {
    let mut app = app();
    let (editor, cx) = word(&mut app);
    cx.write_to_clipboard(ClipboardItem::new_string("#0a0".to_owned()));
    cx.simulate_keystrokes("ctrl-m + ctrl-v enter");
    assert_eq!(text(&editor, cx), "a <mark style=\"background: #00aa00\">word</mark>");
}

#[test]
fn enter_on_a_short_hex_value_keeps_the_field() {
    let mut app = app();
    let (editor, cx) = word(&mut app);
    cx.simulate_keystrokes("ctrl-t +");
    cx.simulate_input("12");
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "a word");
    cx.simulate_input("3");
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "a <span style=\"color: #112233\">word</span>");
}

#[test]
fn a_palette_color_in_the_hex_field_is_not_a_custom_color() {
    let mut app = app();
    let (editor, cx) = word(&mut app);
    let sent = sent(&editor, cx);
    cx.simulate_keystrokes("ctrl-t +");
    cx.simulate_input("b5442c");
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "a <span style=\"color: #b5442c\">word</span>");
    assert!(sent.borrow().is_empty());
}

#[test]
fn escape_in_the_hex_field_closes_with_no_change() {
    let mut app = app();
    let (editor, cx) = word(&mut app);
    cx.simulate_keystrokes("ctrl-t +");
    cx.simulate_input("123456");
    cx.simulate_keystrokes("escape");
    assert_eq!(text(&editor, cx), "a word");
}

#[test]
fn the_window_is_off_with_the_format_keys() {
    let mut app = app();
    let (editor, cx) = word(&mut app);
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            editor.set_writing_help(WritingHelp {
                format_keys: false,
                ..WritingHelp::default()
            });
        });
    });
    cx.simulate_keystrokes("ctrl-t 1");
    assert_eq!(text(&editor, cx), "a 1");
}
