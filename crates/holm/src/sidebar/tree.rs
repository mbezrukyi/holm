//! The data of the sidebar: the folder tree rows and the note lists, from one copy of the
//! notes, folders and tags of the core.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use holm_core::model::{NoteInfo, Tag};

/// An item of the folder tree. Each item has a note list.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Item {
    All,
    Folder(PathBuf),
    Tag(String),
}

impl Item {
    /// The item one level up. "All notes" and the top folders and tags have none.
    pub(crate) fn parent(&self) -> Option<Self> {
        match self {
            Self::All => None,
            Self::Folder(path) => path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .map(|parent| Self::Folder(parent.to_owned())),
            Self::Tag(name) => name
                .rsplit_once('/')
                .map(|(parent, _)| Self::Tag(parent.to_owned())),
        }
    }

    /// The folder for a new note from this item. Notes of "All notes" and tags go in the root.
    pub(crate) fn folder(&self) -> &Path {
        match self {
            Self::Folder(path) => path,
            Self::All | Self::Tag(_) => Path::new(""),
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
    pub(crate) has_children: bool,
    pub(crate) open: bool,
}

/// What the sidebar knows about the notes folder. The notes keep the order of the core.
#[derive(Clone, Debug, Default)]
pub(crate) struct Snapshot {
    pub(crate) notes: Vec<NoteInfo>,
    /// Every folder, sorted by path, so a folder comes before its children.
    pub(crate) folders: Vec<PathBuf>,
    pub(crate) tags: Vec<Tag>,
}

impl Snapshot {
    /// The notes of the list of `item`. A folder list has only the notes right in it.
    /// A tag list also has the notes of its child tags.
    pub(crate) fn list(&self, item: &Item) -> Vec<&NoteInfo> {
        self.notes.iter().filter(|note| holds(item, note)).collect()
    }

    /// Whether the tree still has `item`.
    pub(crate) fn has(&self, item: &Item) -> bool {
        match item {
            Item::All => true,
            Item::Folder(path) => self.folders.contains(path),
            Item::Tag(name) => find_tag(&self.tags, name).is_some(),
        }
    }

    /// The visible rows: "All notes", then the folders, then the tags. Only the children of
    /// `open` items show.
    pub(crate) fn rows(&self, open: &HashSet<Item>) -> Vec<Row> {
        let mut counts: HashMap<&Path, usize> = HashMap::new();
        for note in &self.notes {
            *counts.entry(folder_of(&note.path)).or_default() += 1;
        }
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
                .ancestors()
                .skip(1)
                .take_while(|parent| !parent.as_os_str().is_empty())
                .all(|parent| open.contains(&Item::Folder(parent.to_owned())));
            if !parent_open {
                continue;
            }
            let item = Item::Folder(folder.clone());
            rows.push(Row {
                name: folder
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                depth: folder.components().count() - 1,
                count: counts.get(folder.as_path()).copied().unwrap_or(0),
                has_children: self
                    .folders
                    .get(i + 1)
                    .is_some_and(|next| next.starts_with(folder)),
                open: open.contains(&item),
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
        let is_open = open.contains(&item);
        rows.push(Row {
            name: tag
                .name
                .rsplit_once('/')
                .map_or(tag.name.as_str(), |(_, name)| name)
                .to_owned(),
            depth,
            count: tag.notes,
            has_children: !tag.children.is_empty(),
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

fn holds(item: &Item, note: &NoteInfo) -> bool {
    match item {
        Item::All => true,
        Item::Folder(folder) => folder_of(&note.path) == folder,
        Item::Tag(name) => note.tags.iter().any(|tag| {
            tag == name
                || tag
                    .strip_prefix(name.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
        }),
    }
}

/// The folder of a note. The root folder is the empty path.
pub(crate) fn folder_of(path: &Path) -> &Path {
    path.parent().unwrap_or(Path::new(""))
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;

    fn note(path: &str, tags: &[&str]) -> NoteInfo {
        NoteInfo {
            path: path.into(),
            id: None,
            title: path.to_owned(),
            preview: String::new(),
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
            pinned: false,
            modified: SystemTime::UNIX_EPOCH,
        }
    }

    fn tag(name: &str, notes: usize, children: Vec<Tag>) -> Tag {
        Tag {
            name: name.to_owned(),
            notes,
            children,
        }
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            notes: vec![
                note("a.md", &["work"]),
                note("work/b.md", &["work/holm"]),
                note("work/c.md", &[]),
                note("work/deep/d.md", &["workshop"]),
            ],
            folders: vec!["home".into(), "work".into(), "work/deep".into()],
            tags: vec![
                tag("work", 2, vec![tag("work/holm", 1, Vec::new())]),
                tag("workshop", 1, Vec::new()),
            ],
        }
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
        let rows = snapshot().rows(&HashSet::new());
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
        let open = HashSet::from([Item::Folder("work".into()), Item::Tag("work".into())]);
        let rows = snapshot().rows(&open);
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
    fn lists_hold_their_notes() {
        let snapshot = snapshot();
        let paths = |item: Item| -> Vec<String> {
            snapshot
                .list(&item)
                .iter()
                .map(|note| note.path.display().to_string())
                .collect()
        };
        assert_eq!(paths(Item::All).len(), 4);
        assert_eq!(
            paths(Item::Folder("work".into())),
            ["work/b.md", "work/c.md"]
        );
        assert_eq!(paths(Item::Tag("work".into())), ["a.md", "work/b.md"]);
        assert_eq!(paths(Item::Tag("work/holm".into())), ["work/b.md"]);
    }

    #[test]
    fn items_know_their_parent_and_folder() {
        assert_eq!(
            Item::Folder("work/deep".into()).parent(),
            Some(Item::Folder("work".into()))
        );
        assert_eq!(Item::Folder("work".into()).parent(), None);
        assert_eq!(
            Item::Tag("work/holm".into()).parent(),
            Some(Item::Tag("work".into()))
        );
        assert_eq!(Item::Tag("work/holm".into()).folder(), Path::new(""));
        assert!(snapshot().has(&Item::Tag("work/holm".into())));
        assert!(!snapshot().has(&Item::Folder("gone".into())));
    }
}
