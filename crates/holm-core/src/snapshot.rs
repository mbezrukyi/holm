//! The facts of the notes folder in memory: note infos, folders, tags and links.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::model::{Link, LinkTarget, NoteInfo, NoteSet, Order, Tag};

/// The facts of the notes folder at one moment, from [`Core::snapshot`](crate::Core::snapshot).
/// It never changes.
///
/// It is cheap to clone. All paths are relative to the notes folder, as in
/// [`Core`](crate::Core).
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    revision: u64,
    notes: imbl::HashMap<PathBuf, Arc<Entry>>,
    ids: imbl::HashMap<String, PathBuf>,
    folders: imbl::OrdSet<PathBuf>,
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
    pub(crate) removed: Vec<PathBuf>,
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
        self.list(|entry| set.holds(&entry.info), order)
    }

    /// The paths of the notes of `set`, in `order`. Pinned notes are on top.
    pub fn paths(&self, set: NoteSet<'_>, order: Order) -> Vec<PathBuf> {
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
            Order::Title => self.matching(set).min_by_key(|note| title_key(note)),
        }
    }

    /// Every tag as a tree. `work` holds `work/holm`.
    ///
    /// A parent tag shows even when no note has it, only its children.
    pub fn tags(&self) -> Vec<Tag> {
        let pairs: Vec<(&str, usize)> = self
            .notes
            .values()
            .enumerate()
            .flat_map(|(note, entry)| entry.info.tags.iter().map(move |tag| (tag.as_str(), note)))
            .collect();
        tag_tree(&pairs)
    }

    /// The links and embedded files in the note at `path`, in the order of its text.
    ///
    /// A note that the snapshot does not know has no links.
    pub fn links(&self, path: impl AsRef<Path>) -> &[Link] {
        self.notes
            .get(path.as_ref())
            .map_or(&[], |entry| entry.links.as_slice())
    }

    /// The other notes that link to the note at `path`, by its id or by its file path.
    pub fn backlinks(&self, path: impl AsRef<Path>, order: Order) -> Vec<NoteInfo> {
        let path = path.as_ref();
        let Some(target) = self.notes.get(path) else {
            return Vec::new();
        };
        let id = target.info.id.as_ref();
        self.list(
            |entry| {
                entry.info.path != path
                    && entry.links.iter().any(|link| match &link.target {
                        LinkTarget::Note { id: to, .. } => Some(to) == id,
                        LinkTarget::File(file) => file == path,
                        LinkTarget::Heading(_) | LinkTarget::Web(_) => false,
                    })
            },
            order,
        )
    }

    /// The note at `path`, or `None` when no note is there.
    pub fn note(&self, path: impl AsRef<Path>) -> Option<&NoteInfo> {
        self.notes.get(path.as_ref()).map(|entry| &entry.info)
    }

    /// The note with `id`, or `None` when no note has it.
    pub fn note_with_id(&self, id: &str) -> Option<&NoteInfo> {
        let path = self.ids.get(id)?;
        self.notes.get(path).map(|entry| &entry.info)
    }

    /// The paths of `paths` that the snapshot has a note at, in `order`. Pinned notes are on
    /// top. The others are left out.
    pub fn sorted(&self, paths: impl IntoIterator<Item = PathBuf>, order: Order) -> Vec<PathBuf> {
        let mut notes: Vec<&NoteInfo> = paths
            .into_iter()
            .filter_map(|path| self.note(path))
            .collect();
        sort(&mut notes, order);
        notes.into_iter().map(|note| note.path.clone()).collect()
    }

    /// Every subfolder of the notes folder, sorted by path.
    ///
    /// Hidden folders, symlinks and the `attachments/` folder at the root are skipped.
    pub fn folders(&self) -> impl Iterator<Item = &Path> {
        self.folders.iter().map(PathBuf::as_path)
    }

    /// A snapshot that holds `notes` and `folders`, with revision 0.
    pub(crate) fn new(notes: Vec<(NoteInfo, Vec<Link>)>, folders: Vec<PathBuf>) -> Self {
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

    /// Puts `found` in place of the folders at or below `scope`. It returns whether they changed.
    /// `found` must be sorted.
    pub(crate) fn set_folders(&mut self, scope: &Path, found: Vec<PathBuf>) -> bool {
        let old: Vec<PathBuf> = self
            .folders
            .iter()
            .filter(|folder| folder.starts_with(scope))
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

    fn remove(&mut self, path: &Path) -> bool {
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

    fn list(&self, keep: impl Fn(&Entry) -> bool, order: Order) -> Vec<NoteInfo> {
        let mut notes: Vec<&NoteInfo> = self
            .notes
            .values()
            .filter(|entry| keep(entry))
            .map(|entry| &entry.info)
            .collect();
        sort(&mut notes, order);
        notes.into_iter().cloned().collect()
    }
}

/// Pinned notes first, then `order`. The path breaks ties, so the order never changes at random.
fn sort(notes: &mut [&NoteInfo], order: Order) {
    match order {
        Order::LastChange => notes.sort_by(|a, b| last_change(a, b)),
        Order::Title => notes.sort_by_cached_key(|note| title_key(note)),
    }
}

fn last_change(a: &NoteInfo, b: &NoteInfo) -> std::cmp::Ordering {
    (b.pinned, b.modified)
        .cmp(&(a.pinned, a.modified))
        .then_with(|| a.path.cmp(&b.path))
}

fn title_key(note: &NoteInfo) -> (bool, String, &Path) {
    (!note.pinned, note.title.to_lowercase(), &note.path)
}

/// Builds the tag tree from `(tag, note)` pairs. A parent tag counts the notes of its children.
fn tag_tree(pairs: &[(&str, usize)]) -> Vec<Tag> {
    let mut notes: BTreeMap<&str, HashSet<usize>> = BTreeMap::new();
    for &(tag, note) in pairs {
        let ends = tag.match_indices('/').map(|(i, _)| i).chain([tag.len()]);
        for end in ends {
            notes.entry(&tag[..end]).or_default().insert(note);
        }
    }
    let mut children: HashMap<Option<&str>, Vec<&str>> = HashMap::new();
    for name in notes.keys() {
        let parent = name.rsplit_once('/').map(|(parent, _)| parent);
        children.entry(parent).or_default().push(name);
    }
    tag_children(None, &notes, &children)
}

/// The tags under `parent`, with their children.
fn tag_children(
    parent: Option<&str>,
    notes: &BTreeMap<&str, HashSet<usize>>,
    children: &HashMap<Option<&str>, Vec<&str>>,
) -> Vec<Tag> {
    let mut tags: Vec<Tag> = children
        .get(&parent)
        .into_iter()
        .flatten()
        .map(|name| Tag {
            name: (*name).to_owned(),
            notes: notes.get(name).map_or(0, HashSet::len),
            children: tag_children(Some(name), notes, children),
        })
        .collect();
    tags.sort_by_cached_key(|tag| (tag.name.to_lowercase(), tag.name.clone()));
    tags
}
