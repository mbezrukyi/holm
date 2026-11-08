//! The types that the [`Core`](crate::Core) handle takes and gives.

use std::fmt;
use std::path::PathBuf;
use std::time::SystemTime;

/// The stable id of a note. It lives in the `id:` field of the frontmatter.
///
/// Holm makes a ULID for each new id. An id that the user wrote by hand stays as it is.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NoteId(String);

impl NoteId {
    /// Makes a new id from a fresh ULID.
    pub(crate) fn generate() -> Self {
        Self(ulid::Ulid::generate().to_string())
    }

    pub(crate) fn from_field(id: String) -> Self {
        Self(id)
    }

    /// The id as text, like in `holm:<id>` links.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NoteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a list of notes shows about one note. It has no body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteInfo {
    /// The path of the file, relative to the notes folder, like `work/Plans.md`.
    pub path: PathBuf,
    /// The `id:` field. A note that Holm never wrote can have none.
    pub id: Option<NoteId>,
    /// The first `# heading` of the body, or the file name without `.md`.
    pub title: String,
    /// The `tags:` field, normalized, with no repeats.
    pub tags: Vec<String>,
    /// Whether the frontmatter has `pinned: true`.
    pub pinned: bool,
    /// The modification time of the file.
    pub modified: SystemTime,
}

/// A whole note: its info and its body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    /// The info about the note.
    pub info: NoteInfo,
    /// The text after the frontmatter, with the line endings of the file.
    pub body: String,
}
