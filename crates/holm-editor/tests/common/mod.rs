//! Test windows for the editor, with a real text system and the font from the repo.

#![allow(dead_code, reason = "each test file uses only some helpers")]
#![allow(unreachable_pub, reason = "test helpers")]
#![allow(clippy::expect_used, reason = "tests")]

use std::borrow::Cow;
use std::path::PathBuf;

use gpui_kit::{
    AppContext, Context, Entity, Focusable, IntoElement, ParentElement, Pixels, Render, Styled,
    TestAppContext, TestDispatcher, VisualTestContext, Window, div, font, px, size,
};
use holm_editor::Editor;

pub const FONT: &str = "Fira Mono";
pub const FONT_SIZE: Pixels = px(16.);
pub const LINE_HEIGHT: Pixels = px(20.);
/// The space between the editor edge and the text column, in a test window.
pub const MARGIN: Pixels = px(32.);

/// Gives the editor its text style, like the app does.
pub struct Host {
    pub editor: Entity<Editor>,
}

impl Render for Host {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .font_family(FONT)
            .text_size(FONT_SIZE)
            .line_height(LINE_HEIGHT)
            .child(self.editor.clone())
    }
}

/// A test app with the real text system and only the repo font in use.
pub fn app() -> TestAppContext {
    let text_system = gpui_kit::platform::current_platform(true).text_system();
    text_system
        .add_fonts(vec![Cow::Borrowed(include_bytes!(
            "../fonts/FiraMono-Medium.ttf"
        ))])
        .expect("the repo font loads");
    let cx = TestAppContext::build_with_text_system(TestDispatcher::new(0), None, text_system);
    cx.update(holm_editor::init);
    cx
}

/// A focused editor with `text` in a window of `width` by `height`.
pub fn open<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
    width: Pixels,
    height: Pixels,
) -> (Entity<Editor>, &'a mut VisualTestContext) {
    let text = text.to_owned();
    let (host, cx) = cx.add_window_view(|_, cx| {
        let editor = cx.new(|cx| {
            let mut editor = Editor::new(cx);
            editor.set_text(&text, cx);
            editor
        });
        Host { editor }
    });
    cx.simulate_resize(size(width, height));
    let editor = cx.read(|cx| host.read(cx).editor.clone());
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

/// The width of one char of the repo font.
/// The x of a char column, from the left of the editor.
pub fn column_x(cx: &VisualTestContext, column: f32) -> Pixels {
    MARGIN + advance(cx) * column
}

pub fn advance(cx: &VisualTestContext) -> Pixels {
    let text_system = cx.text_system().clone();
    let font_id = text_system.resolve_font(&font(FONT));
    text_system
        .advance(font_id, FONT_SIZE, 'm')
        .expect("the repo font has an advance")
        .width
}

pub fn data_folder() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

/// A part of a note with every element, each block closed.
const CHUNK: &str = include_str!("../data/chunk.md");

/// About 1 MB of [`CHUNK`] parts, one after another.
pub fn one_mb_note() -> String {
    CHUNK.repeat(1_000_000 / CHUNK.len() + 1)
}

/// 200 long lines that wrap, each with 12 file chips.
pub fn chip_lines_note() -> String {
    let part =
        "some words before a file ![report](attachments/report.pdf) and more words after it, ";
    let line = part.repeat(12);
    vec![line.trim_end(); 200].join("\n\n")
}
