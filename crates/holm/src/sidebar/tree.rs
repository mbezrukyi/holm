//! The data of the sidebar: the folder tree rows and the note lists, from one snapshot of the
//! notes folder.

use holm_core::{FolderPath, Snapshot};
use std::collections::HashSet;

use holm_core::model::{Change, NoteSet, Order, Tag};

use super::list::NoteList;

/// An item of the folder tree. Each item has a note list.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Item {
    All,
    /// A folder that is not the root.
    Folder(FolderPath),
    Tag(String),
}

impl Item {
    /// The item one level up. "All notes" and the top folders and tags have none.
    pub(crate) fn parent(&self) -> Option<Self> {
        match self {
            Self::All => None,
            Self::Folder(path) => path
                .folder()
                .filter(|parent| !parent.is_root())
                .map(Self::Folder),
            Self::Tag(name) => name
                .rsplit_once('/')
                .map(|(parent, _)| Self::Tag(parent.to_owned())),
        }
    }

    /// The folder for a new note from this item. Notes of "All notes" and tags go in the root.
    pub(crate) fn folder(&self) -> FolderPath {
        match self {
            Self::Folder(path) => path.clone(),
            Self::All | Self::Tag(_) => FolderPath::root(),
        }
    }

    /// The notes of the list of this item.
    pub(crate) fn set(&self) -> NoteSet<'_> {
        match self {
            Self::All => NoteSet::All,
            Self::Folder(path) => NoteSet::In(path),
            Self::Tag(name) => NoteSet::WithTag(name),
        }
    }

    /// The last part of the folder path or the tag name. "All notes" has none.
    pub(crate) fn name(&self) -> String {
        match self {
            Self::All => String::new(),
            Self::Folder(path) => path
                .name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            Self::Tag(name) => name
                .rsplit_once('/')
                .map_or(name.as_str(), |(_, name)| name)
                .to_owned(),
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

/// What the sidebar shows: the folder tree with a selected item, or the note list of an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum View {
    Tree(Item),
    List(Item),
}

impl Default for View {
    fn default() -> Self {
        Self::List(Item::All)
    }
}

impl View {
    pub(crate) fn item(&self) -> &Item {
        match self {
            Self::Tree(item) | Self::List(item) => item,
        }
    }
}

/// One row of the folder tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) item: Item,
    /// The last part of the folder path or the tag name.
    pub(crate) name: String,
    pub(crate) depth: usize,
    pub(crate) count: usize,
    /// Whether a row can show under this one. The field of a new folder counts.
    pub(crate) has_children: bool,
    /// Whether rows show under this one. A folder in the open set with no child row is closed.
    pub(crate) open: bool,
}

/// The facts of one snapshot in the form the sidebar needs.
#[derive(Clone, Debug, Default)]
pub(crate) struct Tree {
    pub(crate) snapshot: Snapshot,
    /// Every note, in the order of the sidebar.
    pub(crate) notes: NoteList,
    /// Every folder, sorted by path, so a folder comes before its children.
    pub(crate) folders: Vec<FolderPath>,
    pub(crate) tags: Vec<Tag>,
}

impl Tree {
    pub(crate) fn new(snapshot: &Snapshot, order: Order) -> Self {
        Self {
            snapshot: snapshot.clone(),
            notes: NoteList::all(snapshot.clone(), order),
            folders: snapshot.folders().cloned().collect(),
            tags: snapshot.tags(),
        }
    }

    /// The notes of the list of `item`. A folder list has only the notes right in it.
    /// A tag list also has the notes of its child tags.
    pub(crate) fn list(&self, item: &Item) -> NoteList {
        match item.set() {
            NoteSet::All => self.notes.clone(),
            set => self.notes.filter(set),
        }
    }

    /// Whether the tree still has `item`.
    pub(crate) fn has(&self, item: &Item) -> bool {
        match item {
            Item::All => true,
            Item::Folder(path) => self.folders.contains(path),
            Item::Tag(name) => find_tag(&self.tags, name).is_some(),
        }
    }

