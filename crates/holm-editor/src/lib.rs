//! It does not know the core.

mod color;
mod editor;
mod element;
mod files;
mod find;
mod highlight;
mod ink;
mod model;
mod palette;
pub mod shortcuts;

pub use editor::{
    Appearance, Changed, CursorLink, CustomColors, Editor, OpenFile, OpenLink, PasteFiles, PastedFile, TextWidth,
    UpFromStart, WritingHelp,
};
pub use element::{TextColumn, text_column};

/// Call it once, before the first editor opens.
pub fn init(cx: &mut gpui_kit::App) {
    shortcuts::bind(cx);
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {}
