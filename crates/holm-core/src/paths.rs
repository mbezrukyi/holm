//! The paths of notes and folders in the notes folder, and the names they are made of.
//!
//! Each path is relative to the notes folder and plain: each part is a [`Name`]. So a path has
//! no `.` or `..` part, no hidden part, no `//` and no `/` at the end. The root folder is the
//! empty path.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::{Error, NameProblem, Result};

const NOTE_EXT: &str = ".md";
/// The longest file name that Linux file systems take.
pub(crate) const MAX_NAME_BYTES: usize = 255;

/// The path of a note: a plain path that ends in `.md`, like `work/Plans.md`.
///
/// It holds its whole path and its folder. Notes of one folder can share the folder.
/// Its order is the order of [`Path::cmp`].
#[derive(Clone)]
pub struct NotePath {
    path: Arc<[u8]>,
    folder: FolderPath,
}

/// The path of a folder: any plain path, like `work/2026`. The root folder is the empty path.
///
/// A clone is cheap. Its order is the order of [`Path::cmp`].
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct FolderPath(Plain);

/// One part of a plain path: not empty, with no `/`, no leading `.`, no zero byte, and at most
/// [`MAX_NAME_BYTES`] bytes.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct Name(Arc<[u8]>);

/// A note or a folder that the core brings up to date with the disk. A folder node holds
/// everything at its path and below it.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Node {
    Note(NotePath),
    Folder(FolderPath),
}

impl Name {
    /// The name that a person typed, with no spaces around it.
    ///
    /// # Errors
    ///
    /// [`Error::BadName`] when the name is empty, holds `/` or a zero byte, starts with `.`,
    /// or is too long.
    pub(crate) fn typed(text: &str) -> Result<Self> {
        Self::checked(text.trim(), 0)
    }

    /// The name of a note with the typed `title`: the title with no spaces around it. Its file
    /// name adds `.md`.
    ///
    /// # Errors
    ///
    /// As in [`Name::typed`], and also when the title is too long with `.md`.
    pub(crate) fn title(text: &str) -> Result<Self> {
        Self::checked(text.trim(), NOTE_EXT.len())
    }

    /// A name from the disk, or `None` when it is no part of a plain path, like a hidden name.
    pub(crate) fn from_disk(name: &OsStr) -> Option<Self> {
        let name = name.as_bytes();
        is_part(name).then(|| Self(name.into()))
    }

    pub(crate) fn as_os_str(&self) -> &OsStr {
        OsStr::from_bytes(&self.0)
    }

    /// Checks a trimmed typed name. `more` is the count of bytes that its file name adds.
    fn checked(typed: &str, more: usize) -> Result<Self> {
        let problem = if typed.is_empty() {
            NameProblem::Empty
        } else if typed.contains('/') {
            NameProblem::Slash
        } else if typed.starts_with('.') {
            NameProblem::Dot
        } else if typed.contains('\0') {
            NameProblem::ZeroByte
        } else if typed.len() + more > MAX_NAME_BYTES {
            NameProblem::TooLong
        } else {
            return Ok(Self(typed.as_bytes().into()));
        };
        Err(Error::BadName {
            name: typed.to_owned(),
            problem,
        })
    }
}

impl NotePath {
    /// The path, or `None` when it is not a plain path that ends in `.md`.
    pub fn new(path: impl AsRef<Path>) -> Option<Self> {
        let bytes = path.as_ref().as_os_str().as_bytes();
        let (folder, _) = split(bytes)?;
        Self::from_bytes(FolderPath(Plain::from(folder)), bytes)
    }

    /// The folder that holds the note.
    pub fn folder(&self) -> &FolderPath {
        &self.folder
    }

