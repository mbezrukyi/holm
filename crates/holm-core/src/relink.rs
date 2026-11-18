//! The links of a note and what they come from, and file link paths: where they point, and
//! how to write them after a note moves.

use std::hash::BuildHasher as _;
use std::path::{Component, Path, PathBuf};

use holm_markdown::{Body, Note as Text, Target};

use crate::model::{Link, LinkTarget, Note, NoteId};
use crate::{FolderPath, NotePath};

/// A hash of what the links of a note come from: its body and its folder. Two notes with the
/// same hash have the same links.
///
/// The hash may change with the `foldhash` version. Then each note gets parsed once more.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LinkSource(i64);

impl LinkSource {
    pub(crate) fn of(note: &Note) -> Self {
        let source = (&note.body, AsRef::<Path>::as_ref(note.info.path.folder()));
        let hash = foldhash::quality::FixedState::default().hash_one(source);
        Self(i64::from_ne_bytes(hash.to_ne_bytes()))
    }

    /// The hash from its stored form, [`LinkSource::stored`].
    pub(crate) fn from_stored(stored: i64) -> Self {
        Self(stored)
    }

    pub(crate) fn stored(self) -> i64 {
        self.0
    }
}

/// The links of the note, with file paths from the notes folder `root`.
pub(crate) fn links(root: &Path, note: &Note) -> Vec<Link> {
    let folder = note.info.path.folder();
    Body::parse(&note.body)
        .links()
        .iter()
        .map(|link| Link {
            target: match &link.target {
                Target::Note { id, heading } => LinkTarget::Note {
                    id: NoteId::from_field(id.clone()),
                    heading: heading.clone(),
                },
                Target::Heading(heading) => LinkTarget::Heading(heading.clone()),
                Target::File(file) => LinkTarget::File(resolve(root, folder, file)),
                Target::Web(address) => LinkTarget::Web(address.clone()),
            },
            embedded: link.embedded,
        })
        .collect()
}

/// The path of a file that a note in `folder` links as `file`.
///
/// It is relative to the notes folder `root` when the file is inside it, else absolute.
pub(crate) fn resolve(root: &Path, folder: &FolderPath, file: &str) -> PathBuf {
    let full = clean(&root.join(folder).join(file));
    match full.strip_prefix(root) {
        Ok(rel) => rel.to_owned(),
        Err(_) => full,
    }
}

/// The path for a link from the note `note` to the absolute path `file`.
///
/// A file inside the notes folder `root` gets a path from the folder of the note.
/// A file outside gets its absolute path. The path has no `.` and `..` parts.
/// It does not look at the disk.
pub(crate) fn link(root: &Path, note: &NotePath, file: &Path) -> PathBuf {
    let file = clean(file);
    if file.starts_with(root) {
        relative(&root.join(note.folder()), &file)
    } else {
        file
    }
}

/// A folder move: its old path and its new path, relative to the notes folder.
#[derive(Clone, Copy)]
pub(crate) struct Moved<'a> {
    pub(crate) from: &'a FolderPath,
    pub(crate) to: &'a FolderPath,
}

/// The note text with its relative file links fixed, after the note moved
/// from the folder `from` to the folder `to`.
///
/// With `moved`, the files inside that folder moved too.
/// A link that still points to the same file stays as it is.
/// It returns `None` when no link changes.
pub(crate) fn relink(
    text: &str,
    root: &Path,
    from: &FolderPath,
    to: &FolderPath,
    moved: Option<Moved<'_>>,
) -> Option<String> {
    let mut note = Text::parse(text);
    let body = note.body();
    let mut edits = Vec::new();
    for link in Body::parse(body).links() {
        let (Target::File(file), Some(range)) = (&link.target, &link.target_range) else {
            continue;
        };
        if Path::new(file).is_absolute() {
            continue;
        }
        let mut target = clean(&root.join(from).join(file));
        if let Some(moved) = moved
            && let Ok(rest) = target.strip_prefix(root.join(moved.from))
        {
            target = root.join(moved.to).join(rest);
        }
        if clean(&root.join(to).join(file)) == target {
            continue;
        }
        let path = relative(&root.join(to), &target);
        let Some(path) = path.to_str() else {
            tracing::warn!(path = %path.display(), "kept a file link that is not UTF-8");
            continue;
        };
        edits.push((range.clone(), destination(path)));
    }
    if edits.is_empty() {
        return None;
    }
    // A link can hold an embedded file, so the list is not always in text order.
    edits.sort_by_key(|(range, _)| range.start);
    let mut out = String::with_capacity(body.len());
    let mut last = 0;
    for (range, dest) in edits {
        out.push_str(&body[last..range.start]);
        out.push_str(&dest);
        last = range.end;
    }
    out.push_str(&body[last..]);
    note.set_body(&out);
    Some(note.to_string())
}

/// The path with no `.` and `..` parts. It does not look at the disk.
fn clean(path: &Path) -> PathBuf {
    let mut clean = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                clean.pop();
            }
            Component::CurDir => {}
            part => clean.push(part),
        }
    }
    clean
}

/// The path from the folder `base` to `target`. Both are absolute and clean.
fn relative(base: &Path, target: &Path) -> PathBuf {
    let base: Vec<_> = base.components().collect();
    let target: Vec<_> = target.components().collect();
    let common = base.iter().zip(&target).take_while(|(a, b)| a == b).count();
    let mut out = PathBuf::new();
    for _ in common..base.len() {
        out.push("..");
    }
    out.extend(&target[common..]);
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// A link destination for `path`. A path with spaces or brackets goes in `< >`.
fn destination(path: &str) -> String {
    let angle = path.contains(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '<' | '>'));
    let mut out = String::with_capacity(path.len() + 2);
    if angle {
        out.push('<');
    }
    for c in path.chars() {
        if c == '\\' || (angle && matches!(c, '<' | '>')) {
            out.push('\\');
        }
        out.push(c);
    }
    if angle {
        out.push('>');
    }
    out
}
