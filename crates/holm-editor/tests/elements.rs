//! How the editor shows Markdown elements: the cursor rules of code blocks and tables, task
//! boxes and file chips.

mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use common::{FONT_SIZE, LINE_HEIGHT, MARGIN, advance, app, column_x, open};
use gpui_kit::{Entity, Modifiers, Pixels, VisualTestContext, point, px};
use holm_editor::{Editor, OpenFile};

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

#[test]
fn code_fences_hide_until_the_cursor_enters_the_block() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "a\n```\ncode\n```\nb", WIDTH, HEIGHT);
    // Away from the cursor each fence takes half a line.
    let fence = LINE_HEIGHT * 0.5;
    click(
        cx,
        0.2,
        LINE_HEIGHT + fence + LINE_HEIGHT + fence + LINE_HEIGHT * 0.5,
    );
    cx.simulate_input("X");
    assert_eq!(text(&editor, cx), "a\n```\ncode\n```\nXb");

    click(cx, 0.2, LINE_HEIGHT + fence + LINE_HEIGHT * 0.5);
    cx.simulate_input("Y");
    assert_eq!(text(&editor, cx), "a\n```\nYcode\n```\nXb");

    // Now the fences show as full lines.
    click(cx, 0.2, LINE_HEIGHT * 1.5);
    cx.simulate_input("Z");
    assert_eq!(text(&editor, cx), "a\nZ```\nYcode\n```\nXb");
}

#[test]
fn a_table_shows_its_source_while_the_cursor_is_inside() {
    let mut app = app();
    let (editor, cx) = open(
        &mut app,
        "a\n\n| x | y |\n|---|---|\n| 1 | 2 |",
        WIDTH,
        HEIGHT,
    );
    // In the grid a row is a line and a bit high, and the `|---|` line takes no space.
    let row = LINE_HEIGHT * 1.3;
    click(cx, 0.2, LINE_HEIGHT * 2. + row + row * 0.5);
    cx.simulate_input("X");
    assert_eq!(text(&editor, cx), "a\n\n| x | y |\n|---|---|\n| X1 | 2 |");

    // Now each table line is one row of source.
    click(cx, 0.2, LINE_HEIGHT * 3.5);
    cx.simulate_input("Z");
    assert_eq!(text(&editor, cx), "a\n\n| x | y |\nZ|---|---|\n| X1 | 2 |");
}

/// The middle of the task box on the line that starts at `top`.
fn task_box(top: Pixels) -> gpui_kit::Point<Pixels> {
    let area = FONT_SIZE * 1.6;
    point(
        MARGIN + area / 2. - FONT_SIZE * 0.1,
        top + LINE_HEIGHT * 0.5,
    )
}

#[test]
fn a_click_on_a_task_box_toggles_it_and_keeps_the_cursor() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "a\n\n- [ ] task", WIDTH, HEIGHT);
    cx.simulate_click(task_box(LINE_HEIGHT * 2.), Modifiers::none());
    assert_eq!(text(&editor, cx), "a\n\n- [x] task");
    cx.simulate_input("Q");
    assert_eq!(text(&editor, cx), "Qa\n\n- [x] task");
    cx.simulate_click(task_box(LINE_HEIGHT * 2.), Modifiers::none());
    assert_eq!(text(&editor, cx), "Qa\n\n- [ ] task");
}

#[test]
fn ctrl_l_toggles_the_task_at_the_cursor() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "- [ ] one\n- [ ] two", WIDTH, HEIGHT);
    cx.simulate_keystrokes("ctrl-l");
    assert_eq!(text(&editor, cx), "- [x] one\n- [ ] two");
    cx.simulate_keystrokes("ctrl-a ctrl-l");
    assert_eq!(text(&editor, cx), "- [ ] one\n- [x] two");
}

/// The paths of the files that `editor` asks to open, from now on.
fn opened_files(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Rc<RefCell<Vec<PathBuf>>> {
    let opened = Rc::new(RefCell::new(Vec::new()));
    let seen = opened.clone();
    cx.update(|_, cx| {
        cx.subscribe(editor, move |_, event: &OpenFile, _| {
            seen.borrow_mut().push(event.path.clone());
        })
        .detach();
    });
    opened
}

#[test]
fn a_file_link_shows_as_a_chip_in_its_line() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "a\n\nSee ![f](report.pdf) now\nb", WIDTH, HEIGHT);
    // The chip line is one row high, so the next row holds "b".
    click(cx, 0.2, LINE_HEIGHT * 3.5);
    cx.simulate_input("X");
    assert_eq!(text(&editor, cx), "a\n\nSee ![f](report.pdf) now\nXb");
}

#[test]
fn ctrl_click_on_a_file_chip_sends_open_file() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "a\n\nSee ![f](report.pdf) now", WIDTH, HEIGHT);
    let opened = opened_files(&editor, cx);
    // "See ", then the icon room of six chars, then the name.
    let name = point(column_x(cx, 12.5), LINE_HEIGHT * 2.5);
    cx.simulate_click(name, Modifiers::control());
    assert_eq!(*opened.borrow(), [PathBuf::from("report.pdf")]);

    // A plain click only places the cursor, and the line shows its Markdown.
    cx.simulate_click(name, Modifiers::none());
    assert_eq!(opened.borrow().len(), 1);
    cx.simulate_keystrokes("ctrl-enter");
    assert_eq!(*opened.borrow(), vec![PathBuf::from("report.pdf"); 2]);
    assert_eq!(text(&editor, cx), "a\n\nSee ![f](report.pdf) now");
}

#[test]
fn a_list_line_keeps_its_text_in_place_when_it_shows_its_marker() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "a\n\n- one", WIDTH, HEIGHT);
    // The text of a top level item starts after the marker area of 1.6 text sizes.
    let x = MARGIN + FONT_SIZE * 1.6 + advance(cx) * 1.2;
    let y = LINE_HEIGHT * 2.5;
    cx.simulate_click(point(x, y), Modifiers::none());
    cx.simulate_input("X");
    assert_eq!(text(&editor, cx), "a\n\n- oXne");
    // Now the line shows "- " to the left of the text, so the same point is still after "o".
    cx.simulate_click(point(x, y), Modifiers::none());
    cx.simulate_input("Y");
    assert_eq!(text(&editor, cx), "a\n\n- oYXne");
}