    /// The file name without `.md`.
    pub fn title(&self) -> Cow<'_, str> {
        let name = self.name_bytes();
        String::from_utf8_lossy(&name[..name.len() - NOTE_EXT.len()])
    }

    /// The file name, with `.md`.
    pub fn name(&self) -> &OsStr {
        OsStr::from_bytes(self.name_bytes())
    }

    /// The note with the same name in `folder`.
    #[must_use]
    pub fn in_folder(&self, folder: &FolderPath) -> Self {
        Self::joined(folder, self.name_bytes())
    }

    /// Whether the note is right in `folder`, not in one of its subfolders.
    pub fn is_in(&self, folder: &FolderPath) -> bool {
        self.folder == *folder
    }

    /// Whether the note is in `folder` or in one of its subfolders.
    pub fn is_below(&self, folder: &FolderPath) -> bool {
        self.folder == *folder || self.folder.is_below(folder)
    }

    /// The path after the folder `from` moved to `to`, or `None` when the note is not below
    /// `from`.
    pub fn moved(&self, from: &FolderPath, to: &FolderPath) -> Option<Self> {
        self.folder
            .moved(from, to)
            .map(|folder| self.in_folder(&folder))
    }

    /// The folders that hold the note, the nearest first. The root is last.
    pub fn parents(&self) -> impl Iterator<Item = FolderPath> + use<> {
        std::iter::once(self.folder.clone()).chain(self.folder.parents())
    }

    /// The path of the note below the folder `root`.
    pub fn under(&self, root: &Path) -> PathBuf {
        root.join(self.as_path())
    }

    /// Whether `path` is the path of this note, relative to the notes folder.
    pub fn is_at(&self, path: &Path) -> bool {
        path.as_os_str().as_bytes() == &*self.path
    }

    /// The path relative to the notes folder.
    pub fn to_path(&self) -> PathBuf {
        self.as_path().to_owned()
    }

    /// The bytes of the path relative to the notes folder.
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.path
    }

    /// The file name, with `.md`.
    pub(crate) fn file(&self) -> Name {
        Name(self.name_bytes().into())
    }

    fn as_path(&self) -> &Path {
        Path::new(OsStr::from_bytes(&self.path))
    }

    fn name_bytes(&self) -> &[u8] {
        let folder = self.folder.0.bytes().len();
        &self.path[if folder == 0 { 0 } else { folder + 1 }..]
    }

    /// The note at `bytes`, a plain path in `folder`, or `None` when it does not end in `.md`.
    fn from_bytes(folder: FolderPath, bytes: &[u8]) -> Option<Self> {
        is_note_name(&bytes[folder.0.bytes().len()..]).then(|| Self {
            path: bytes.into(),
            folder,
        })
    }

    /// The note `name` in `folder`. `name` must be a note name.
    fn joined(folder: &FolderPath, name: &[u8]) -> Self {
        Self {
            path: folder.0.join(name).0,
            folder: folder.clone(),
        }
    }
}

/// Whether the last part of a path is the file name of a note. It may start with `/`.
fn is_note_name(name: &[u8]) -> bool {
    let name = name.strip_prefix(b"/").unwrap_or(name);
    name.len() > NOTE_EXT.len() && name.ends_with(NOTE_EXT.as_bytes())
}

impl FolderPath {
    /// The root folder.
    pub fn root() -> Self {
        Self::default()
    }

    /// The path, or `None` when it is not a plain path.
    pub fn new(path: impl AsRef<Path>) -> Option<Self> {
        Plain::new(path.as_ref()).map(Self)
    }

    /// Whether this is the root folder.
    pub fn is_root(&self) -> bool {
        self.0.bytes().is_empty()
    }

    /// The folder that holds this folder. The root has none.
    pub fn folder(&self) -> Option<Self> {
        (!self.is_root()).then(|| Self(self.0.parent()))
    }

    /// The last part of the path. The root has none.
    pub fn name(&self) -> Option<&OsStr> {
        (!self.is_root()).then(|| OsStr::from_bytes(self.0.name()))
    }

    /// How many folders down from the root this folder is. The root is 0.
    pub fn depth(&self) -> usize {
        if self.is_root() {
            0
        } else {
            self.0.bytes().split(|&byte| byte == b'/').count()
        }
    }

    /// Whether this folder is right in `folder`.
    pub fn is_in(&self, folder: &Self) -> bool {
        self.0.is_in(&folder.0)
    }

    /// Whether this folder is inside `folder`, at any depth. A folder is not below itself.
    pub fn is_below(&self, folder: &Self) -> bool {
        self.0.is_below(&folder.0)
    }

    /// The path after the folder `from` moved to `to`, or `None` when this folder is not
    /// `from` and not below it.
    pub fn moved(&self, from: &Self, to: &Self) -> Option<Self> {
        self.0.moved(&from.0, &to.0).map(Self)
    }

    /// The folders that hold this folder, the nearest first. The root is last, and has none.
    pub fn parents(&self) -> impl Iterator<Item = Self> + use<> {
        self.0.parents().map(Self)
    }

    /// The note with the title `title` in this folder: `<title>.md`.
    pub(crate) fn note(&self, title: &Name) -> NotePath {
        let mut name = title.0.to_vec();
        name.extend_from_slice(NOTE_EXT.as_bytes());
        NotePath::joined(self, &name)
    }

