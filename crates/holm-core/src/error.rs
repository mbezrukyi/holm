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
    /// The index can't be opened or used, also after a rebuild.
    #[error("cannot use the index {path}")]
    Index {
        /// The index file.
        path: PathBuf,
        /// The cause.
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// The path is absolute, leaves the notes folder, is hidden, or is not a `.md` file.
    /// Also a folder that Holm does not let the user move or delete, or an empty name.
    #[error("{path} is not a note or a folder inside the notes folder")]
    BadPath {
        /// The path as the caller gave it.
        path: PathBuf,
    },
    /// The tag name is empty, or it is not written as Holm writes tags.
    #[error("{tag:?} is not a tag name")]
    BadTag {
        /// The tag as the caller gave it.
        tag: String,
    },
}

/// A `Result` with the `holm-core` [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;
