//! The live preview Markdown editor element for GPUI.
//!
//! It uses `holm-markdown` for parsing. It does not know the core.

mod editor;
mod element;
mod highlight;
mod ink;
mod model;
mod palette;

pub use editor::{Appearance, Changed, Editor, OpenFile, WritingHelp};

/// An error from `holm-editor`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {}