    /// The note with the file name `name` in this folder, or `None` when `name` does not
    /// end in `.md`.
    pub(crate) fn note_file(&self, name: &Name) -> Option<NotePath> {
        is_note_name(&name.0).then(|| NotePath::joined(self, &name.0))
    }

    /// The folder `name` in this folder.
    pub(crate) fn child(&self, name: &Name) -> Self {
        Self(self.0.join(&name.0))
    }

    /// The folder that holds this folder, and its name. The root has none.
    pub(crate) fn split(&self) -> Option<(Self, Name)> {
        (!self.is_root()).then(|| (Self(self.0.parent()), Name(self.0.name().into())))
    }
}

/// Reads many note paths, like the rows of a query. The notes of one folder share it.
#[derive(Default)]
pub(crate) struct NoteReader {
    folders: HashMap<Box<[u8]>, FolderPath, foldhash::fast::RandomState>,
}

impl NoteReader {
    /// The note path in `bytes`, or `None` as in [`NotePath::new`].
    pub(crate) fn note(&mut self, bytes: &[u8]) -> Option<NotePath> {
        let (folder, _) = split(bytes)?;
        let folder = if let Some(known) = self.folders.get(folder) {
            known.clone()
        } else {
            let new = FolderPath(Plain::from(folder));
            self.folders.insert(folder.into(), new.clone());
            new
        };
        NotePath::from_bytes(folder, bytes)
    }
}

/// The folder and the last part of a plain path, or `None` when it is not plain or is the
/// root.
fn split(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    if bytes.is_empty() || !is_plain(bytes) {
        return None;
    }
    Some(match bytes.iter().rposition(|&byte| byte == b'/') {
        Some(slash) => (&bytes[..slash], &bytes[slash + 1..]),
        None => (&b""[..], bytes),
    })
}

impl Node {
    /// Whether `other` is this node or below it.
    pub(crate) fn holds(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Folder(folder), Self::Note(note)) => note.is_below(folder),
            (Self::Folder(_), Self::Folder(folder)) => self.holds_folder(folder),
            (Self::Note(note), Self::Note(other)) => note == other,
            (Self::Note(_), Self::Folder(_)) => false,
        }
    }

    /// Whether `folder` is this node or below it.
    pub(crate) fn holds_folder(&self, folder: &FolderPath) -> bool {
        match self {
            Self::Folder(node) => node == folder || folder.is_below(node),
            Self::Note(_) => false,
        }
    }
}

/// Whether `name` can be one part of a plain path. A zero byte would break the order.
fn is_part(name: &[u8]) -> bool {
    name.first().is_some_and(|&first| first != b'.')
        && name.len() <= MAX_NAME_BYTES
        && !name.contains(&b'/')
        && !name.contains(&0)
}

/// Whether `bytes` is a plain path: the root, or names split by `/` that follow [`is_part`].
fn is_plain(bytes: &[u8]) -> bool {
    let mut start = 0;
    for (at, &byte) in bytes.iter().enumerate() {
        if at == start && (byte == b'.' || byte == b'/') {
            return false;
        }
        match byte {
            0 => return false,
            b'/' => start = at + 1,
            _ if at - start >= MAX_NAME_BYTES => return false,
            _ => {}
        }
    }
    bytes.is_empty() || start < bytes.len()
}

/// The bytes of a plain path. A clone is cheap.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Plain(Arc<[u8]>);

impl Plain {
    fn new(path: &Path) -> Option<Self> {
        let bytes = path.as_os_str().as_bytes();
        is_plain(bytes).then(|| Self::from(bytes))
    }

    fn bytes(&self) -> &[u8] {
        &self.0
    }

    /// The path with one more part. `name` must follow [`is_part`].
    fn join(&self, name: &[u8]) -> Self {
        if self.0.is_empty() {
            return Self::from(name);
        }
        let mut bytes = Vec::with_capacity(self.0.len() + 1 + name.len());
        bytes.extend_from_slice(self.bytes());
        bytes.push(b'/');
        bytes.extend_from_slice(name);
        Self(bytes.into())
    }

    /// The last part. The root gives an empty name.
    fn name(&self) -> &[u8] {
        let bytes = self.bytes();
        bytes
            .iter()
            .rposition(|&byte| byte == b'/')
            .map_or(bytes, |slash| &bytes[slash + 1..])
    }

