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

/// The order of a list of notes. Pinned notes are always on top.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Order {
    /// The last changed note first.
    #[default]
    LastChange,
    /// By title, from A to Z, with no regard to capital letters.
    Title,
}

/// One tag in the tag tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    /// The full name, like `work/holm`.
    pub name: String,
    /// How many notes have this tag or one of its child tags.
    pub notes: usize,
    /// The child tags, like `work/holm` under `work`, sorted by name.
    pub children: Vec<Tag>,
}

/// A link or an embedded file in a note.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Link {
    /// Where the link points.
    pub target: LinkTarget,
    /// Whether it is `![..](..)`: an image or a file card.
    pub embedded: bool,
}

/// Where a link points.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LinkTarget {
    /// Another note, or a heading in it: `holm:<id>` or `holm:<id>#<heading>`.
    Note {
        /// The id of the note.
        id: NoteId,
        /// The heading text after `#`.
        heading: Option<String>,
    },
    /// A heading in the same note: `#<heading>`.
    Heading(String),
    /// A file. The path is relative to the notes folder when the file is inside it.
    /// It is absolute when the file is outside.
    File(PathBuf),
    /// A web address, like `https://...` or `mailto:...`.
    Web(String),
}

/// One change to the notes folder that came from outside Holm, from [`Changes`](crate::Changes).
///
/// Saves through [`Core`](crate::Core) never come back as changes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Change {
    /// A new note file.
    Added(NoteInfo),
    /// A note file whose text or info changed.
    Changed(NoteInfo),
    /// A note file that is gone. The path is where it was.
    Removed(PathBuf),
    /// A note file with an id that is now at another path.
    Moved {
        /// The old path.
        from: PathBuf,
        /// The note at its new path.
        note: NoteInfo,
    },
    /// The list from [`Core::folders`](crate::Core::folders) may have changed.
    Folders,
    /// The tag tree from [`Core::tags`](crate::Core::tags) changed.
    Tags,
}
