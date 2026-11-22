use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::hash::BuildHasher as _;
use std::path::Path;

use holm_markdown::{Body, NoteFile, OffsetRange, SourceOffset, Target};

use crate::model::{FileTarget, LinkTarget, Note, NoteId};
use crate::{AbsolutePath, FolderPath, LinkPath, NotePath, Result, Snapshot};

/// A hash of what the links of a note come from: its body and its folder. Two notes with the same hash have the same
/// links.
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

    pub(crate) fn from_stored(stored: i64) -> Self {
        Self(stored)
    }

    pub(crate) fn stored(self) -> i64 {
        self.0
    }
}

pub(crate) fn links(root: &AbsolutePath, note: &Note) -> Vec<LinkTarget> {
    let folder = note.info.path.folder();
    Body::parse(&note.body)
        .links()
        .iter()
        .map(|link| match &link.target {
            Target::Note { id, heading } => LinkTarget::Note {
                id: NoteId::from_field(id.clone()),
                heading: heading.clone(),
            },
            Target::Heading(heading) => LinkTarget::Heading(heading.clone()),
            Target::File(file) => LinkTarget::File(resolve(root, folder, file)),
            Target::Web(address) => LinkTarget::Web(address.clone()),
        })
        .collect()
}

pub(crate) fn headings(note: &Note) -> Vec<String> {
    Body::parse(&note.body)
        .headings()
        .iter()
        .map(|heading| heading.text.clone())
        .collect()
}

/// The note links in the note at `path` that point to a note or heading missing in `snapshot`, each once. A heading
/// link needs a heading with the same text, also in capital letters. `read` reads a linked note, once for each note.
pub(crate) fn broken(
    snapshot: &Snapshot,
    path: &NotePath,
    mut read: impl FnMut(&NotePath) -> Result<Note>,
) -> Result<Vec<LinkTarget>> {
    let mut known: HashMap<&NoteId, Option<Vec<String>>> = HashMap::new();
    let mut broken = Vec::new();
    for link in snapshot.links(path) {
        let LinkTarget::Note { id, heading } = link else {
            continue;
        };
        let targets = match known.entry(id) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let note = snapshot.note_with_id(id).map(|info| read(&info.path)).transpose()?;
                entry.insert(note.as_ref().map(headings))
            }
        };
        let found = match (targets, heading) {
            (None, _) => false,
            (Some(_), None) => true,
            (Some(targets), Some(heading)) => targets.contains(heading),
        };
        if !found && !broken.contains(link) {
            broken.push(link.clone());
        }
    }
    Ok(broken)
}

/// The file that a note in `folder` links as `file`, with the notes folder at `root`.
pub(crate) fn resolve(root: &AbsolutePath, folder: &FolderPath, file: &str) -> FileTarget {
    let full = root.join(folder).join(file).clean();
    match full.inside(root) {
        Some(inside) => FileTarget::Inside(inside),
        None => FileTarget::Outside(full),
    }
}

/// The path for a link from the note `note` to `file`.
///
/// A file inside the notes folder `root` gets a path from the folder of the note. A file outside gets its absolute
/// path. The path has no `.` and `..` parts. It does not look at the disk.
pub(crate) fn link(root: &AbsolutePath, note: &NotePath, file: &AbsolutePath) -> LinkPath {
    let file = file.clean();
    if file.starts_with(root) {
        root.join(note.folder()).link_to(&file)
    } else {
        LinkPath::from(file)
    }
}

/// Both paths are relative to the notes folder.
#[derive(Clone, Copy)]
pub(crate) struct FolderMove<'a> {
    pub(crate) from: &'a FolderPath,
    pub(crate) to: &'a FolderPath,
}

/// The note text with its relative file links fixed, after the note moved from the folder `from` to the folder `to`.
///
/// With `moved`, the files inside that folder moved too. A link that still points to the same file stays as it is. It
/// returns `None` when no link changes.
pub(crate) fn relink(
    text: &str,
    root: &AbsolutePath,
    from: &FolderPath,
    to: &FolderPath,
    moved: Option<FolderMove<'_>>,
) -> Option<String> {
    let mut note = NoteFile::parse(text);
    let body = note.body();
    let mut edits = Vec::new();
    for link in Body::parse(body).links() {
        let (Target::File(file), Some(range)) = (&link.target, &link.target_range) else {
            continue;
        };
        if Path::new(file).is_absolute() {
            continue;
        }
        let mut target = root.join(from).join(file).clean();
        if let Some(moved) = moved
            && let Ok(rest) = target.strip_prefix(root.join(moved.from))
        {
            target = root.join(moved.to).join(rest);
        }
        if root.join(to).join(file).clean() == target {
            continue;
        }
        let path = root.join(to).link_to(&target);
        let path = path.as_ref();
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
    let mut last = SourceOffset::default();
    for (range, dest) in edits {
        out.push_str(&body[(last..range.start).bytes()]);
        out.push_str(&dest);
        last = range.end;
    }
    out.push_str(&body[last.get()..]);
    note.set_body(&out);
    Some(note.to_string())
}

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
