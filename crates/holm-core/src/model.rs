//! The types that the [`Core`](crate::Core) handle takes and gives.

use std::fmt;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::{FolderPath, NotePath, Snapshot, TagName};

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
    /// The path of the file, like `work/Plans.md`.
    pub path: NotePath,
    /// The `id:` field. A note that Holm never wrote can have none.
    pub id: Option<NoteId>,
    /// The file name without `.md`.
    pub title: String,
    /// The first line of text of the body, with no Markdown syntax. It can be empty.
    pub preview: String,
    /// The `tags:` field, normalized, with no repeats.
    pub tags: Vec<TagName>,
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

/// Which notes of a [`Snapshot`] a list holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteSet<'a> {
    /// Every note.
    All,
    /// The notes right in a folder, without the notes of its subfolders.
    In(&'a FolderPath),
    /// The notes with a tag or one of its child tags.
    WithTag(&'a TagName),
}

impl NoteSet<'_> {
    /// Whether the set holds `note`.
    pub fn holds(self, note: &NoteInfo) -> bool {
        self.holds_at(&note.path, || Some(note))
    }

    /// Whether the set holds the note at `path`. It calls `note` only when the path can't tell.
    pub(crate) fn holds_at<'n>(
        self,
        path: &NotePath,
        note: impl FnOnce() -> Option<&'n NoteInfo>,
    ) -> bool {
        match self {
            Self::All => true,
            Self::In(folder) => path.is_in(folder),
            Self::WithTag(tag) => {
                note().is_some_and(|note| note.tags.iter().any(|name| tag.holds(name)))
            }
        }
    }
}

/// One tag of the nested tags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    /// The full name, like `work/holm`.
    pub name: TagName,
    /// How many notes have this tag or one of its child tags.
    pub notes: usize,
    /// The child tags, like `work/holm` under `work`, sorted by name.
    pub children: Vec<Tag>,
}

impl Tag {
    /// Each tag of `tags` and each child tag at any depth. A tag comes before its child tags.
    pub fn walk(tags: &[Self]) -> impl Iterator<Item = &Self> {
        let mut stack = vec![tags.iter()];
        std::iter::from_fn(move || {
            loop {
                match stack.last_mut()?.next() {
                    Some(tag) => {
                        stack.push(tag.children.iter());
                        return Some(tag);
                    }
                    None => {
                        stack.pop();
                    }
                }
            }
        })
    }
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

/// One change to the notes folder, in a [`ChangeList`]. A note change names only paths. The
/// snapshot of the list has the facts of each note.
///
/// A write through [`Core`](crate::Core) gives the moves it makes: [`Change::Moved`],
/// [`Change::FolderMoved`] and [`Change::TagMoved`]. A note that such a move explains gives
/// no other change.
///
/// The core finds no folder moves from other tools. Each note of a moved folder comes as
/// [`Change::Moved`] or as [`Change::Removed`] and [`Change::Added`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Change {
    /// A new note file.
    Added(NotePath),
    /// A note file whose text, info or links changed.
    Changed(NotePath),
    /// A note file that is gone. The path is where it was.
    Removed(NotePath),
    /// A note that is now at another path. From another tool, only a note with an id moves.
    Moved {
        /// The old path.
        from: NotePath,
        /// The new path.
        to: NotePath,
    },
    /// A folder that Holm moved, with everything below it.
    FolderMoved {
        /// The old path.
        from: FolderPath,
        /// The new path.
        to: FolderPath,
    },
    /// A tag that Holm renamed, with its child tags.
    TagMoved {
        /// The old name.
        from: TagName,
        /// The new name.
        to: TagName,
    },
    /// The list from [`Snapshot::folders`](crate::Snapshot::folders) changed.
    Folders,
    /// The nested tags from [`Snapshot::tags`](crate::Snapshot::tags) changed.
    Tags,
}

impl Change {
    /// Where the note at `path` is after this change, or `None` when the change does not move
    /// it.
    pub fn note_after(&self, path: &NotePath) -> Option<NotePath> {
        match self {
            Self::Moved { from, to } if from == path => Some(to.clone()),
            Self::FolderMoved { from, to } => path.moved(from, to),
            _ => None,
        }
    }

    /// Where the folder at `path` is after this change, or `None` when the change does not move
    /// it.
    pub fn folder_after(&self, path: &FolderPath) -> Option<FolderPath> {
        match self {
            Self::FolderMoved { from, to } => path.moved(from, to),
            _ => None,
        }
    }

    /// The name of `tag` after this change, when the change renames it or a parent tag.
    pub fn tag_after(&self, tag: &TagName) -> Option<TagName> {
        match self {
            Self::TagMoved { from, to } => tag.moved(from, to),
            _ => None,
        }
    }
}

/// The notes that match a search, from [`Core::search`](crate::Core::search), in the order
/// of a list, with the snapshot that knows each one.
#[derive(Clone, Debug, Default)]
pub struct SearchResults {
    snapshot: Snapshot,
    paths: Vec<NotePath>,
}

impl SearchResults {
    /// The notes at `paths` that `snapshot` has, in `order`.
    pub(crate) fn new(snapshot: Snapshot, paths: Vec<NotePath>, order: Order) -> Self {
        let paths = snapshot.sorted(paths, order);
        Self { snapshot, paths }
    }

    /// The snapshot that has a note at each path.
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// The paths of the notes, pinned notes on top.
    pub fn paths(&self) -> &[NotePath] {
        &self.paths
    }

    /// The snapshot and the paths, to keep them apart.
    pub fn into_parts(self) -> (Snapshot, Vec<NotePath>) {
        (self.snapshot, self.paths)
    }
}

/// Who made the changes of a [`ChangeList`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// A write through [`Core`](crate::Core).
    Holm,
    /// Another tool, seen by the watch.
    Outside,
}

/// The changes that made one revision of the snapshot, from [`ChangeLists`](crate::ChangeLists).
#[derive(Clone, Debug)]
pub struct ChangeList {
    /// Who made the changes.
    pub source: Source,
    /// The first snapshot that shows the changes.
    pub snapshot: Snapshot,
    /// The changes in order: first the moves of a write, then the note changes in path order,
    /// then [`Change::Folders`] and [`Change::Tags`]. It is never empty, and each new revision
    /// has one.
    pub changes: Vec<Change>,
}