    /// The path without its last part. The root gives the root.
    fn parent(&self) -> Self {
        let bytes = self.bytes();
        let end = bytes.iter().rposition(|&byte| byte == b'/').unwrap_or(0);
        Self::from(&bytes[..end])
    }

    fn parents(&self) -> impl Iterator<Item = Self> + use<> {
        let start = (!self.0.is_empty()).then(|| self.parent());
        std::iter::successors(start, |plain| (!plain.0.is_empty()).then(|| plain.parent()))
    }

    /// Whether the path is right in `folder`.
    fn is_in(&self, folder: &Self) -> bool {
        let rest = self.rest(folder);
        rest.is_some_and(|rest| !rest.contains(&b'/'))
    }

    /// Whether the path is inside `folder`, at any depth.
    fn is_below(&self, folder: &Self) -> bool {
        self.rest(folder).is_some()
    }

    /// The part of the path below `folder`, or `None` when it is not below it.
    fn rest(&self, folder: &Self) -> Option<&[u8]> {
        let (path, folder) = (self.bytes(), folder.bytes());
        if folder.is_empty() {
            return (!path.is_empty()).then_some(path);
        }
        path.strip_prefix(folder)?.strip_prefix(b"/")
    }

    fn moved(&self, from: &Self, to: &Self) -> Option<Self> {
        if self == from {
            return Some(to.clone());
        }
        self.rest(from).map(|rest| to.join(rest))
    }
}

impl Default for Plain {
    fn default() -> Self {
        Self::from(&b""[..])
    }
}

impl From<&[u8]> for Plain {
    fn from(bytes: &[u8]) -> Self {
        Self(bytes.into())
    }
}

/// The key of a byte in the order of [`Path::cmp`] for plain paths: `/` is the lowest byte.
fn key(byte: u8) -> u8 {
    if byte == b'/' { 0 } else { byte }
}

/// Where `a` and `b` differ first, within the shorter of them.
fn mismatch(a: &[u8], b: &[u8]) -> Option<usize> {
    a.iter().zip(b).position(|(x, y)| x != y)
}

/// The order of [`Path::cmp`] for plain paths.
fn compare(a: &[u8], b: &[u8]) -> Ordering {
    match mismatch(a, b) {
        Some(at) => key(a[at]).cmp(&key(b[at])),
        None => a.len().cmp(&b.len()),
    }
}

impl Ord for Plain {
    fn cmp(&self, other: &Self) -> Ordering {
        compare(self.bytes(), other.bytes())
    }
}

impl PartialOrd for Plain {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for NotePath {
    fn cmp(&self, other: &Self) -> Ordering {
        compare(&self.path, &other.path)
    }
}

impl PartialEq for NotePath {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl Eq for NotePath {}

impl Hash for NotePath {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.path.hash(state);
    }
}

impl PartialOrd for NotePath {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Debug for Plain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(Path::new(OsStr::from_bytes(self.bytes())), f)
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_os_str(), f)
    }
}

impl AsRef<Path> for FolderPath {
    fn as_ref(&self) -> &Path {
        Path::new(OsStr::from_bytes(self.0.bytes()))
    }
}

impl fmt::Display for NotePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_path().display().fmt(f)
    }
}

impl fmt::Display for FolderPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        AsRef::<Path>::as_ref(self).display().fmt(f)
    }
}

impl fmt::Debug for NotePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.to_path(), f)
    }
}

