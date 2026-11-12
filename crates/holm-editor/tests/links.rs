//! Links, file paste and drop, through the editor interface.

mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use common::{LINE_HEIGHT, app, column_x, open};
use gpui_kit::{
    ClipboardEntry, ClipboardItem, Entity, ExternalPaths, FileDropEvent, Image, ImageFormat,
    Modifiers, Pixels, VisualTestContext, point, px,
};
use holm_editor::{CursorLink, Editor, OpenFile, OpenLink, PasteFiles, PastedFile};
use holm_markdown::Target;

const WIDTH: Pixels = px(600.);
const HEIGHT: Pixels = px(400.);

fn text(editor: &Entity<Editor>, cx: &VisualTestContext) -> String {
    cx.read(|cx| editor.read(cx).text().to_owned())
}

/// Collects the events of type `E` that the editor sends.
fn events<E: Clone + 'static>(
    editor: &Entity<Editor>,
    cx: &mut VisualTestContext,
) -> Rc<RefCell<Vec<E>>>
where
    Editor: gpui_kit::EventEmitter<E>,
{
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(editor, move |_, event: &E, _| {
            sink.borrow_mut().push(event.clone());
        })
        .detach();
    });
    events
}

fn note(id: &str) -> Target {
    Target::Note {
        id: id.to_owned(),
        heading: None,
    }
}

fn link_at_cursor(editor: &Entity<Editor>, cx: &VisualTestContext) -> Option<CursorLink> {
    cx.read(|cx| editor.read(cx).link_at_cursor())
}

/// A point at a char column and a pixel height, from the top left of the editor.
fn at(cx: &VisualTestContext, column: f32, y: Pixels) -> gpui_kit::Point<Pixels> {
    point(column_x(cx, column), y)
}

const LINKS: &str = "top\n\nsee [plans](holm:X) and [site](https://a.org)";

#[test]
fn a_click_on_a_link_places_the_cursor() {
    let mut app = app();
    let (editor, cx) = open(&mut app, LINKS, WIDTH, HEIGHT);
    let opened = events::<OpenLink>(&editor, cx);
    // The line shows "see plans and site", so column 6 is in "plans".
    let position = at(cx, 6.5, LINE_HEIGHT * 2.5);
    cx.simulate_click(position, Modifiers::none());
    assert!(opened.borrow().is_empty());
    assert_eq!(
        link_at_cursor(&editor, cx),
        Some(CursorLink {
            target: note("X"),
            embedded: false
        })
    );
}

#[test]
fn ctrl_click_opens_the_link_under_the_mouse() {
    let mut app = app();
    let (editor, cx) = open(&mut app, LINKS, WIDTH, HEIGHT);
    let opened = events::<OpenLink>(&editor, cx);
    let position = at(cx, 6.5, LINE_HEIGHT * 2.5);
    cx.simulate_click(position, Modifiers::control());
    let position = at(cx, 15.5, LINE_HEIGHT * 2.5);
    cx.simulate_click(position, Modifiers::control());
    assert_eq!(
        *opened.borrow(),
        vec![
            OpenLink { target: note("X") },
            OpenLink {
                target: Target::Web("https://a.org".into())
            },
        ]
    );
    // The cursor stays at the start.
    cx.simulate_input("!");
    assert!(text(&editor, cx).starts_with("!top"));
}

#[test]
fn ctrl_enter_opens_the_link_at_the_cursor() {
    let mut app = app();
    let (editor, cx) = open(
        &mut app,
        "[a](<my file.pdf>)\n\n# Plans\n\n[b](#Plans)",
        WIDTH,
        HEIGHT,
    );
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_folder(Some(PathBuf::from("/notes")), cx);
        });
    });
    let files = events::<OpenFile>(&editor, cx);
    cx.simulate_keystrokes("right ctrl-enter");
    assert_eq!(
        *files.borrow(),
        vec![OpenFile {
            path: PathBuf::from("/notes/my file.pdf")
        }]
    );
    // A heading in this note: the cursor moves to it.
    cx.simulate_keystrokes("ctrl-end left ctrl-enter");
    cx.simulate_input("X");
    assert_eq!(
        text(&editor, cx),
        "[a](<my file.pdf>)\n\nX# Plans\n\n[b](#Plans)"
    );
    // No link at the cursor: Ctrl+Enter does nothing.
    cx.simulate_keystrokes("ctrl-enter");
    assert_eq!(files.borrow().len(), 1);
}

