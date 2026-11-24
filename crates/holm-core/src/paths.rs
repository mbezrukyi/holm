//! A [`NotePath`] or a [`FolderPath`] is relative to the notes folder and plain: each part is a [`Name`]. So it has
//! no `.` or `..` part, no hidden part, no `//` and no `/` at the end. The root folder is the empty path.
//!
//! Paths of other files are an [`AbsolutePath`], a [`RawPath`] or a [`LinkPath`].

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::io;
use std::ops::Deref;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::error::{Error, NameProblem, Result};

const NOTE_EXT: &str = ".md";
/// The longest file name that Linux file systems take.
pub(crate) const MAX_NAME_BYTES: usize = 255;

/// A plain path that ends in `.md`, like `work/Plans.md`.
///
/// Notes of one folder can share the folder. Its order is the order of [`Path::cmp`].
#[derive(Clone)]
pub struct NotePath {
    path: Arc<[u8]>,
    folder: FolderPath,
}

/// Any plain path, like `work/2026`. The root folder is the empty path.
///
/// A clone is cheap. Its order is the order of [`Path::cmp`].
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct FolderPath(Plain);

/// One part of a plain path: not empty, with no `/`, no leading `.`, no zero byte, and at most [`MAX_NAME_BYTES`]
/// bytes.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct Name(Arc<[u8]>);

impl Name {
    /// The typed `text` with no spaces around it.
    pub(crate) fn typed(text: &str) -> Result<Self> {
        Self::checked(text.trim(), 0)
    }

    /// The typed note title `text` with no spaces around it. [`NameProblem::TooLong`] when its file name with `.md` is
    /// too long.
    pub(crate) fn title(text: &str) -> Result<Self> {
        Self::checked(text.trim(), NOTE_EXT.len())
    }

    /// `None` when `name` is no part of a plain path, like a hidden name.
    pub(crate) fn from_disk(name: &OsStr) -> Option<Self> {
        let name = name.as_bytes();
        is_part(name).then(|| Self(name.into()))
    }

    pub(crate) fn as_os_str(&self) -> &OsStr {
        OsStr::from_bytes(&self.0)
    }

