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

pub use editor::{
    Appearance, Changed, CursorLink, CustomColors, Editor, OpenFile, OpenLink, PasteFiles,
    PastedFile, UpFromStart, WritingHelp,
};
pub use element::text_column;

/// An error from `holm-editor`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {}