#[test]
fn insert_link_takes_the_selection_and_set_link_target_keeps_the_text() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "see plans", WIDTH, HEIGHT);
    cx.simulate_keystrokes("ctrl-end shift-ctrl-left");
    assert_eq!(
        cx.read(|cx| editor.read(cx).selected_text().to_owned()),
        "plans"
    );
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.insert_link("Plans", &note("X"), false, cx);
        });
    });
    assert_eq!(text(&editor, cx), "see [plans](holm:X)");
    assert_eq!(
        link_at_cursor(&editor, cx).map(|l| l.target),
        Some(note("X"))
    );

    let heading = Target::Note {
        id: "Y".into(),
        heading: Some("Bug list".into()),
    };
    let changed =
        cx.update(|_, cx| editor.update(cx, |editor, cx| editor.set_link_target(&heading, cx)));
    assert!(changed);
    assert_eq!(text(&editor, cx), "see [plans](<holm:Y#Bug list>)");
    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&editor, cx), "see [plans](holm:X)");
}

#[test]
fn ctrl_v_with_files_sends_paste_files() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "", WIDTH, HEIGHT);
    let pasted = events::<PasteFiles>(&editor, cx);
    let paths = ExternalPaths(vec![PathBuf::from("/tmp/a.pdf")].into());
    cx.write_to_clipboard(ClipboardItem {
        entries: vec![ClipboardEntry::ExternalPaths(paths)],
    });
    cx.simulate_keystrokes("ctrl-v");
    let image = Image::from_bytes(ImageFormat::Png, vec![1, 2, 3]);
    cx.write_to_clipboard(ClipboardItem::new_image(&image));
    cx.simulate_keystrokes("ctrl-v");
    assert_eq!(
        *pasted.borrow(),
        vec![
            PasteFiles {
                files: vec![PastedFile::Path(PathBuf::from("/tmp/a.pdf"))]
            },
            PasteFiles {
                files: vec![PastedFile::Image {
                    extension: "png",
                    bytes: vec![1, 2, 3]
                }]
            },
        ]
    );
    assert_eq!(text(&editor, cx), "");

    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.insert_link("", &Target::File("attachments/a.pdf".into()), true, cx);
        });
    });
    assert_eq!(text(&editor, cx), "![](attachments/a.pdf)\n");
}

#[test]
fn a_drop_moves_the_cursor_and_sends_paste_files() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "one\n\ntwo", WIDTH, HEIGHT);
    let pasted = events::<PasteFiles>(&editor, cx);
    let position = at(cx, 1.2, LINE_HEIGHT * 2.5);
    let paths = ExternalPaths(vec![PathBuf::from("/tmp/b.png")].into());
    cx.simulate_event(FileDropEvent::Entered { position, paths });
    cx.simulate_event(FileDropEvent::Submit { position });
    cx.simulate_event(FileDropEvent::Ended);
    assert_eq!(
        *pasted.borrow(),
        vec![PasteFiles {
            files: vec![PastedFile::Path(PathBuf::from("/tmp/b.png"))]
        }]
    );
    cx.simulate_input("X");
    assert_eq!(text(&editor, cx), "one\n\ntXwo");
}

#[test]
fn show_heading_puts_the_cursor_at_the_heading() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "# Top\n\ntext\n\n## Bug list\n", WIDTH, HEIGHT);
    let found =
        cx.update(|_, cx| editor.update(cx, |editor, cx| editor.show_heading("Bug list", cx)));
    assert!(found);
    cx.simulate_input("X");
    assert_eq!(text(&editor, cx), "# Top\n\ntext\n\nX## Bug list\n");

    let found =
        cx.update(|_, cx| editor.update(cx, |editor, cx| editor.show_heading("bug list", cx)));
    assert!(!found);
}

#[test]
fn an_embedded_link_gets_a_line_of_its_own() {
    let mut app = app();
    let (editor, cx) = open(&mut app, "ab", WIDTH, HEIGHT);
    cx.simulate_keystrokes("right");
    let file = |path: &str| Target::File(path.into());
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.insert_link("x", &file("x.pdf"), true, cx);
            editor.insert_link("y", &file("y.pdf"), true, cx);
            editor.insert_link("z", &file("z.pdf"), false, cx);
        });
    });
    assert_eq!(
        text(&editor, cx),
        "a\n![x](x.pdf)\n![y](y.pdf)\n[z](z.pdf)b"
    );
}
