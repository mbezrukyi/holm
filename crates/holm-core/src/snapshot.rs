//! The facts of the notes folder in memory: note infos, folders, tags and links.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::model::{Link, LinkTarget, NoteInfo, NoteSet, Order, Tag};
use crate::paths::{FolderPath, NotePath};

/// The facts of the notes folder at one moment, from [`Core::snapshot`](crate::Core::snapshot).
/// It never changes.
///
/// It is cheap to clone.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    revision: u64,
    notes: imbl::HashMap<NotePath, Arc<Entry>>,
    ids: imbl::HashMap<String, NotePath>,
    folders: imbl::OrdSet<FolderPath>,
}

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    info: NoteInfo,
    links: Vec<Link>,
}

/// The notes that a change of the index took out and put in.
///
/// [`Snapshot::apply`] takes them out first, so a note can move to a path and keep its id.
#[derive(Debug, Default)]
pub(crate) struct Delta {
    pub(crate) removed: Vec<NotePath>,
    pub(crate) put: Vec<(NoteInfo, Vec<Link>)>,
}

impl Snapshot {
    /// A number that grows with each change of the snapshot.
    ///
    /// Two snapshots with the same revision hold the same facts.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The notes of `set`, in `order`. Pinned notes are on top.
    ///
    /// Only `*.md` files are notes. Hidden files and folders, and symlinks, are skipped.
    /// A note that can't be read is left out.
    pub fn notes(&self, set: NoteSet<'_>, order: Order) -> Vec<NoteInfo> {
        let mut notes: Vec<&NoteInfo> = self.matching(set).collect();
        sort(&mut notes, order);
        notes.into_iter().cloned().collect()
    }

    /// The notes of `set`, the newest change first. Pins have no effect.
    pub fn newest(&self, set: NoteSet<'_>) -> Vec<NoteInfo> {
        let mut notes: Vec<&NoteInfo> = self.matching(set).collect();
        notes.sort_unstable_by(|a, b| newest(a, b));
        notes.into_iter().cloned().collect()
    }

    /// The paths of the notes of `set`, in `order`. Pinned notes are on top.
    pub fn paths(&self, set: NoteSet<'_>, order: Order) -> Vec<NotePath> {
        let mut notes: Vec<&NoteInfo> = self.matching(set).collect();
        sort(&mut notes, order);
        notes.into_iter().map(|note| note.path.clone()).collect()
    }

