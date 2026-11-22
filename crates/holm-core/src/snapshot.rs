use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::OsStr;
use std::path::PathBuf;
use std::sync::Arc;

use crate::model::{Link, LinkTarget, NoteId, NoteInfo, NoteSet, Order, Tag};
use crate::nodes::Node;
use crate::paths::{FolderPath, NotePath};
use crate::tag::{self, TagName};

/// The facts of the notes folder at one moment, from [`Core::snapshot`](crate::Core::snapshot). It never changes.
///
/// It is cheap to clone.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    revision: Revision,
    notes: Map<NotePath, Arc<NoteData>>,
    ids: Map<NoteId, NotePath>,
    folders: imbl::OrdSet<FolderPath>,
}

/// A number that grows with each new snapshot.
///
/// Two snapshots with the same revision hold the same facts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Revision(u64);

impl Revision {
    /// The revision after this one.
    #[must_use]
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// The paths come from file names on disk, so the hasher has a random seed.
type Map<K, V> = imbl::GenericHashMap<K, V, foldhash::fast::RandomState, imbl::shared_ptr::DefaultSharedPtr>;

#[derive(Debug)]
pub(crate) struct Removal {
    pub(crate) notes: usize,
    /// The file link targets that the removed notes link and no other note links, relative to the notes folder, in
    /// byte order. They are outside input.
    pub(crate) files: Vec<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
struct NoteData {
    info: NoteInfo,
    links: Vec<Link>,
}

impl Snapshot {
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Pinned notes are on top.
    ///
    /// Only `*.md` files are notes. Hidden files and folders, and symlinks, are skipped. A note that can't be read is
    /// left out.
    pub fn notes(&self, set: NoteSet<'_>, order: Order) -> Vec<NoteInfo> {
        let mut notes: Vec<&NoteInfo> = self.matching(set).collect();
        sort(&mut notes, order);
        notes.into_iter().cloned().collect()
    }

    /// The newest change first. Pins have no effect.
    pub fn newest(&self, set: NoteSet<'_>) -> Vec<NoteInfo> {
        let mut notes: Vec<&NoteInfo> = self.matching(set).collect();
        notes.sort_unstable_by(|a, b| newest(a, b));
        notes.into_iter().cloned().collect()
    }

    /// Pinned notes are on top.
    pub fn paths(&self, set: NoteSet<'_>, order: Order) -> Vec<NotePath> {
        let mut notes: Vec<&NoteInfo> = self.matching(set).collect();
        sort(&mut notes, order);
        notes.into_iter().map(|note| note.path.clone()).collect()
    }

    /// In no fixed order.
    pub fn matching<'s, 'a>(&'s self, set: NoteSet<'a>) -> impl Iterator<Item = &'s NoteInfo> + use<'s, 'a> {
        self.notes
            .values()
            .filter(move |entry| set.holds(&entry.info))
            .map(|entry| &entry.info)
    }

    pub fn first(&self, set: NoteSet<'_>, order: Order) -> Option<&NoteInfo> {
        match order {
            Order::LastChange => self.matching(set).min_by(|a, b| last_change(a, b)),
            Order::Title => self
                .matching(set)
                .map(|note| (title_key(note), note))
                .min_by(by_title)
                .map(|(_, note)| note),
        }
    }

    pub fn note_or_first(&self, path: Option<&NotePath>, order: Order) -> Option<&NoteInfo> {
        path.and_then(|path| self.note(path))
            .or_else(|| self.first(NoteSet::All, order))
    }

    /// Nested: `work` holds `work/holm`.
    ///
    /// A parent tag shows even when no note has it, only its children.
    pub fn tags(&self) -> Vec<Tag> {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        let mut names: Vec<&str> = Vec::new();
        for entry in self.notes.values() {
            names.clear();
            for tag in &entry.info.tags {
                names.extend(tag.names());
            }
            names.sort_unstable();
            names.dedup();
            for name in &names {
                *counts.entry(name).or_default() += 1;
            }
        }
        nested_tags(&counts)
    }

