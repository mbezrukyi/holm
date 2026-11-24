use std::path::PathBuf;

use gpui_kit::{Entity, Modifiers, Pixels, Point, VisualTestContext, point};
use holm_editor::Editor;
use holm_editor::testing::column_x;

/// The folder of the files that the test notes link to.
pub(crate) fn data_folder() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/test_data/images"))
}

pub(crate) fn text(editor: &Entity<Editor>, cx: &VisualTestContext) -> String {
    cx.read(|cx| editor.read(cx).text().to_owned())
}

pub(crate) fn at(cx: &VisualTestContext, column: f32, y: Pixels) -> Point<Pixels> {
    point(column_x(cx, column), y)
}

pub(crate) fn click(cx: &mut VisualTestContext, column: f32, y: Pixels) {
    cx.simulate_click(at(cx, column, y), Modifiers::none());
}