    /// `item` if the tree has it, or else its nearest parent in the tree, or "All notes".
    pub(crate) fn nearest(&self, item: &Item) -> Item {
        let mut item = item.clone();
        while !self.has(&item) {
            item = item.parent().unwrap_or(Item::All);
        }
        item
    }

    /// The visible rows: "All notes", then the folders, then the tags. Only the children of
    /// `open` items show. `new_folder` is the parent of the field of a new folder.
    pub(crate) fn rows(&self, open: &HashSet<Item>, new_folder: Option<&FolderPath>) -> Vec<Row> {
        let counts = self.snapshot.folder_counts();
        let mut rows = vec![Row {
            item: Item::All,
            name: String::new(),
            depth: 0,
            count: self.notes.len(),
            has_children: false,
            open: false,
        }];
        for (i, folder) in self.folders.iter().enumerate() {
            let parent_open = folder
                .parents()
                .take_while(|parent| !parent.is_root())
                .all(|parent| open.contains(&Item::Folder(parent)));
            if !parent_open {
                continue;
            }
            let item = Item::Folder(folder.clone());
            let has_children = new_folder == Some(folder)
                || self
                    .folders
                    .get(i + 1)
                    .is_some_and(|next| next.is_below(folder));
            rows.push(Row {
                name: item.name(),
                depth: folder.depth() - 1,
                count: counts.get(folder).copied().unwrap_or(0),
                has_children,
                open: has_children && open.contains(&item),
                item,
            });
        }
        tag_rows(&self.tags, 0, open, &mut rows);
        rows
    }
}