impl fmt::Debug for FolderPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use std::path::Path;

    use proptest::prelude::*;

    use super::*;

    /// A plain path with 1 to 3 parts.
    fn plain() -> impl Strategy<Value = String> {
        let part = "[ !#+,.a-bA-Bé-]{1,4}".prop_filter("not hidden", |part| !part.starts_with('.'));
        prop::collection::vec(part, 1..4).prop_map(|parts| parts.join("/"))
    }

    fn folder(path: &str) -> FolderPath {
        FolderPath::new(path).unwrap()
    }

    fn note(path: &str) -> NotePath {
        NotePath::new(path).unwrap()
    }

    fn problem<T: fmt::Debug>(result: Result<T>) -> NameProblem {
        match result {
            Err(Error::BadName { problem, .. }) => problem,
            other => panic!("not a bad name: {other:?}"),
        }
    }

    proptest! {
        #[test]
        fn the_order_is_the_order_of_path_cmp(a in plain(), b in plain()) {
            prop_assert_eq!(folder(&a).cmp(&folder(&b)), Path::new(&a).cmp(Path::new(&b)));
            let (a, b) = (format!("{a}.md"), format!("{b}.md"));
            prop_assert_eq!(note(&a).cmp(&note(&b)), Path::new(&a).cmp(Path::new(&b)));
        }

        #[test]
        fn the_order_is_the_order_of_path_cmp_with_a_shared_start(
            start in plain(),
            a in plain(),
            b in plain(),
        ) {
            let (a, b) = (format!("{start}{a}"), format!("{start}/{b}"));
            prop_assert_eq!(folder(&a).cmp(&folder(&b)), Path::new(&a).cmp(Path::new(&b)));
            let (a, b) = (format!("{a}.md"), format!("{b}.md"));
            prop_assert_eq!(note(&a).cmp(&note(&b)), Path::new(&a).cmp(Path::new(&b)));
            prop_assert_eq!(note(&b).cmp(&note(&a)), Path::new(&b).cmp(Path::new(&a)));
        }
    }

    #[test]
    fn a_deeper_note_can_come_first() {
        assert!(note("a/b/c.md") < note("a/b.md"));
        assert!(Path::new("a/b/c.md") < Path::new("a/b.md"));
    }

    #[test]
    fn is_at_compares_the_whole_path() {
        let plans = note("work/Plans.md");
        assert!(plans.is_at(Path::new("work/Plans.md")));
        assert!(!plans.is_at(Path::new("workPlans.md")));
        assert!(!plans.is_at(Path::new("work/x/Plans.md")));
        assert!(!plans.is_at(Path::new("/work/Plans.md")));
        assert!(note("A.md").is_at(Path::new("A.md")));
        assert!(!note("A.md").is_at(Path::new("x/A.md")));
    }

    #[test]
    fn in_folder_keeps_the_name() {
        let plans = note("work/Plans.md");
        assert_eq!(plans.in_folder(&folder("home")), note("home/Plans.md"));
        assert_eq!(plans.in_folder(&FolderPath::root()), note("Plans.md"));
    }

    #[test]
    fn a_bad_path_gives_no_value() {
        let bad = [
            "/a.md",
            "./a.md",
            "a/./b.md",
            "../a.md",
            "a/../b.md",
            "a//b.md",
            "a.md/",
            ".a.md",
            "a/.b/c.md",
            ".md",
            "a/.md",
            "a\0b.md",
        ];
        for path in bad {
            assert_eq!(NotePath::new(path), None, "{path:?}");
            assert_eq!(FolderPath::new(path), None, "{path:?}");
        }
        assert_eq!(NotePath::new(""), None);
        assert_eq!(NotePath::new("a/b.txt"), None);
        assert_eq!(NotePath::new("a.md.txt"), None);
    }

    #[test]
    fn a_part_has_at_most_255_bytes() {
        let part = |len: usize| "a".repeat(len);
        assert!(FolderPath::new(format!("x/{}", part(255))).is_some());
        assert!(FolderPath::new(format!("x/{}/y", part(256))).is_none());
        assert!(NotePath::new(format!("{}.md", part(252))).is_some());
        assert!(NotePath::new(format!("{}.md", part(253))).is_none());
    }

    #[test]
    fn a_plain_path_gives_a_value() {
        assert!(NotePath::new("a b/c.d.md").is_some());
        assert!(FolderPath::new("").is_some_and(|root| root.is_root()));
        assert!(FolderPath::new("a b/c.md").is_some());
        assert!(FolderPath::new("attachments").is_some());
    }

    #[test]
    fn a_note_knows_its_folder_title_and_name() {
        let plans = note("work/2026/Plans.md");
        assert_eq!(*plans.folder(), folder("work/2026"));
        assert_eq!(plans.title(), "Plans");
        assert_eq!(plans.name(), "Plans.md");
        assert_eq!(*note("A.md").folder(), FolderPath::root());
        let parents: Vec<_> = plans.parents().collect();
        assert_eq!(
            parents,
            [folder("work/2026"), folder("work"), FolderPath::root()]
        );
    }

    #[test]
    fn a_folder_knows_its_folder_name_depth_and_parents() {
        let year = folder("work/2026");
        assert_eq!(year.folder(), Some(folder("work")));
        assert_eq!(folder("work").folder(), Some(FolderPath::root()));
        assert_eq!(FolderPath::root().folder(), None);
        assert_eq!(year.name(), Some(OsStr::new("2026")));
        assert_eq!(FolderPath::root().name(), None);
        assert_eq!(year.depth(), 2);
        assert_eq!(FolderPath::root().depth(), 0);
        let parents: Vec<_> = year.parents().collect();
        assert_eq!(parents, [folder("work"), FolderPath::root()]);
        assert_eq!(FolderPath::root().parents().count(), 0);
    }

    #[test]
    fn in_and_below_compare_whole_parts() {
        let work = folder("work");
        assert!(note("work/a.md").is_in(&work));
        assert!(!note("work/x/a.md").is_in(&work));
        assert!(note("work/x/a.md").is_below(&work));
        assert!(!note("workshop/a.md").is_below(&work));
        assert!(note("a.md").is_in(&FolderPath::root()));
        assert!(folder("work/x").is_in(&work));
        assert!(!work.is_below(&work));
        assert!(work.is_below(&FolderPath::root()));
        assert!(!FolderPath::root().is_below(&FolderPath::root()));
    }

    #[test]
    fn moved_follows_a_folder_move() {
        let (from, to) = (folder("work"), folder("old/job"));
        assert_eq!(
            note("work/x/a.md").moved(&from, &to),
            Some(note("old/job/x/a.md"))
        );
        assert_eq!(note("workshop/a.md").moved(&from, &to), None);
        assert_eq!(from.moved(&from, &to), Some(to.clone()));
        assert_eq!(
            folder("work/x").moved(&from, &to),
            Some(folder("old/job/x"))
        );
        assert_eq!(
            note("a/b.md").moved(&folder("a"), &FolderPath::root()),
            Some(note("b.md"))
        );
    }

    #[test]
    fn typed_names_give_each_name_problem() {
        let long = "a".repeat(MAX_NAME_BYTES - 2);
        assert_eq!(problem(Name::typed("  ")), NameProblem::Empty);
        assert_eq!(problem(Name::typed("a/b")), NameProblem::Slash);
        assert_eq!(problem(Name::typed("..")), NameProblem::Dot);
        assert_eq!(problem(Name::typed("a\0b")), NameProblem::ZeroByte);
        assert_eq!(problem(Name::typed(&"a".repeat(256))), NameProblem::TooLong);
        assert!(Name::typed(&long).is_ok());
        assert_eq!(problem(Name::title(&long)), NameProblem::TooLong);
        assert_eq!(problem(Name::title(".a")), NameProblem::Dot);
    }

    #[test]
    fn typed_names_lose_the_spaces_around_them() {
        let work = folder("work");
        let title = Name::title(" Plans ").unwrap();
        assert_eq!(work.note(&title), note("work/Plans.md"));
        assert_eq!(
            work.child(&Name::typed(" 2026 ").unwrap()),
            folder("work/2026")
        );
        let attachments = Name::typed("attachments").unwrap();
        assert_eq!(
            FolderPath::root().child(&attachments),
            folder("attachments")
        );
    }

    #[test]
    fn disk_names_skip_hidden_names_only() {
        let disk = |name: &str| Name::from_disk(OsStr::new(name));
        let work = folder("work");
        assert_eq!(work.child(&disk(" x ").unwrap()), folder("work/ x "));
        assert_eq!(
            work.child(&disk("attachments").unwrap()),
            folder("work/attachments")
        );
        assert_eq!(
            work.note_file(&disk("a.md").unwrap()),
            Some(note("work/a.md"))
        );
        assert_eq!(work.note_file(&disk("a.txt").unwrap()), None);
        assert_eq!(disk(".a.md"), None);
        assert_eq!(disk(".git"), None);
        assert_eq!(disk(""), None);
        assert_eq!(disk(&"a".repeat(256)), None);
    }

    #[test]
    fn a_folder_splits_into_its_folder_and_name() {
        let (parent, name) = folder("work/2026").split().unwrap();
        assert_eq!(parent, folder("work"));
        assert_eq!(parent.child(&name), folder("work/2026"));
        assert!(FolderPath::root().split().is_none());
    }

    #[test]
    fn a_folder_node_holds_itself_and_what_is_below_it() {
        let work = Node::Folder(folder("work"));
        assert!(work.holds(&work));
        assert!(work.holds(&Node::Note(note("work/a.md"))));
        assert!(work.holds(&Node::Folder(folder("work/x"))));
        assert!(!work.holds(&Node::Folder(folder("workshop"))));
        assert!(Node::Folder(FolderPath::root()).holds(&Node::Note(note("a.md"))));
        let a = Node::Note(note("a.md"));
        assert!(a.holds(&a));
        assert!(!a.holds(&Node::Folder(folder("a.md"))));
    }
}