    /// The count of notes right in each folder, without the notes of its subfolders. A folder with no notes is not in
    /// the map.
    pub fn folder_counts(&self) -> HashMap<&FolderPath, usize> {
        let mut counts = HashMap::new();
        for note in self.notes.keys() {
            *counts.entry(note.folder()).or_default() += 1;
        }
        counts
    }

    /// In the order of the text of the note.
    ///
    /// A note that the snapshot does not know has no links.
    pub fn links(&self, path: &NotePath) -> &[Link] {
        self.notes.get(path).map_or(&[], |entry| entry.links.as_slice())
    }

    /// The other notes that link to the note at `path`, by its id or by its file path. The newest change is first, as
    /// in [`Snapshot::newest`].
    pub fn backlinks(&self, path: &NotePath) -> Vec<NoteInfo> {
        let Some(target) = self.notes.get(path) else {
            return Vec::new();
        };
        let id = target.info.id.as_ref();
        let mut notes: Vec<&NoteInfo> = self
            .notes
            .values()
            .filter(|entry| {
                entry.info.path != *path
                    && entry.links.iter().any(|link| match &link.target {
                        LinkTarget::Note { id: to, .. } => Some(to) == id,
                        LinkTarget::File(file) => path.as_ref().as_os_str() == file.as_os_str(),
                        LinkTarget::Heading(_) | LinkTarget::Web(_) => false,
                    })
            })
            .map(|entry| &entry.info)
            .collect();
        notes.sort_unstable_by(|a, b| newest(a, b));
        notes.into_iter().cloned().collect()
    }

    /// The files may be missing on disk.
    pub(crate) fn removal(&self, node: &Node) -> Removal {
        let mut notes = 0;
        let mut inside = BTreeSet::new();
        let mut outside = HashSet::new();
        for entry in self.notes.values() {
            let held = node.holds_note(&entry.info.path);
            notes += usize::from(held);
            let files = entry.links.iter().filter_map(|link| match &link.target {
                LinkTarget::File(file) => Some(file.as_os_str()),
                LinkTarget::Note { .. } | LinkTarget::Heading(_) | LinkTarget::Web(_) => None,
            });
            if held {
                inside.extend(files);
            } else {
                outside.extend(files);
            }
        }
        let files = inside
            .into_iter()
            .filter(|file| !outside.contains(file))
            .map(|file: &OsStr| PathBuf::from(file))
            .collect();
        Removal { notes, files }
    }

    pub(crate) fn note_and_links(&self, path: &NotePath) -> Option<(&NoteInfo, &[Link])> {
        self.notes.get(path).map(|entry| (&entry.info, entry.links.as_slice()))
    }

    pub fn note(&self, path: &NotePath) -> Option<&NoteInfo> {
        self.notes.get(path).map(|entry| &entry.info)
    }

    pub fn note_with_id(&self, id: &NoteId) -> Option<&NoteInfo> {
        let path = self.ids.get(id)?;
        self.notes.get(path).map(|entry| &entry.info)
    }

    /// Pinned notes are on top. A path with no note is left out.
    pub(crate) fn sorted(&self, paths: impl IntoIterator<Item = NotePath>, order: Order) -> Vec<NotePath> {
        let mut notes: Vec<&NoteInfo> = paths.into_iter().filter_map(|path| self.note(&path)).collect();
        sort(&mut notes, order);
        notes.into_iter().map(|note| note.path.clone()).collect()
    }

    /// In the same order. The paths come from this snapshot.
    pub fn keep(&self, paths: &[NotePath], set: NoteSet<'_>) -> Vec<NotePath> {
        paths
            .iter()
            .filter(|path| set.holds_at(path, || self.note(path)))
            .cloned()
            .collect()
    }

    /// Every subfolder of the notes folder, sorted by path.
    ///
    /// Hidden folders, symlinks and the `attachments/` folder at the root are skipped.
    pub fn folders(&self) -> impl Iterator<Item = &FolderPath> {
        self.folders.iter()
    }

    pub(crate) fn has_folder(&self, folder: &FolderPath) -> bool {
        self.folders.contains(folder)
    }

