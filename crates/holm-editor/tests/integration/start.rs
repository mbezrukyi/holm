use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{
    AppContext as _, Entity, FocusHandle, Focusable as _, Modifiers, Pixels, ScrollDelta, ScrollEvent,
    VisualTestContext, point, px,
};
use holm_editor::Editor;
use holm_editor::testing::{LINE_HEIGHT, app, open};

use crate::support::{at, text};

const WIDTH: Pixels = px(600.);
const HEIGHT: Pixels = px(200.);
/// The height of the start view: two rows.
const START: Pixels = px(40.);

/// A view of [`START`] height that counts its clicks and its keys.
struct Start {
    probe: Probe,
}

/// What the start view saw, and its focus.
#[derive(Clone)]
struct Probe {
    clicks: Rc<RefCell<usize>>,
    keys: Rc<RefCell<usize>>,
    focus: FocusHandle,
}

impl gpui_kit::Render for Start {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        _cx: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::{InteractiveElement as _, MouseButton, Styled as _};
        let clicks = self.probe.clicks.clone();
        let keys = self.probe.keys.clone();
        gpui_kit::div()
            .id("start")
            .track_focus(&self.probe.focus)
            .w_full()
            .h(START)
            .on_mouse_down(MouseButton::Left, move |_, _, _| {
                *clicks.borrow_mut() += 1;
            })
            .on_key_down(move |_, _, _| *keys.borrow_mut() += 1)
    }
}

fn open_with_start<'a>(
    app: &'a mut gpui_kit::TestAppContext,
    text: &str,
) -> (Entity<Editor>, &'a mut VisualTestContext, Probe) {
    let (editor, cx) = open(app, text, WIDTH, HEIGHT);
    let probe = cx.update(|_, cx| {
        let probe = Probe {
            clicks: Rc::default(),
            keys: Rc::default(),
            focus: cx.focus_handle(),
        };
        let start = cx.new(|_| Start { probe: probe.clone() });
        editor.update(cx, |editor, cx| editor.set_start(Some(start.into()), cx));
        probe
    });
    cx.run_until_parked();
    (editor, cx, probe)
}

fn focus(cx: &mut VisualTestContext, handle: &FocusHandle) {
    cx.update(|window, cx| window.focus(handle, cx));
    cx.run_until_parked();
}

/// Up when `by` is below zero.
fn scroll(cx: &mut VisualTestContext, by: Pixels) {
    cx.simulate_event(ScrollEvent {
        position: point(px(100.), px(100.)),
        delta: ScrollDelta::Pixels(point(px(0.), -by)),
        ..Default::default()
    });
    cx.run_until_parked();
}

fn long_text() -> String {
    (0..30).map(|line| format!("l{line:02}")).collect::<Vec<_>>().join("\n")
}

#[test]
fn the_start_view_shows_before_the_text_and_takes_its_clicks() {
    let mut app = app();
    let (editor, cx, probe) = open_with_start(&mut app, "one\ntwo");

    // A click in the start view gives it the focus. Its keys do not reach the text.
    cx.simulate_click(at(cx, 1., START / 2.), Modifiers::none());
    assert_eq!(*probe.clicks.borrow(), 1);
    cx.simulate_keystrokes("x delete");
    assert_eq!(*probe.keys.borrow(), 2);
    assert_eq!(text(&editor, cx), "one\ntwo");

    // The first line lies right under the start view.
    cx.simulate_click(at(cx, 2., START + LINE_HEIGHT / 2.), Modifiers::none());
    cx.simulate_input("Y");
    assert_eq!(text(&editor, cx), "onYe\ntwo");
    cx.simulate_click(at(cx, 1., START + LINE_HEIGHT * 1.5), Modifiers::none());
    cx.simulate_input("Z");
    assert_eq!(text(&editor, cx), "onYe\ntZwo");
    assert_eq!(*probe.clicks.borrow(), 1);
}

