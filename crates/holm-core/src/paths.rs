//! The paths of notes and folders in the notes folder.
//!
//! Each path is relative to the notes folder and plain: each part is a name that is not empty
//! and does not start with `.`. So a path has no `.` or `..` part, no hidden part, no `//` and
//! no `/` at the end. The root folder is the empty path.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::path::Path;

use crate::error::{Error, NameProblem, Result};

/// The folder for pasted files at the root.
pub(crate) const ATTACHMENTS: &str = "attachments";
const NOTE_EXT: &str = ".md";
/// The longest file name that Linux file systems take.
pub(crate) const MAX_NAME_BYTES: usize = 255;

/// The path of a note: a plain path that ends in `.md`, like `work/Plans.md`.
///
/// Its order is the order of [`Path::cmp`].
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NotePath(Plain);

/// The path of a folder: any plain path, like `work/2026`. The root folder is the empty path.
///
/// Its order is the order of [`Path::cmp`].
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct FolderPath(Plain);

/// Any entry in the notes folder: a note, a folder, another file, or a path with nothing
/// there. The core checks the disk at a scope and below it.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub(crate) struct Scope(Plain);

impl NotePath {
    /// The path, or `None` when it is not a plain path that ends in `.md`.
    pub fn new(path: impl AsRef<Path>) -> Option<Self> {
        Plain::new(path.as_ref()).and_then(Self::from_plain)
    }

    /// The folder that holds the note.
    pub fn folder(&self) -> FolderPath {
        FolderPath(self.0.parent())
    }

    /// The file name without `.md`.
    pub fn title(&self) -> Cow<'_, str> {
        let name = self.0.name();
        String::from_utf8_lossy(&name[..name.len() - NOTE_EXT.len()])
    }

    /// The file name, with `.md`.
    pub fn name(&self) -> &OsStr {
        OsStr::from_bytes(self.0.name())
    }

    /// Whether the note is right in `folder`, not in one of its subfolders.
    pub fn is_in(&self, folder: &FolderPath) -> bool {
        self.0.is_in(&folder.0)
    }

    /// Whether the note is in `folder` or in one of its subfolders.
    pub fn is_below(&self, folder: &FolderPath) -> bool {
        self.0.is_below(&folder.0)
    }

    /// The path after the folder `from` moved to `to`, or `None` when the note is not below
    /// `from`.
    pub fn moved(&self, from: &FolderPath, to: &FolderPath) -> Option<Self> {
        self.0.moved(&from.0, &to.0).map(Self)
    }

    /// The folders that hold the note, the nearest first. The root is last.
    pub fn parents(&self) -> impl Iterator<Item = FolderPath> + use<> {
        self.0.parents().map(FolderPath)
    }

    pub(crate) fn scope(&self) -> Scope {
        Scope(self.0.clone())
    }

    fn from_plain(plain: Plain) -> Option<Self> {
        let name = plain.name();
        (name.len() > NOTE_EXT.len() && name.ends_with(NOTE_EXT.as_bytes())).then_some(Self(plain))
    }
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
        self.0.0.is_empty()
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

    /// The path of a new note with the typed `title` in this folder: `<title>.md`, with no
    /// spaces around `title`.
    ///
    /// # Errors
    ///
    /// [`Error::BadName`] when `title` is empty, holds `/`, starts with `.`, or is too long
    /// with `.md`.
    pub fn note(&self, title: &str) -> Result<NotePath> {
        let title = title.trim();
        let name = format!("{title}{NOTE_EXT}");
        check_name(title, &name)?;
        Ok(NotePath(self.0.join(name.as_bytes())))
    }

    /// The path of a new folder with the typed `name` in this folder, with no spaces around
    /// `name`.
    ///
    /// # Errors
    ///
    /// [`Error::BadName`] when `name` is empty, holds `/`, starts with `.`, is too long, or is
    /// `attachments` at the root.
    pub fn child(&self, name: &str) -> Result<Self> {
        let name = name.trim();
        if self.is_root() && name.eq_ignore_ascii_case(ATTACHMENTS) {
            return Err(Error::BadName {
                name: name.to_owned(),
                problem: NameProblem::Attachments,
            });
        }
        check_name(name, name)?;
        Ok(Self(self.0.join(name.as_bytes())))
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

    pub(crate) fn scope(&self) -> Scope {
        Scope(self.0.clone())
    }

    /// The note with the name `name` from the disk in this folder, or `None` when it is not a
    /// note name.
    pub(crate) fn note_named(&self, name: &OsStr) -> Option<NotePath> {
        Plain::is_part(name.as_bytes())
            .then(|| self.0.join(name.as_bytes()))
            .and_then(NotePath::from_plain)
    }

    /// The folder with the name `name` from the disk in this folder, or `None` when it is
    /// hidden.
    pub(crate) fn folder_named(&self, name: &OsStr) -> Option<Self> {
        Plain::is_part(name.as_bytes()).then(|| Self(self.0.join(name.as_bytes())))
    }
}

