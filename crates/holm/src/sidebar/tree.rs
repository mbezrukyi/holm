//! The data of the sidebar: the tree rows and the note lists, from one snapshot.

use holm_core::{FolderPath, Snapshot, TagName};
use std::collections::HashSet;
use std::ops::Range;

use holm_core::model::{Change, NoteSet, Order, Tag};

use super::list::NoteList;
use crate::config::{SavedSidebarView, SidebarItem};
use crate::list_cursor::{self, Toward};
use crate::text;

/// Each item has a note list.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Item {
    All,
    /// A folder that is not the root.
    Folder(FolderPath),
    Tag(TagName),
}

impl Item {
    /// "All notes" and the top folders and tags have none.
    pub(crate) fn parent(&self) -> Option<Self> {
        match self {
            Self::All => None,
            Self::Folder(path) => path.folder().filter(|parent| !parent.is_root()).map(Self::Folder),
            Self::Tag(name) => name.parent().map(Self::Tag),
        }
    }

    /// The folder for a new note or folder from this item. "All notes" and tags give the root.
    pub(crate) fn folder(&self) -> FolderPath {
        match self {
            Self::Folder(path) => path.clone(),
            Self::All | Self::Tag(_) => FolderPath::root(),
        }
    }

    pub(crate) fn set(&self) -> NoteSet<'_> {
        match self {
            Self::All => NoteSet::All,
            Self::Folder(path) => NoteSet::In(path),
            Self::Tag(name) => NoteSet::WithTag(name),
        }
    }

    /// The name the tree shows: "All notes", the last part of the folder path, or the last part of the tag name.
    pub(crate) fn name(&self) -> String {
        match self {
            Self::All => text::ALL_NOTES.to_owned(),
            Self::Folder(path) => path.last_part().into_owned(),
            Self::Tag(name) => name.last_part().to_owned(),
        }
    }

    /// This item after `change`, when it moves the item.
    pub(crate) fn moved(&self, change: &Change) -> Option<Self> {
        match self {
            Self::All => None,
            Self::Folder(path) => change.folder_after(path).map(Self::Folder),
            Self::Tag(name) => change.tag_after(name).map(Self::Tag),
        }
    }
}

/// The tree with a selected item, or the note list of an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SidebarView {
    Tree(Item),
    List(Item),
}

impl From<&SavedSidebarView> for SidebarView {
    fn from(saved: &SavedSidebarView) -> Self {
        let item = Item::from(&saved.item);
        if saved.list { Self::List(item) } else { Self::Tree(item) }
    }
}

impl From<&SidebarView> for SavedSidebarView {
    fn from(view: &SidebarView) -> Self {
        Self {
            list: matches!(view, SidebarView::List(_)),
            item: SidebarItem::from(view.item()),
        }
    }
}

impl From<&SidebarItem> for Item {
    fn from(saved: &SidebarItem) -> Self {
        match saved {
            SidebarItem::All => Self::All,
            SidebarItem::Folder(path) => Self::Folder(path.clone()),
            SidebarItem::Tag(name) => Self::Tag(name.clone()),
        }
    }
}

impl From<&Item> for SidebarItem {
    fn from(item: &Item) -> Self {
        match item {
            Item::All => Self::All,
            Item::Folder(path) => Self::Folder(path.clone()),
            Item::Tag(name) => Self::Tag(name.clone()),
        }
    }
}

impl SidebarView {
    pub(crate) fn item(&self) -> &Item {
        match self {
            Self::Tree(item) | Self::List(item) => item,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TreeRow {
    pub(crate) item: Item,
    /// The name of [`Item::name`].
    pub(crate) name: String,
    pub(crate) depth: usize,
    pub(crate) count: usize,
    /// Whether a row can show under this one. The field of a new folder counts.
    pub(crate) has_children: bool,
    /// Whether rows show under this one. A folder in the open set with no child row is closed.
    pub(crate) open: bool,
}

/// A group label of the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Label {
    Folders,
    Tags,
}

/// A place in the rows of a [`Tree`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RowIndex(usize);

impl RowIndex {
    /// For an element id.
    pub(crate) fn get(self) -> usize {
        self.0
    }
}

/// A place in the lines of a [`Tree`], which is the place in its uniform list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EntryIndex(usize);

impl EntryIndex {
    /// For the uniform list.
    pub(crate) fn get(self) -> usize {
        self.0
    }
}

/// One line of the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    Label(Label),
    Row(RowIndex),
    /// The name field of a new folder.
    NewFolder {
        depth: usize,
    },
}

/// The visible tree: its rows, and its lines with the group labels between the rows.
#[derive(Clone, Debug, Default)]
pub(crate) struct Tree {
    rows: Vec<TreeRow>,
    entries: Vec<Entry>,
}

impl Tree {
    pub(crate) fn row(&self, at: RowIndex) -> &TreeRow {
        &self.rows[at.0]
    }

    pub(crate) fn row_of(&self, item: &Item) -> Option<RowIndex> {
        self.rows.iter().position(|row| row.item == *item).map(RowIndex)
    }

    /// The row after `at`, or before it toward [`Toward::Previous`], and wrapping.
    pub(crate) fn step(&self, at: Option<RowIndex>, toward: Toward) -> Option<RowIndex> {
        list_cursor::step(at.map(|at| at.0), self.rows.len(), toward, |_| true).map(RowIndex)
    }

    /// The count of lines.
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// The lines at the places `range` of the uniform list.
    pub(crate) fn entries_in(&self, range: Range<usize>) -> impl Iterator<Item = Entry> {
        range.filter_map(|at| self.entries.get(at).copied())
    }