#[test]
fn the_scroll_counts_the_start_view() {
    let mut app = app();
    let (editor, cx, probe) = open_with_start(&mut app, &long_text());

    // Past the start view and one line, the second line is at the top.
    scroll(cx, START + LINE_HEIGHT);
    cx.simulate_click(at(cx, 0., LINE_HEIGHT / 2.), Modifiers::none());
    cx.simulate_input("A");
    assert!(text(&editor, cx).contains("\nAl01\n"));
    assert_eq!(*probe.clicks.borrow(), 0);

    // Half of the start view is in sight. A click on it brings all of it in sight.
    scroll(cx, -(LINE_HEIGHT + START / 2.));
    cx.simulate_click(at(cx, 0., START / 4.), Modifiers::none());
    assert_eq!(*probe.clicks.borrow(), 1);
    cx.simulate_click(at(cx, 0., START - px(1.)), Modifiers::none());
    assert_eq!(*probe.clicks.borrow(), 2);
    cx.simulate_click(at(cx, 0., START + LINE_HEIGHT / 2.), Modifiers::none());
    cx.simulate_input("B");
    assert!(text(&editor, cx).starts_with("Bl00\n"));

    // The scroll stops at the very top.
    scroll(cx, START * 2.);
    scroll(cx, -START * 4.);
    cx.simulate_click(at(cx, 0., START - px(1.)), Modifiers::none());
    assert_eq!(*probe.clicks.borrow(), 3);
}

#[test]
fn up_and_page_up_bring_the_start_view_back() {
    let mut app = app();
    let (editor, cx, probe) = open_with_start(&mut app, &long_text());

    cx.simulate_keystrokes("ctrl-end");
    cx.run_until_parked();
    // The text of the last line is at the bottom.
    cx.simulate_click(at(cx, 0., HEIGHT - LINE_HEIGHT / 2.), Modifiers::none());
    cx.simulate_input("A");
    assert!(text(&editor, cx).ends_with("\nAl29"));

    for _ in 0..4 {
        cx.simulate_keystrokes("pageup");
        cx.run_until_parked();
    }
    cx.simulate_input("B");
    assert!(text(&editor, cx).starts_with("Bl00\n"), "{}", &text(&editor, cx)[..20]);
    cx.simulate_click(at(cx, 0., START / 2.), Modifiers::none());
    assert_eq!(*probe.clicks.borrow(), 1);
    let editor_focus = cx.read(|cx| editor.focus_handle(cx));
    focus(cx, &editor_focus);

    // Up from the second line, after a scroll that hides the start view and cuts off the first line.
    cx.simulate_keystrokes("down");
    scroll(cx, START + LINE_HEIGHT / 2.);
    cx.simulate_keystrokes("up");
    cx.run_until_parked();
    cx.simulate_input("C");
    assert!(text(&editor, cx).starts_with("BCl00\n"));
    cx.simulate_click(at(cx, 0., START / 2.), Modifiers::none());
    assert_eq!(*probe.clicks.borrow(), 2);
}

#[test]
fn the_focus_in_the_start_view_scrolls_to_it_and_back_to_the_cursor() {
    let mut app = app();
    let (editor, cx, probe) = open_with_start(&mut app, &long_text());
    cx.simulate_keystrokes("ctrl-end");
    cx.run_until_parked();

    focus(cx, &probe.focus);
    cx.simulate_click(at(cx, 0., START / 2.), Modifiers::none());
    assert_eq!(*probe.clicks.borrow(), 1);

    let editor_focus = cx.read(|cx| editor.focus_handle(cx));
    focus(cx, &editor_focus);
    cx.simulate_click(at(cx, 3., HEIGHT - LINE_HEIGHT / 2.), Modifiers::none());
    cx.simulate_input("A");
    assert!(text(&editor, cx).ends_with("\nl29A"));
    assert_eq!(*probe.clicks.borrow(), 1);
}

#[test]
fn the_start_view_keeps_its_keys_out_of_sight() {
    let mut app = app();
    let (editor, cx, probe) = open_with_start(&mut app, &long_text());
    focus(cx, &probe.focus);
    scroll(cx, START * 4.);

    cx.simulate_keystrokes("x delete");
    assert_eq!(*probe.keys.borrow(), 2);
    assert!(text(&editor, cx).starts_with("l00\n"), "keys do not reach the text");
}

#[test]
fn typing_in_the_first_line_in_sight_does_not_scroll() {
    let mut app = app();
    let (editor, cx, probe) = open_with_start(&mut app, &long_text());
    // Half of the start view is in sight, and all of the first line.
    scroll(cx, START / 2.);
    cx.simulate_click(at(cx, 0., START / 2. + LINE_HEIGHT / 2.), Modifiers::none());
    cx.simulate_input("A");
    assert!(text(&editor, cx).starts_with("Al00\n"));

    // The first line is still at the same place.
    cx.simulate_click(at(cx, 1., START / 2. + LINE_HEIGHT / 2.), Modifiers::none());
    cx.simulate_input("B");
    assert!(text(&editor, cx).starts_with("ABl00\n"));
    assert_eq!(*probe.clicks.borrow(), 0);
}