fn tag_rows(tags: &[Tag], depth: usize, open: &HashSet<Item>, rows: &mut Vec<Row>) {
    for tag in tags {
        let item = Item::Tag(tag.name.clone());
        let has_children = !tag.children.is_empty();
        let is_open = has_children && open.contains(&item);
        rows.push(Row {
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

fn find_tag<'a>(tags: &'a [Tag], name: &str) -> Option<&'a Tag> {
    tags.iter().find_map(|tag| {
        if tag.name == name {
            Some(tag)
        } else if name.starts_with(&format!("{}/", tag.name)) {
            find_tag(&tag.children, name)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use holm_core::Core;
    use tempfile::TempDir;

    use super::*;
    use crate::test_paths::folder_path;

    /// A notes folder with four notes, and its tree in title order.
    fn tree() -> Tree {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().join("notes");
        for folder in ["home", "work/deep"] {
            fs::create_dir_all(notes.join(folder)).unwrap();
        }
        let tags = |tags: &str| format!("---\ntags: [{tags}]\n---\n");
        fs::write(notes.join("a.md"), tags("work")).unwrap();
        fs::write(notes.join("work/b.md"), tags("work/holm")).unwrap();
        fs::write(notes.join("work/c.md"), "").unwrap();
        fs::write(notes.join("work/deep/d.md"), tags("workshop")).unwrap();
        let (core, _) = Core::open_with_feed(&notes, dir.path().join("index")).unwrap();
        Tree::new(&core.snapshot(), Order::Title)
    }

    fn names(rows: &[Row]) -> Vec<(String, usize, usize)> {
        rows.iter()
            .map(|row| (row.name.clone(), row.depth, row.count))
            .collect()
    }

    fn owned(rows: &[(&str, usize, usize)]) -> Vec<(String, usize, usize)> {
        rows.iter()
            .map(|(name, depth, count)| ((*name).to_owned(), *depth, *count))
            .collect()
    }

    #[test]
    fn closed_items_hide_their_children() {
        let rows = tree().rows(&HashSet::new(), None);
        assert_eq!(
            names(&rows),
            owned(&[
                ("", 0, 4),
                ("home", 0, 0),
                ("work", 0, 2),
                ("work", 0, 2),
                ("workshop", 0, 1)
            ])
        );
        assert!(rows[2].has_children && !rows[2].open);
        assert!(!rows[1].has_children);
    }

    #[test]
    fn open_items_show_their_children() {
        let open = HashSet::from([Item::Folder(folder_path("work")), Item::Tag("work".into())]);
        let rows = tree().rows(&open, None);
        assert_eq!(
            names(&rows),
            owned(&[
                ("", 0, 4),
                ("home", 0, 0),
                ("work", 0, 2),
                ("deep", 1, 1),
                ("work", 0, 2),
                ("holm", 1, 1),
                ("workshop", 0, 1)
            ])
        );
        assert!(rows[2].open);
    }

    #[test]
    fn an_open_folder_without_child_rows_is_closed() {
        let open = HashSet::from([Item::Folder(folder_path("home"))]);
        let rows = tree().rows(&open, None);
        assert!(!rows[1].has_children && !rows[1].open);

        let rows = tree().rows(&open, Some(&folder_path("home")));
        assert!(rows[1].has_children && rows[1].open);
    }

    #[test]
    fn lists_hold_their_notes() {
        let tree = tree();
        let paths = |item: Item| -> Vec<String> {
            tree.list(&item)
                .notes()
                .map(|note| note.path.to_string())
                .collect()
        };
        assert_eq!(paths(Item::All).len(), 4);
        assert_eq!(
            paths(Item::Folder(folder_path("work"))),
            ["work/b.md", "work/c.md"]
        );
        assert_eq!(paths(Item::Tag("work".into())), ["a.md", "work/b.md"]);
        assert_eq!(paths(Item::Tag("work/holm".into())), ["work/b.md"]);
    }

    #[test]
    fn items_know_their_parent_and_folder() {
        assert_eq!(
            Item::Folder(folder_path("work/deep")).parent(),
            Some(Item::Folder(folder_path("work")))
        );
        assert_eq!(Item::Folder(folder_path("work")).parent(), None);
        assert_eq!(
            Item::Tag("work/holm".into()).parent(),
            Some(Item::Tag("work".into()))
        );
        assert_eq!(Item::Tag("work/holm".into()).folder(), FolderPath::root());
        assert!(tree().has(&Item::Tag("work/holm".into())));
        assert!(!tree().has(&Item::Folder(folder_path("gone"))));
    }

    #[test]
    fn a_rename_moves_the_item_and_its_children() {
        let folder = |path: &str| Item::Folder(folder_path(path));
        let tag = |name: &str| Item::Tag(name.into());
        let work = Change::FolderMoved {
            from: folder_path("work"),
            to: folder_path("job"),
        };
        assert_eq!(folder("work").moved(&work), Some(folder("job")));
        assert_eq!(folder("work/deep").moved(&work), Some(folder("job/deep")));
        assert_eq!(folder("workshop").moved(&work), None);
        assert_eq!(tag("work").moved(&work), None);
        assert_eq!(Item::All.moved(&work), None);
        let work = Change::TagMoved {
            from: "work".into(),
            to: "home/job".into(),
        };
        assert_eq!(tag("work").moved(&work), Some(tag("home/job")));
        assert_eq!(tag("work/holm").moved(&work), Some(tag("home/job/holm")));
        assert_eq!(tag("workshop").moved(&work), None);
        assert_eq!(folder("work").moved(&work), None);
        assert_eq!(tag("home/job/holm").name(), "holm");
    }

    #[test]
    fn a_gone_item_falls_back_to_its_nearest_parent() {
        let tree = tree();
        let nearest = |item: Item| tree.nearest(&item);
        assert_eq!(
            nearest(Item::Folder(folder_path("work/deep"))),
            Item::Folder(folder_path("work/deep"))
        );
        assert_eq!(
            nearest(Item::Folder(folder_path("work/gone/deeper"))),
            Item::Folder(folder_path("work"))
        );
        assert_eq!(nearest(Item::Folder(folder_path("gone"))), Item::All);
        assert_eq!(
            nearest(Item::Tag("work/gone".into())),
            Item::Tag("work".into())
        );
        assert_eq!(nearest(Item::Tag("gone/holm".into())), Item::All);
    }
}
