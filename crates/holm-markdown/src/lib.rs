//! Parses a note into frontmatter, title, links, tags and styled spans.
//!
//! It does no I/O.

/// An error from `holm-markdown`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {}