    /// The notes of `set`, in no fixed order.
    pub fn matching<'s, 'a>(
        &'s self,
        set: NoteSet<'a>,
    ) -> impl Iterator<Item = &'s NoteInfo> + use<'s, 'a> {
        self.notes
            .values()
            .filter(move |entry| set.holds(&entry.info))
            .map(|entry| &entry.info)
    }

    /// The first note of `set` in `order`, or `None` when `set` has no notes.
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

    /// Every tag as a tree. `work` holds `work/holm`.
    ///
    /// A parent tag shows even when no note has it, only its children.
    pub fn tags(&self) -> Vec<Tag> {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        let mut names: Vec<&str> = Vec::new();
        for entry in self.notes.values() {
            names.clear();
            for tag in &entry.info.tags {
                let ends = tag.match_indices('/').map(|(i, _)| i).chain([tag.len()]);
                names.extend(ends.map(|end| &tag[..end]));
            }
            names.sort_unstable();
            names.dedup();
            for name in &names {
                *counts.entry(name).or_default() += 1;
            }
        }
        tag_tree(&counts)
    }

    /// The count of notes right in each folder, without the notes of its subfolders. A folder
    /// with no notes is not in the map.
    pub fn folder_counts(&self) -> HashMap<&FolderPath, usize> {
        let mut counts = HashMap::new();
        for note in self.notes.keys() {
            *counts.entry(note.folder()).or_default() += 1;
        }
        counts
    }

    /// The links and embedded files in the note at `path`, in the order of its text.
    ///
    /// A note that the snapshot does not know has no links.
    pub fn links(&self, path: &NotePath) -> &[Link] {
        self.notes
            .get(path)
            .map_or(&[], |entry| entry.links.as_slice())
    }

    /// The other notes that link to the note at `path`, by its id or by its file path. The
    /// newest change is first, as in [`Snapshot::newest`].
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

    /// The note at `path`, or `None` when no note is there.
    pub fn note(&self, path: &NotePath) -> Option<&NoteInfo> {
        self.notes.get(path).map(|entry| &entry.info)
    }

    /// The note with `id`, or `None` when no note has it.
    pub fn note_with_id(&self, id: &str) -> Option<&NoteInfo> {
        let path = self.ids.get(id)?;
        self.notes.get(path).map(|entry| &entry.info)
    }

    /// The paths of `paths` that the snapshot has a note at, in `order`. Pinned notes are on
    /// top. The others are left out.
    pub fn sorted(&self, paths: impl IntoIterator<Item = NotePath>, order: Order) -> Vec<NotePath> {
        let mut notes: Vec<&NoteInfo> = paths
            .into_iter()
            .filter_map(|path| self.note(&path))
            .collect();
        sort(&mut notes, order);
        notes.into_iter().map(|note| note.path.clone()).collect()
    }

    /// The paths of `paths` in `set`, in the same order. The paths come from this snapshot.
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

    /// Whether the snapshot knows the folder.
    pub(crate) fn has_folder(&self, folder: &FolderPath) -> bool {
        self.folders.contains(folder)
    }

    /// A snapshot that holds `notes` and `folders`, with revision 0.
    pub(crate) fn new(notes: Vec<(NoteInfo, Vec<Link>)>, folders: Vec<FolderPath>) -> Self {
        let mut snapshot = Self {
            folders: folders.into_iter().collect(),
            ..Self::default()
        };
        snapshot.apply(Delta {
            removed: Vec::new(),
            put: notes,
        });
        snapshot
    }

    /// Takes out and puts in the notes of `delta`. It returns whether a fact changed.
    pub(crate) fn apply(&mut self, delta: Delta) -> bool {
        let mut changed = false;
        for path in delta.removed {
            changed |= self.remove(&path);
        }
        for (info, links) in delta.put {
            let entry = Entry { info, links };
            if self
                .notes
                .get(&entry.info.path)
                .is_some_and(|old| **old == entry)
            {
                continue;
            }
            self.remove(&entry.info.path);
            if let Some(id) = &entry.info.id {
                self.ids
                    .insert(id.as_str().to_owned(), entry.info.path.clone());
            }
            self.notes.insert(entry.info.path.clone(), Arc::new(entry));
            changed = true;
        }
        changed
    }

    /// Puts `found` in place of the folders at `folder` or below it. It returns whether they
    /// changed. `found` must be sorted.
    pub(crate) fn set_folders(&mut self, folder: &FolderPath, found: Vec<FolderPath>) -> bool {
        let old: Vec<FolderPath> = self
            .folders
            .iter()
            .filter(|old| *old == folder || old.is_below(folder))
            .cloned()
            .collect();
        if old == found {
            return false;
        }
        for folder in &old {
            self.folders.remove(folder);
        }
        self.folders.extend(found);
        true
    }

    /// The snapshot after this one, with the next revision.
    pub(crate) fn next_revision(mut self) -> Self {
        self.revision += 1;
        self
    }

    fn remove(&mut self, path: &NotePath) -> bool {
        let Some(old) = self.notes.remove(path) else {
            return false;
        };
        if let Some(id) = &old.info.id
            && self.ids.get(id.as_str()).is_some_and(|owner| owner == path)
        {
            self.ids.remove(id.as_str());
        }
        true
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
    b.modified
        .cmp(&a.modified)
        .then_with(|| a.path.cmp(&b.path))
}

fn title_key(note: &NoteInfo) -> (bool, String) {
    (!note.pinned, note.title.to_lowercase())
}

fn by_title(a: &((bool, String), &NoteInfo), b: &((bool, String), &NoteInfo)) -> Ordering {
    a.0.cmp(&b.0).then_with(|| a.1.path.cmp(&b.1.path))
}

/// Builds the tag tree from the count of notes of each tag. A parent tag counts the notes of its
/// children.
fn tag_tree(counts: &BTreeMap<&str, usize>) -> Vec<Tag> {
    let mut children: HashMap<Option<&str>, Vec<&str>> = HashMap::new();
    for name in counts.keys() {
        let parent = name.rsplit_once('/').map(|(parent, _)| parent);
        children.entry(parent).or_default().push(name);
    }
    tag_children(None, counts, &children)
}

/// The tags under `parent`, with their children.
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
            name: (*name).to_owned(),
            notes: counts.get(name).copied().unwrap_or(0),
            children: tag_children(Some(name), counts, children),
        })
        .collect();
    tags.sort_by_cached_key(|tag| (tag.name.to_lowercase(), tag.name.clone()));
    tags
}
