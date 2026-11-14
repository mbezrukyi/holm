//! Parses a note into frontmatter, links, tags and styled spans, and restyles inline
//! text for the format keys.
//!
//! It does no I/O. The caller reads and writes the files.

mod body;
mod frontmatter;
pub mod inline;
mod note;
mod preview;
mod tag;
mod yaml;

pub use body::{
    Alignment, Block, BlockKind, Body, CalloutKind, Color, Heading, Link, Span, Style, Target,
    Task, escape_text, unescape_text,
};
pub use note::{LineEnding, Note};
pub use tag::normalize_tag;

/// An error from `holm-markdown`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {}
