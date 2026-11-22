//! How the editor shows Markdown elements: the cursor rules of code blocks and tables, task
//! boxes and file chips.

mod common;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use common::{FONT_SIZE, LINE_HEIGHT, MARGIN, advance, app, column_x, open};
use gpui_kit::{Entity, EntityInputHandler as _, Modifiers, Pixels, Point, VisualTestContext, point, px};
use holm_editor::{Editor, OpenFile};

const WIDTH: Pixels = px(600.);
const HEIGHT: Pixels = px(400.);
/// A window that shows all of `FLEX_TABLE`.
const TALL: Pixels = px(900.);

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
    click(cx, 0.2, LINE_HEIGHT + fence + LINE_HEIGHT + fence + LINE_HEIGHT * 0.5);
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
    let (editor, cx) = open(&mut app, "a\n\n| x | y |\n|---|---|\n| 1 | 2 |", WIDTH, HEIGHT);
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

/// The table of the flexbox note, whose words broke into letters.
const FLEX_TABLE: &str = "a\n\n\
| Property | Axis | Common values | What it does |
|---|---|---|---|
| `flex-direction` | — | `row`, `column`, `row-reverse` | Picks the main axis |
| `flex-wrap` | main | `nowrap` (default), `wrap` | Allows items to break onto new lines |
| `justify-content` | main | `start`, `center`, `space-between`, `end` | Distributes leftover main-axis space |
| `align-items` | cross | `stretch` (default), `start`, `center`, `baseline` | Aligns items inside their line |
| `gap` | both | `1rem`, `1rem 2rem` | Space between items. Replaces margin hacks |";

/// Where each char offset shows in a window of `width` by `height`, by a sweep of points 2 px
/// apart over the rows. A click past the end of a row gives the start of the next row, so each
/// offset keeps the first point on its lowest row.
fn char_places(
    editor: &Entity<Editor>,
    cx: &mut VisualTestContext,
    (width, height): (Pixels, Pixels),
) -> HashMap<usize, Point<Pixels>> {
    let mut places = HashMap::new();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let mut y = LINE_HEIGHT / 2.;
            while y < height {
                let mut x = px(0.);
                while x < width {
                    let at = point(x, y);
                    if let Some(offset) = editor.character_index_for_point(at, window, cx) {
                        let place = places.entry(offset).or_insert(at);
                        if at.y > place.y {
                            *place = at;
                        }
                    }
                    x += px(2.);
                }
                y += LINE_HEIGHT / 2.;
            }
        });
    });
    places
}

/// Whether `word` in `FLEX_TABLE` shows on one row, by the places of its first and last char.
/// `None` when it is not in the table or does not show.
fn on_one_row(places: &HashMap<usize, Point<Pixels>>, word: &str) -> Option<bool> {
    let start = FLEX_TABLE.find(word)?;
    let utf16 = |end: usize| FLEX_TABLE[..end].encode_utf16().count();
    let first = places.get(&utf16(start))?;
    let last = places.get(&utf16(start + word.len() - 1))?;
    Some((first.y - last.y).abs() < LINE_HEIGHT)
}

#[test]
fn a_too_wide_table_wraps_its_long_text_and_keeps_short_words_whole() {
    let mut app = app();
    let (editor, cx) = open(&mut app, FLEX_TABLE, WIDTH, TALL);
    let places = char_places(&editor, cx, (WIDTH, TALL));
    for word in ["Property", "Axis", "cross", "justify-", "space-", "Distributes"] {
        assert_eq!(on_one_row(&places, word), Some(true), "{word} breaks");
    }
    assert_eq!(on_one_row(&places, "Distributes leftover main-axis space"), Some(false));
    // The table stays inside the text column.
    let end = FLEX_TABLE.find("margin hacks").expect("in the table") + "margin hacks".len();
    let last = places
        .get(&FLEX_TABLE[..end - 1].encode_utf16().count())
        .expect("the last char shows");
    assert!(last.x < WIDTH - MARGIN, "{last:?}");
}

#[test]
fn a_table_that_fits_keeps_each_cell_on_one_row() {
    let mut app = app();
    let wide = px(1400.);
    let (editor, cx) = open(&mut app, FLEX_TABLE, wide, TALL);
    let places = char_places(&editor, cx, (wide, TALL));
    for cell in [
        "Distributes leftover main-axis space",
        "Space between items. Replaces margin hacks",
        "justify-content",
    ] {
        assert_eq!(on_one_row(&places, cell), Some(true), "{cell} wraps");
    }
}

/// The middle of the task box on the line that starts at `top`.
fn task_box(top: Pixels) -> gpui_kit::Point<Pixels> {
    let area = FONT_SIZE * 1.6;
    point(MARGIN + area / 2. - FONT_SIZE * 0.1, top + LINE_HEIGHT * 0.5)
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
    let (editor, cx) = open(&mut app, "a\n\nSee ![report.pdf](report.pdf) now", WIDTH, HEIGHT);
    let opened = opened_files(&editor, cx);
    // "See ", then the chip, with its name after the icon.
    let name = point(column_x(cx, 12.5), LINE_HEIGHT * 2.5);
    cx.simulate_click(name, Modifiers::control());
    assert_eq!(*opened.borrow(), [PathBuf::from("report.pdf")]);

    // A plain click only places the cursor, and the line shows its Markdown.
    cx.simulate_click(name, Modifiers::none());
    assert_eq!(opened.borrow().len(), 1);
    cx.simulate_keystrokes("ctrl-enter");
    assert_eq!(*opened.borrow(), vec![PathBuf::from("report.pdf"); 2]);
    assert_eq!(text(&editor, cx), "a\n\nSee ![report.pdf](report.pdf) now");
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
