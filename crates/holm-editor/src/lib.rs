//! The live preview Markdown editor element for GPUI.
//!
//! It uses `holm-markdown` for parsing. It does not know the core.

mod editor;
mod element;
mod model;

pub use editor::{Changed, Editor};

/// An error from `holm-editor`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {}