    fn add_new_folder(&mut self, parent: &FolderPath) {
        let depth = parent.depth();
        let at = if parent.is_root() {
            self.entries
                .iter()
                .position(|entry| *entry == Entry::Label(Label::Folders))
                .map(EntryIndex)
        } else {
            self.entry_of(&Item::Folder(parent.clone()))
        };
        if let Some(at) = at {
            self.entries.insert(at.0 + 1, Entry::NewFolder { depth });
        } else {
            // With no folders, the label comes after "All notes".
            self.entries.insert(1, Entry::NewFolder { depth });
            self.entries.insert(1, Entry::Label(Label::Folders));
        }
    }

    pub(crate) fn entry_of(&self, item: &Item) -> Option<EntryIndex> {
        self.entries
            .iter()
            .position(|entry| matches!(entry, Entry::Row(at) if self.row(*at).item == *item))
            .map(EntryIndex)
    }
}

/// With the count of notes right in it.
#[derive(Clone, Debug)]
pub(crate) struct Folder {
    pub(crate) path: FolderPath,
    pub(crate) notes: usize,
}

/// The facts of one snapshot in the form the sidebar needs.
#[derive(Clone, Debug, Default)]
pub(crate) struct Lists {
    pub(crate) snapshot: Snapshot,
    /// The order of `notes`.
    pub(crate) order: Order,
    /// Every note, in the order of the sidebar.
    pub(crate) notes: NoteList,
    /// Every folder, sorted by path, so a folder comes before its children.
    pub(crate) folders: Vec<Folder>,
    pub(crate) tags: Vec<Tag>,
}

impl Lists {
    pub(crate) fn new(snapshot: &Snapshot, order: Order) -> Self {
        let counts = snapshot.folder_counts();
        let folders = snapshot
            .folders()
            .map(|path| Folder {
                path: path.clone(),
                notes: counts.get(path).copied().unwrap_or(0),
            })
            .collect();
        Self {
            snapshot: snapshot.clone(),
            order,
            notes: NoteList::all(snapshot.clone(), order),
            folders,
            tags: snapshot.tags(),
        }
    }

    /// A folder list has only the notes right in it. A tag list also has the notes of its child tags.
    pub(crate) fn list(&self, item: &Item) -> NoteList {
        match item.set() {
            NoteSet::All => self.notes.clone(),
            set => self.notes.filter(set),
        }
    }

    pub(crate) fn has(&self, item: &Item) -> bool {
        match item {
            Item::All => true,
            Item::Folder(path) => self.folders.iter().any(|folder| folder.path == *path),
            Item::Tag(name) => Tag::walk(&self.tags).any(|tag| tag.name == *name),
        }
    }

    /// `item` if the lists have it, or else its nearest parent in the tree, or "All notes".
    pub(crate) fn nearest(&self, item: &Item) -> Item {
        let mut item = item.clone();
        while !self.has(&item) {
            item = item.parent().unwrap_or(Item::All);
        }
        item
    }

    /// The visible tree: "All notes", then the folders, then the tags, each group under its label. Only the children of
    /// `open` items show. `new_folder` is the parent of the field of a new folder. The field is the first child line of
    /// its parent. A field at the root comes first under the folder label.
    pub(crate) fn tree(&self, open: &HashSet<Item>, new_folder: Option<&FolderPath>) -> Tree {
        let rows = self.rows(open, new_folder);
        let mut entries = Vec::with_capacity(rows.len() + 3);
        let (mut folders, mut tags) = (false, false);
        for (at, row) in rows.iter().enumerate() {
            match row.item {
                Item::Folder(_) if !folders => {
                    folders = true;
                    entries.push(Entry::Label(Label::Folders));
                }
                Item::Tag(_) if !tags => {
                    tags = true;
                    entries.push(Entry::Label(Label::Tags));
                }
                _ => {}
            }
            entries.push(Entry::Row(RowIndex(at)));
        }
        let mut tree = Tree { rows, entries };
        if let Some(parent) = new_folder {
            tree.add_new_folder(parent);
        }
        tree
    }

    /// The visible rows of [`Self::tree`]. The parent of a new folder has children.
    fn rows(&self, open: &HashSet<Item>, new_folder: Option<&FolderPath>) -> Vec<TreeRow> {
        let mut rows = vec![TreeRow {
            item: Item::All,
            name: Item::All.name(),
            depth: 0,
            count: self.notes.len(),
            has_children: false,
            open: false,
        }];
        for (i, Folder { path, notes }) in self.folders.iter().enumerate() {
            let parent_open = path
                .parents()
                .take_while(|parent| !parent.is_root())
                .all(|parent| open.contains(&Item::Folder(parent)));
            if !parent_open {
                continue;
            }
            let item = Item::Folder(path.clone());
            let has_children =
                new_folder == Some(path) || self.folders.get(i + 1).is_some_and(|next| next.path.is_below(path));
            rows.push(TreeRow {
                name: item.name(),
                depth: path.depth() - 1,
                count: *notes,
                has_children,
                open: has_children && open.contains(&item),
                item,
            });
        }
        tag_rows(&self.tags, 0, open, &mut rows);
        rows
    }
}

fn tag_rows(tags: &[Tag], depth: usize, open: &HashSet<Item>, rows: &mut Vec<TreeRow>) {
    for tag in tags {
        let item = Item::Tag(tag.name.clone());
        let has_children = !tag.children.is_empty();
        let is_open = has_children && open.contains(&item);
        rows.push(TreeRow {
            name: item.name(),
            depth,
            count: tag.notes,
            has_children,
            open: is_open,
            item,
        });
        if is_open {
            tag_rows(&tag.children, depth + 1, open, rows);
        }
    }
}

#[cfg(test)]
mod tests;