impl Scope {
    /// The scope, or `None` when it is not a plain path.
    pub(crate) fn new(path: &Path) -> Option<Self> {
        Plain::new(path).map(Self)
    }

    /// The whole notes folder.
    pub(crate) fn root() -> Self {
        Self(Plain::default())
    }

    /// The scope as a note path, when it is one.
    pub(crate) fn note(&self) -> Option<NotePath> {
        NotePath::from_plain(self.0.clone())
    }

    /// The scope as a folder path. It need not be a folder on disk.
    pub(crate) fn folder(&self) -> FolderPath {
        FolderPath(self.0.clone())
    }

    /// Whether `other` is this scope or below it.
    pub(crate) fn holds(&self, other: &Self) -> bool {
        self == other || other.0.is_below(&self.0)
    }

    /// Whether `folder` is this scope or below it.
    pub(crate) fn holds_folder(&self, folder: &FolderPath) -> bool {
        self.0 == folder.0 || folder.0.is_below(&self.0)
    }

    /// Whether this is a file path in `attachments/` at the root.
    pub(crate) fn is_attachment(&self) -> bool {
        self.0.is_below(&Plain(ATTACHMENTS.into()))
    }
}

/// The one form of all path types: the bytes of a plain path.
#[derive(Clone, PartialEq, Eq, Hash, Default)]
struct Plain(OsString);

impl Plain {
    fn new(path: &Path) -> Option<Self> {
        let bytes = path.as_os_str().as_bytes();
        let plain = bytes.is_empty() || bytes.split(|&byte| byte == b'/').all(Self::is_part);
        plain.then(|| Self(path.as_os_str().to_owned()))
    }

    /// Whether `name` can be one part of a plain path. A zero byte would break the order.
    fn is_part(name: &[u8]) -> bool {
        name.first().is_some_and(|&first| first != b'.')
            && !name.contains(&b'/')
            && !name.contains(&0)
    }

    fn bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// The path with one more part. `name` must follow [`Plain::is_part`].
    fn join(&self, name: &[u8]) -> Self {
        if self.0.is_empty() {
            return Self(OsString::from_vec(name.to_vec()));
        }
        let mut bytes = Vec::with_capacity(self.0.len() + 1 + name.len());
        bytes.extend_from_slice(self.bytes());
        bytes.push(b'/');
        bytes.extend_from_slice(name);
        Self(OsString::from_vec(bytes))
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
        Self(OsString::from_vec(bytes[..end].to_vec()))
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

/// The order of [`Path::cmp`] for plain paths. It compares bytes, with `/` as the lowest byte.
impl Ord for Plain {
    fn cmp(&self, other: &Self) -> Ordering {
        let key = |byte: u8| if byte == b'/' { 0 } else { byte };
        let (a, b) = (self.bytes(), other.bytes());
        match a.iter().zip(b).find(|(x, y)| x != y) {
            Some((&x, &y)) => key(x).cmp(&key(y)),
            None => a.len().cmp(&b.len()),
        }
    }
}

impl PartialOrd for Plain {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Debug for Plain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(Path::new(&self.0), f)
    }
}

impl AsRef<Path> for NotePath {
    fn as_ref(&self) -> &Path {
        Path::new(&self.0.0)
    }
}

impl AsRef<Path> for FolderPath {
    fn as_ref(&self) -> &Path {
        Path::new(&self.0.0)
    }
}

impl AsRef<Path> for Scope {
    fn as_ref(&self) -> &Path {
        Path::new(&self.0.0)
    }
}

impl fmt::Display for NotePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Path::new(&self.0.0).display().fmt(f)
    }
}

impl fmt::Display for FolderPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Path::new(&self.0.0).display().fmt(f)
    }
}

impl fmt::Debug for NotePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Debug for FolderPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The count of notes right in each folder. A folder with no notes is not in the map.
pub(crate) fn folder_counts<'a>(
    notes: impl Iterator<Item = &'a NotePath>,
) -> HashMap<FolderPath, usize> {
    let mut counts: HashMap<&[u8], usize> = HashMap::new();
    for note in notes {
        let bytes = note.0.bytes();
        let end = bytes.len() - note.0.name().len();
        *counts.entry(&bytes[..end.saturating_sub(1)]).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(folder, count)| {
            (
                FolderPath(Plain(OsStr::from_bytes(folder).to_owned())),
                count,
            )
        })
        .collect()
}