    /// `typed` must be trimmed.
    fn checked(typed: &str, file_name_extra_bytes: usize) -> Result<Self> {
        let problem = if typed.is_empty() {
            NameProblem::Empty
        } else if typed.contains('/') {
            NameProblem::Slash
        } else if typed.starts_with('.') {
            NameProblem::Dot
        } else if typed.contains('\0') {
            NameProblem::ZeroByte
        } else if typed.len() + file_name_extra_bytes > MAX_NAME_BYTES {
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
    pub fn new(path: impl AsRef<Path>) -> Option<Self> {
        let bytes = path.as_ref().as_os_str().as_bytes();
        let (folder, _) = split(bytes)?;
        Self::from_bytes(FolderPath(Plain::from(folder)), bytes)
    }

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

    #[must_use]
    pub fn in_folder(&self, folder: &FolderPath) -> Self {
        Self::joined(folder, self.name_bytes())
    }

    /// Right in `folder`, not in one of its subfolders.
    pub fn is_in(&self, folder: &FolderPath) -> bool {
        self.folder == *folder
    }

    /// In `folder` or in one of its subfolders.
    pub fn is_below(&self, folder: &FolderPath) -> bool {
        self.folder == *folder || self.folder.is_below(folder)
    }

    /// The path after the folder `from` moved to `to`, or `None` when the note is not below `from`.
    pub fn moved(&self, from: &FolderPath, to: &FolderPath) -> Option<Self> {
        self.folder.moved(from, to).map(|folder| self.in_folder(&folder))
    }

    /// The nearest first. The root is last.
    pub fn parents(&self) -> impl Iterator<Item = FolderPath> + use<> {
        std::iter::once(self.folder.clone()).chain(self.folder.parents())
    }

    /// The file name, with `.md`.
    pub(crate) fn file(&self) -> Name {
        Name(self.name_bytes().into())
    }

    fn name_bytes(&self) -> &[u8] {
        let folder = self.folder.0.bytes().len();
        &self.path[if folder == 0 { 0 } else { folder + 1 }..]
    }

    /// `bytes` must be a plain path in `folder`. `None` when it does not end in `.md`.
    fn from_bytes(folder: FolderPath, bytes: &[u8]) -> Option<Self> {
        is_note_name(&bytes[folder.0.bytes().len()..]).then(|| Self {
            path: bytes.into(),
            folder,
        })
    }

    /// `name` must be a note name.
    fn joined(folder: &FolderPath, name: &[u8]) -> Self {
        Self {
            path: folder.0.join(name).0,
            folder: folder.clone(),
        }
    }
}

/// `name` is the last part of a path, and may start with `/`.
fn is_note_name(name: &[u8]) -> bool {
    let name = name.strip_prefix(b"/").unwrap_or(name);
    name.len() > NOTE_EXT.len() && name.ends_with(NOTE_EXT.as_bytes())
}

impl FolderPath {
    pub fn root() -> Self {
        Self::default()
    }

    pub fn new(path: impl AsRef<Path>) -> Option<Self> {
        Plain::new(path.as_ref()).map(Self)
    }

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

    /// `2026` for `work/2026`. The root gives an empty text.
    pub fn last_part(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(if self.is_root() { b"" } else { self.0.name() })
    }

    /// How many folders down from the root this folder is. The root is 0.
    pub fn depth(&self) -> usize {
        if self.is_root() {
            0
        } else {
            self.0.bytes().split(|&byte| byte == b'/').count()
        }
    }

    /// Right in `folder`, not in one of its subfolders.
    pub fn is_in(&self, folder: &Self) -> bool {
        self.0.is_in(&folder.0)
    }

    /// At any depth. A folder is not below itself.
    pub fn is_below(&self, folder: &Self) -> bool {
        self.0.is_below(&folder.0)
    }

    /// The root can't move, and no folder can move into itself or a folder below it.
    pub fn can_move_into(&self, parent: &Self) -> bool {
        !self.is_root() && parent != self && !parent.is_below(self)
    }

    /// The path after the folder `from` moved to `to`, or `None` when this folder is not `from` and not below it.
    pub fn moved(&self, from: &Self, to: &Self) -> Option<Self> {
        self.0.moved(&from.0, &to.0).map(Self)
    }

    /// The nearest first. The root is last, and has none.
    pub fn parents(&self) -> impl Iterator<Item = Self> + use<> {
        self.0.parents().map(Self)
    }

    /// `<title>.md` in this folder.
    pub(crate) fn note(&self, title: &Name) -> NotePath {
        let mut name = title.0.to_vec();
        name.extend_from_slice(NOTE_EXT.as_bytes());
        NotePath::joined(self, &name)
    }

    /// `None` when `name` does not end in `.md`.
    pub(crate) fn note_file(&self, name: &Name) -> Option<NotePath> {
        is_note_name(&name.0).then(|| NotePath::joined(self, &name.0))
    }

    pub(crate) fn child(&self, name: &Name) -> Self {
        Self(self.0.join(&name.0))
    }

    /// The folder that holds this folder, and its name. The root has none.
    pub(crate) fn split(&self) -> Option<(Self, Name)> {
        (!self.is_root()).then(|| (Self(self.0.parent()), Name(self.0.name().into())))
    }

    /// The folder and the last part of `path`, or `None` when `path` is empty or not plain.
    pub(crate) fn split_raw(path: &RawPath) -> Option<(Self, Name)> {
        let (folder, name) = split(path.0.as_os_str().as_bytes())?;
        Some((Self(Plain::from(folder)), Name(name.into())))
    }
}

/// An absolute path, like the notes folder or a linked file outside it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AbsolutePath(PathBuf);

/// A path from the notes folder that is not checked to be plain like a [`FolderPath`], like the target of a file link
/// or a path from the watch. Each part is a normal part, so it has no `.` or `..` part and no `/` at the start. The empty path
/// is the notes folder.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RawPath(PathBuf);

/// The path that a file link in a note holds: from the folder of the note, or absolute.
#[derive(Debug)]
pub struct LinkPath(PathBuf);

impl AbsolutePath {
    /// `None` when `path` is relative.
    pub fn new(path: impl Into<PathBuf>) -> Option<Self> {
        let path = path.into();
        path.is_absolute().then_some(Self(path))
    }

    /// The path with no symlinks and no `.` and `..` parts. The file must exist.
    pub(crate) fn canonical(path: &Path) -> io::Result<Self> {
        std::fs::canonicalize(path).map(Self)
    }

    #[must_use]
    pub(crate) fn join(&self, path: impl AsRef<Path>) -> Self {
        Self(self.0.join(path))
    }

    /// The path with no `.` and `..` parts. It does not look at the disk.
    #[must_use]
    pub(crate) fn clean(&self) -> Self {
        let mut clean = PathBuf::new();
        for part in self.0.components() {
            match part {
                Component::ParentDir => {
                    clean.pop();
                }
                Component::CurDir => {}
                part => clean.push(part),
            }
        }
        Self(clean)
    }

    /// The path from the folder at this path to `target`. Both are clean.
    pub(crate) fn link_to(&self, target: &Self) -> LinkPath {
        let base: Vec<_> = self.0.components().collect();
        let target: Vec<_> = target.0.components().collect();
        let common = base.iter().zip(&target).take_while(|(a, b)| a == b).count();
        let mut out = PathBuf::new();
        for _ in common..base.len() {
            out.push("..");
        }
        out.extend(&target[common..]);
        if out.as_os_str().is_empty() {
            out.push(".");
        }
        LinkPath(out)
    }

    /// The path from `folder` to this path, when this path is inside it.
    pub(crate) fn inside(&self, folder: &Self) -> Option<RawPath> {
        self.0.strip_prefix(&folder.0).ok().and_then(RawPath::new)
    }
}

impl RawPath {
    /// `None` when a part of `path` is not a normal part, as in an absolute path or a path with `..`.
    pub fn new(path: impl Into<PathBuf>) -> Option<Self> {
        let path = path.into();
        path.components()
            .all(|part| matches!(part, Component::Normal(_)))
            .then_some(Self(path))
    }

    /// The notes folder.
    pub(crate) fn root() -> Self {
        Self(PathBuf::new())
    }

    pub(crate) fn is_root(&self) -> bool {
        self.0.as_os_str().is_empty()
    }
}

impl From<AbsolutePath> for LinkPath {
    fn from(path: AbsolutePath) -> Self {
        Self(path.0)
    }
}

impl From<AbsolutePath> for PathBuf {
    fn from(path: AbsolutePath) -> Self {
        path.0
    }
}

impl Deref for AbsolutePath {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for AbsolutePath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for RawPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for LinkPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

/// Reads many note paths, like the rows of a query. The notes of one folder share it.
#[derive(Default)]
pub(crate) struct NoteReader {
    folders: HashMap<Box<[u8]>, FolderPath, foldhash::fast::RandomState>,
}

impl NoteReader {
    /// `None` when `bytes` is not a plain path that ends in `.md`.
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

/// The folder and the last part of a plain path, or `None` when it is not plain or is the root.
fn split(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    if bytes.is_empty() || !is_plain(bytes) {
        return None;
    }
    Some(match bytes.iter().rposition(|&byte| byte == b'/') {
        Some(slash) => (&bytes[..slash], &bytes[slash + 1..]),
        None => (&b""[..], bytes),
    })
}

/// A zero byte would break the order.
fn is_part(name: &[u8]) -> bool {
    name.first().is_some_and(|&first| first != b'.')
        && name.len() <= MAX_NAME_BYTES
        && !name.contains(&b'/')
        && !name.contains(&0)
}

/// The root, or names split by `/` that follow [`is_part`].
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

/// The bytes of a plain path.
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

    /// `name` must follow [`is_part`].
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

    fn is_in(&self, folder: &Self) -> bool {
        let rest = self.part_below(folder);
        rest.is_some_and(|rest| !rest.contains(&b'/'))
    }

    fn is_below(&self, folder: &Self) -> bool {
        self.part_below(folder).is_some()
    }

    fn part_below(&self, folder: &Self) -> Option<&[u8]> {
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
        self.part_below(from).map(|rest| to.join(rest))
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

/// In the order of [`Path::cmp`] for plain paths, `/` is the lowest byte.
fn key(byte: u8) -> u8 {
    if byte == b'/' { 0 } else { byte }
}

fn first_difference(a: &[u8], b: &[u8]) -> Option<usize> {
    a.iter().zip(b).position(|(x, y)| x != y)
}

/// The order of [`Path::cmp`] for plain paths.
fn compare(a: &[u8], b: &[u8]) -> Ordering {
    match first_difference(a, b) {
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

impl AsRef<Path> for NotePath {
    fn as_ref(&self) -> &Path {
        Path::new(OsStr::from_bytes(&self.path))
    }
}

impl AsRef<Path> for FolderPath {
    fn as_ref(&self) -> &Path {
        Path::new(OsStr::from_bytes(self.0.bytes()))
    }
}

impl fmt::Display for NotePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        AsRef::<Path>::as_ref(self).display().fmt(f)
    }
}

impl fmt::Display for FolderPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        AsRef::<Path>::as_ref(self).display().fmt(f)
    }
}

impl fmt::Debug for NotePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(AsRef::<Path>::as_ref(self), f)
    }
}

impl fmt::Debug for FolderPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests;
