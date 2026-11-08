//! Parses a note into frontmatter, title, links, tags and styled spans.
//!
//! It does no I/O. The caller reads and writes the files.

mod frontmatter;
mod note;
mod tag;
mod title;
mod yaml;

pub use note::{LineEnding, Note};
pub use tag::normalize_tag;
pub use title::file_stem;

/// An error from `holm-markdown`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {}
