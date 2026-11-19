use std::io;
use std::path::PathBuf;

/// An error from `holm-core`. Each path is absolute.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The notes folder is missing, or it is not a folder.
    #[error("cannot open the notes folder {path}")]
    NotesFolder {
        /// The notes folder.
        path: PathBuf,
        /// The cause.
        source: io::Error,
    },
    /// A file or a folder could not be read.
    #[error("cannot read {path}")]
    Read {
        /// The file or folder.
        path: PathBuf,
        /// The cause.
        source: io::Error,
    },
    /// A note file is not UTF-8 text.
    #[error("{path} is not UTF-8 text")]
    NotUtf8 {
        /// The note file.
        path: PathBuf,
        /// The cause.
        source: std::str::Utf8Error,
    },
    /// A file or a folder could not be written, moved or deleted.
    #[error("cannot write {path}")]
    Write {
        /// The file or folder.
        path: PathBuf,
        /// The cause.
        source: io::Error,
    },
    /// The index can't be used, even in memory.
    #[error("cannot use the index {path}")]
    Index {
        /// The index file.
        path: PathBuf,
        /// The cause.
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// A folder that Holm does not let the user move or delete, or a move of a folder into
    /// itself. Also a file path that must be absolute but is not, and a changed path from a
    /// [`Feed`](crate::Feed) that leaves the notes folder.
    #[error("{path} is not a note or a folder inside the notes folder")]
    BadPath {
        /// The path as the caller gave it.
        path: PathBuf,
    },
    /// Nothing is left of a typed tag name after [`TagName::typed`](crate::TagName::typed)
    /// cleans it.
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
    /// A save with a seen revision found an outside change of the note that the revision does
    /// not hold. Nothing was written.
    #[error("{path} changed on disk")]
    Changed {
        /// The note file.
        path: PathBuf,
    },
    /// A typed name breaks a rule of file names.
    #[error("{name:?} is not a name: {problem:?}")]
    BadName {
        /// The name as the caller gave it.
        name: String,
        /// The rule it breaks.
        problem: NameProblem,
    },
}

/// The rule of file names that a typed name breaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NameProblem {
    /// The name is empty or only spaces.
    Empty,
    /// The name holds `/`.
    Slash,
    /// The name starts with `.`, so it would be a hidden file.
    Dot,
    /// The name is `attachments` at the root, the folder for attachments.
    Attachments,
    /// The name is too long for a file name.
    TooLong,
    /// The name holds a zero byte, which no file name can hold.
    ZeroByte,
}

/// A `Result` with the `holm-core` [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;
