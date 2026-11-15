//! The types that the [`Core`](crate::Core) handle takes and gives.

use std::fmt;
use std::path::{Path, PathBuf};
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
    /// The file name without `.md`.
    pub title: String,
    /// The first line of text of the body, with no Markdown syntax. It can be empty.
    pub preview: String,
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
    /// Whether it is `![..](..)`: an image or a file chip.
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

/// What a delete removes, for the delete dialog.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeleteCount {
    /// How many notes.
    pub notes: usize,
    /// How many files in `attachments/` that no other note links.
    pub attachments: usize,
}

/// One change to the notes folder, in a [`ChangeList`].
///
/// A write through [`Core`](crate::Core) gives only the moves it makes: [`Change::Moved`],
/// [`Change::FolderMoved`] and [`Change::TagMoved`]. Its other changes show only in the
/// snapshot.
///
/// The core finds no folder moves from other tools. Each note of a moved folder comes as
/// [`Change::Moved`] or as [`Change::Removed`] and [`Change::Added`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Change {
    /// A new note file from another tool.
    Added(NoteInfo),
    /// A note file whose text or info another tool changed.
    Changed(NoteInfo),
    /// A note file that is gone. The path is where it was.
    Removed(PathBuf),
    /// A note that is now at another path. From another tool, only a note with an id moves.
    Moved {
        /// The old path.
        from: PathBuf,
        /// The note at its new path.
        note: NoteInfo,
    },
    /// A folder that Holm moved, with everything below it.
    FolderMoved {
        /// The old path.
        from: PathBuf,
        /// The new path.
        to: PathBuf,
    },
    /// A tag that Holm renamed, with its child tags.
    TagMoved {
        /// The old name.
        from: String,
        /// The new name.
        to: String,
    },
    /// The list from [`Snapshot::folders`](crate::Snapshot::folders) changed.
    Folders,
    /// The tag tree from [`Snapshot::tags`](crate::Snapshot::tags) changed.
    Tags,
}

impl Change {
    /// Where the note or folder at `path` is after this change, when the change moves it.
    pub fn path_after(&self, path: &Path) -> Option<PathBuf> {
        match self {
            Self::Moved { from, note } if from == path => Some(note.path.clone()),
            Self::FolderMoved { from, to } => {
                let rest = path.strip_prefix(from).ok()?;
                Some(if rest.as_os_str().is_empty() {
                    to.clone()
                } else {
                    to.join(rest)
                })
            }
            _ => None,
        }
    }

    /// The name of `tag` after this change, when the change renames it or a parent tag.
    pub fn tag_after(&self, tag: &str) -> Option<String> {
        let Self::TagMoved { from, to } = self else {
            return None;
        };
        if tag == from {
            return Some(to.clone());
        }
        let rest = tag.strip_prefix(from.as_str())?.strip_prefix('/')?;
        Some(format!("{to}/{rest}"))
    }
}

/// The changes that made one revision of the snapshot, from [`Changes`](crate::Changes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeList {
    /// The revision of the first snapshot that shows the changes.
    pub revision: u64,
    /// The changes in order: first the note and folder changes, then [`Change::Folders`] and
    /// [`Change::Tags`]. It is never empty.
    pub changes: Vec<Change>,
}
