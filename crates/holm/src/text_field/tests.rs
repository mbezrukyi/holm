use gpui_kit::{TestAppContext, VisualTestContext};

use super::*;

fn field<'a>(cx: &'a mut TestAppContext, text: &str) -> (Entity<TextField>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::shortcuts::bind(cx);
    });
    let (field, cx) = cx.add_window_view(|window, cx| TextField::new("f", "F", text, window, cx));
    cx.update(|window, cx| window.focus(&field.focus_handle(cx), cx));
    (field, cx)
}

#[gpui_kit::test]
fn typing_replaces_the_selection_and_keys_edit_the_text(cx: &mut TestAppContext) {
    let (field, cx) = field(cx, "Plans");
    cx.simulate_input("Goals");
    cx.simulate_keystrokes("left left backspace");
    cx.simulate_input("ö");
    cx.simulate_keystrokes("shift-home delete");
    cx.simulate_input("x");
    cx.simulate_keystrokes("end shift-left");
    cx.simulate_input("S");
    field.read_with(cx, |field, _| assert_eq!(field.text(), "xlS"));
}

#[gpui_kit::test]
fn ctrl_a_and_cut_and_paste_work(cx: &mut TestAppContext) {
    let (field, cx) = field(cx, "a b");
    cx.simulate_keystrokes("end ctrl-a ctrl-x");
    field.read_with(cx, |field, _| assert_eq!(field.text(), ""));
    cx.write_to_clipboard(ClipboardItem::new_string("one\ntwo".to_owned()));
    cx.simulate_keystrokes("ctrl-v");
    field.read_with(cx, |field, _| assert_eq!(field.text(), "one two"));
}