/// Checks the rules for a trimmed typed name. `file_name` is the name on disk for `typed`.
fn check_name(typed: &str, file_name: &str) -> Result<()> {
    let problem = if typed.is_empty() {
        NameProblem::Empty
    } else if typed.contains('/') {
        NameProblem::Slash
    } else if typed.starts_with('.') {
        NameProblem::Dot
    } else if file_name.len() > MAX_NAME_BYTES {
        NameProblem::TooLong
    } else {
        return Ok(());
    };
    Err(Error::BadName {
        name: typed.to_owned(),
        problem,
    })
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
        }
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
    fn a_plain_path_gives_a_value() {
        assert!(NotePath::new("a b/c.d.md").is_some());
        assert!(FolderPath::new("").is_some_and(|root| root.is_root()));
        assert!(FolderPath::new("a b/c.md").is_some());
        assert!(FolderPath::new("attachments").is_some());
    }

    #[test]
    fn note_and_child_give_a_name_problem_for_bad_names() {
        let root = FolderPath::root();
        let long = "a".repeat(MAX_NAME_BYTES - 2);
        assert_eq!(problem(root.note("  ")), NameProblem::Empty);
        assert_eq!(problem(root.note("a/b")), NameProblem::Slash);
        assert_eq!(problem(root.note(".a")), NameProblem::Dot);
        assert_eq!(problem(root.note(&long)), NameProblem::TooLong);
        assert_eq!(problem(root.child("")), NameProblem::Empty);
        assert_eq!(problem(root.child("a/b")), NameProblem::Slash);
        assert_eq!(problem(root.child("..")), NameProblem::Dot);
        assert_eq!(problem(root.child(&"a".repeat(256))), NameProblem::TooLong);
        assert_eq!(
            problem(root.child(" Attachments ")),
            NameProblem::Attachments
        );
    }

    #[test]
    fn note_and_child_trim_the_name() {
        let work = folder("work");
        assert_eq!(work.note(" Plans ").unwrap(), note("work/Plans.md"));
        assert_eq!(work.child(" 2026 ").unwrap(), folder("work/2026"));
        assert_eq!(
            work.child("attachments").unwrap(),
            folder("work/attachments")
        );
        assert_eq!(FolderPath::root().note("A").unwrap(), note("A.md"));
    }

    #[test]
    fn a_note_knows_its_folder_title_and_name() {
        let plans = note("work/2026/Plans.md");
        assert_eq!(plans.folder(), folder("work/2026"));
        assert_eq!(plans.title(), "Plans");
        assert_eq!(plans.name(), "Plans.md");
        assert_eq!(note("A.md").folder(), FolderPath::root());
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
    fn disk_names_skip_hidden_entries_and_other_files() {
        let work = folder("work");
        assert_eq!(work.note_named(OsStr::new("a.md")), Some(note("work/a.md")));
        assert_eq!(work.note_named(OsStr::new("a.txt")), None);
        assert_eq!(work.note_named(OsStr::new(".a.md")), None);
        assert_eq!(
            work.folder_named(OsStr::new(" x ")),
            Some(folder("work/ x "))
        );
        assert_eq!(work.folder_named(OsStr::new(".git")), None);
    }

    #[test]
    fn folder_counts_count_the_notes_right_in_each_folder() {
        let notes = [note("a.md"), note("b.md"), note("w/c.md"), note("w/x/d.md")];
        let counts = folder_counts(notes.iter());
        let expected = [
            (FolderPath::root(), 2),
            (folder("w"), 1),
            (folder("w/x"), 1),
        ];
        assert_eq!(counts, expected.into_iter().collect());
    }

    #[test]
    fn a_scope_holds_itself_and_what_is_below_it() {
        let scope = |path: &str| Scope::new(Path::new(path)).unwrap();
        assert!(scope("work").holds(&scope("work")));
        assert!(scope("work").holds(&scope("work/a.md")));
        assert!(!scope("work").holds(&scope("workshop")));
        assert!(Scope::root().holds(&scope("a")));
        assert!(scope("attachments/a.png").is_attachment());
        assert!(!scope("attachments").is_attachment());
        assert_eq!(scope("a/b.md").note(), Some(note("a/b.md")));
        assert_eq!(scope("a/b").note(), None);
    }
}