    pub(crate) fn new(folders: Vec<FolderPath>) -> Self {
        Self {
            folders: folders.into_iter().collect(),
            ..Self::default()
        }
    }

    /// Takes out the notes at `removed`, then puts in `put`. So a note can move to a path and keep its id.
    pub(crate) fn swap(&mut self, removed: Vec<NotePath>, put: impl IntoIterator<Item = (NoteInfo, Vec<Link>)>) {
        for path in removed {
            self.remove(&path);
        }
        for (info, links) in put {
            let entry = NoteData { info, links };
            if self.notes.get(&entry.info.path).is_some_and(|old| **old == entry) {
                continue;
            }
            self.remove(&entry.info.path);
            if let Some(id) = &entry.info.id {
                self.ids.insert(id.clone(), entry.info.path.clone());
            }
            self.notes.insert(entry.info.path.clone(), Arc::new(entry));
        }
    }

    /// Puts `found` in place of the folders at `folder` or below it. `found` must be sorted.
    pub(crate) fn set_folders(&mut self, folder: &FolderPath, found: Vec<FolderPath>) {
        let old: Vec<FolderPath> = self
            .folders
            .iter()
            .filter(|old| *old == folder || old.is_below(folder))
            .cloned()
            .collect();
        if old == found {
            return;
        }
        for folder in &old {
            self.folders.remove(folder);
        }
        self.folders.extend(found);
    }

    pub(crate) fn next_revision(mut self) -> Self {
        self.revision = self.revision.next();
        self
    }

    fn remove(&mut self, path: &NotePath) {
        let Some(old) = self.notes.remove(path) else {
            return;
        };
        if let Some(id) = &old.info.id
            && self.ids.get(id).is_some_and(|owner| owner == path)
        {
            self.ids.remove(id);
        }
    }
}

/// Pinned notes first, then `order`. The path breaks ties, so the order never changes at random.
fn sort(notes: &mut [&NoteInfo], order: Order) {
    match order {
        Order::LastChange => notes.sort_unstable_by(|a, b| last_change(a, b)),
        Order::Title => {
            let mut keyed: Vec<_> = notes.iter().map(|note| (title_key(note), *note)).collect();
            keyed.sort_unstable_by(by_title);
            for (slot, (_, note)) in notes.iter_mut().zip(keyed) {
                *slot = note;
            }
        }
    }
}

fn last_change(a: &NoteInfo, b: &NoteInfo) -> Ordering {
    b.pinned.cmp(&a.pinned).then_with(|| newest(a, b))
}

/// The newest change first. The path breaks a tie.
fn newest(a: &NoteInfo, b: &NoteInfo) -> Ordering {
    b.modified.cmp(&a.modified).then_with(|| a.path.cmp(&b.path))
}

fn title_key(note: &NoteInfo) -> (bool, String) {
    (!note.pinned, note.title.to_lowercase())
}

fn by_title(a: &((bool, String), &NoteInfo), b: &((bool, String), &NoteInfo)) -> Ordering {
    a.0.cmp(&b.0).then_with(|| a.1.path.cmp(&b.1.path))
}

/// A parent tag counts the notes of its children.
fn nested_tags(counts: &BTreeMap<&str, usize>) -> Vec<Tag> {
    let mut children: HashMap<Option<&str>, Vec<&str>> = HashMap::new();
    for name in counts.keys() {
        children.entry(tag::parent(name)).or_default().push(name);
    }
    tag_children(None, counts, &children)
}

fn tag_children(
    parent: Option<&str>,
    counts: &BTreeMap<&str, usize>,
    children: &HashMap<Option<&str>, Vec<&str>>,
) -> Vec<Tag> {
    let mut tags: Vec<Tag> = children
        .get(&parent)
        .into_iter()
        .flatten()
        .map(|name| Tag {
            name: TagName::from_written((*name).to_owned()),
            notes: counts.get(name).copied().unwrap_or(0),
            children: tag_children(Some(name), counts, children),
        })
        .collect();
    tags.sort_by_cached_key(|tag| (tag.name.as_str().to_lowercase(), tag.name.clone()));
    tags
}
