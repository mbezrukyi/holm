//! The crate does no I/O. The caller reads and writes the files.

mod body;
mod frontmatter;
pub mod inline;
pub mod list;
mod note;
mod offset;
mod preview;
mod tag;
mod yaml;

pub use body::{
    Alignment, Block, BlockKind, Body, CalloutKind, CodeStyle, Color, Heading, Link, LinkKind, Span, Style, TablePart,
    Target, Task, TaskState, escape_text, unescape_text,
};
pub use note::{LineEnding, NoteFile};
pub(crate) use offset::JoinedOffset;
pub use offset::{NewOffset, OffsetRange, SourceOffset};
pub use tag::normalize_tag;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {}
