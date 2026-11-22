use std::io;
use std::path::PathBuf;

/// A path that Holm builds from the notes folder is absolute. A path that the caller gave stays as given, so it can be
/// relative. The fields of such paths say so.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The notes folder is missing, or it is not a folder.
    #[error("cannot open the notes folder {path}")]
    NotesFolder {
        /// The notes folder as the caller gave it.
        path: PathBuf,
        source: io::Error,
    },
    /// A file or a folder could not be read.
    #[error("cannot read {path}")]
    Read {
        /// Absolute, but a file to attach is as the caller gave it.
        path: PathBuf,
        source: io::Error,
    },
    #[error("{path} is not UTF-8 text")]
    NotUtf8 { path: PathBuf, source: std::str::Utf8Error },
    /// A file or a folder could not be written, moved or deleted.
    #[error("cannot write {path}")]
    Write { path: PathBuf, source: io::Error },
    /// The index can't be used, even in memory.
    #[error("cannot use the index {path}")]
    Index {
        /// The index file in the index folder as the caller gave it.
        path: PathBuf,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// A folder that Holm does not let the user move or delete, or a move of a folder into itself. Also a file path
    /// that must be absolute but is not, and a changed path from a [`Feed`](crate::Feed) that leaves the notes folder.
    #[error("{path} is not a note or a folder inside the notes folder")]
    BadPath {
        /// The path as the caller gave it.
        path: PathBuf,
    },
    /// Nothing is left of a typed tag name after [`TagName::typed`](crate::TagName::typed) cleans it.
    #[error("{tag:?} is not a tag name")]
    BadTag {
        /// The tag as the person typed it.
        tag: String,
    },
    /// Something in the folder already has this name, also when only the capital letters differ.
    #[error("{path} is taken")]
    NameTaken {
        /// The path that the name asks for.
        path: PathBuf,
    },
    /// A save with a seen revision found an outside change of the note that the revision does not hold. Nothing was
    /// written.
    #[error("{path} changed on disk")]
    Changed { path: PathBuf },
    #[error("{name:?} is not a name: {problem:?}")]
    BadName {
        /// The name as the caller gave it.
        name: String,
        problem: NameProblem,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NameProblem {
    /// The name is empty or only spaces.
    Empty,
    Slash,
    /// The name starts with `.`, so it would be a hidden file.
    Dot,
    /// The name is `attachments` at the root, the folder for attachments.
    Attachments,
    TooLong,
    /// The name holds a zero byte, which no file name can hold.
    ZeroByte,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
