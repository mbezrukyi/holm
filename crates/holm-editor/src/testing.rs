//! Helpers for the tests and benches of this crate.

#![expect(clippy::expect_used, reason = "test helpers")]

use std::borrow::Cow;

use gpui_kit::{
    AppContext, Bounds, Context, Entity, EntityInputHandler as _, Focusable, IntoElement, ParentElement, Pixels,
    Render, Styled, TestAppContext, TestDispatcher, VisualTestContext, Window, div, font, px, size,
};

use crate::Editor;

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
        .add_fonts(vec![Cow::Borrowed(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/test_data/fonts/FiraMono-Medium.ttf"
        )))])
        .expect("the repo font loads");
    let cx = TestAppContext::build_with_text_system(TestDispatcher::new(0), None, text_system);
    cx.update(crate::init);
    cx
}

/// The editor has the focus.
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

/// The middle height of the cursor in the window. It fails when the cursor line is not laid out.
pub fn cursor_y(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Pixels {
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.bounds_for_range(0..0, Bounds::default(), window, cx)
        })
    })
    .map(|bounds| bounds.center().y)
    .expect("the cursor line is in the layout")
}

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
