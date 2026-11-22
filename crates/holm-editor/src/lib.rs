//! The live preview Markdown editor element for GPUI.
//!
//! It uses `holm-markdown` for parsing. It does not know the core.

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

/// Binds the keys of [`shortcuts`]. The app calls it once, before it opens an editor.
pub fn init(cx: &mut gpui_kit::App) {
    shortcuts::bind(cx);
}

/// An error from `holm-editor`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {}
