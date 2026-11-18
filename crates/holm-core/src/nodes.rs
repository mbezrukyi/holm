//! The notes and folders that the core brings up to date with the disk.

use std::path::{Path, PathBuf};

use crate::paths::{FolderPath, NotePath};
use crate::snapshot::Snapshot;
use crate::store::{Entry, Store};

/// A note or a folder that the core brings up to date with the disk. A folder node holds
/// everything at its path and below it.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Node {
    Note(NotePath),
    Folder(FolderPath),
}

/// Nodes in which no node holds another, each once, sorted.
#[derive(Debug)]
pub(crate) struct Nodes(Vec<Node>);

impl Node {
    /// Whether `other` is this node or below it.
    pub(crate) fn holds(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Folder(folder), Self::Note(note)) => note.is_below(folder),
            (Self::Folder(node), Self::Folder(folder)) => node == folder || folder.is_below(node),
            (Self::Note(note), Self::Note(other)) => note == other,
            (Self::Note(_), Self::Folder(_)) => false,
        }
    }
}

impl Nodes {
    /// The nodes of `nodes` that no other of them holds.
    pub(crate) fn new(nodes: impl IntoIterator<Item = Node>) -> Self {
        let mut nodes: Vec<Node> = nodes.into_iter().collect();
        nodes.sort();
        nodes.dedup();
        let all = nodes.clone();
        nodes.retain(|node| !all.iter().any(|other| other != node && other.holds(node)));
        Self(nodes)
    }

    /// The whole notes folder.
    pub(crate) fn root() -> Self {
        Self(vec![Node::Folder(FolderPath::root())])
    }

    /// The nodes for outside changes at the raw paths `paths`: what is there now, and what was
    /// there in `before`.
    pub(crate) fn outside(
        store: &Store,
        before: &Snapshot,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> Self {
        Self::new(
            paths
                .into_iter()
                .flat_map(|path| outside(store, before, &path)),
        )
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn iter(&self) -> std::slice::Iter<'_, Node> {
        self.0.iter()
    }
}

impl<'a> IntoIterator for &'a Nodes {
    type Item = &'a Node;
    type IntoIter = std::slice::Iter<'a, Node>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// The nodes for an outside change at the raw path `path`. The empty path is the whole notes
/// folder.
///
/// A path that is gone, and that `before` knows as no note, may have been a folder that
/// `before` does not list, like the attachment folder. So it gives a folder node.
fn outside(store: &Store, before: &Snapshot, path: &Path) -> Vec<Node> {
    if path.as_os_str().is_empty() {
        return vec![Node::Folder(FolderPath::root())];
    }
    let Some((folder, name)) = FolderPath::split_raw(path) else {
        return Vec::new();
    };
    let mut nodes = Vec::new();
    let now = match store.entry(&folder, &name) {
        Entry::Note(note, _) => Some(Node::Note(note)),
        Entry::Folder(folder) => Some(Node::Folder(folder)),
        Entry::Other | Entry::Missing => None,
    };
    let note = folder
        .note_file(&name)
        .filter(|note| before.note(note).is_some());
    let child = folder.child(&name);
    match (&now, note) {
        (_, Some(note)) => nodes.push(Node::Note(note)),
        (Some(Node::Note(_)), None) if before.has_folder(&child) => {
            nodes.push(Node::Folder(child));
        }
        (Some(_), None) => {}
        (None, None) => nodes.push(Node::Folder(child)),
    }
    nodes.extend(now);
    nodes
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    fn note(path: &str) -> Node {
        Node::Note(NotePath::new(path).unwrap())
    }

    fn folder(path: &str) -> Node {
        Node::Folder(FolderPath::new(path).unwrap())
    }

    #[test]
    fn a_folder_node_holds_itself_and_what_is_below_it() {
        let work = folder("work");
        assert!(work.holds(&work));
        assert!(work.holds(&note("work/a.md")));
        assert!(work.holds(&folder("work/x")));
        assert!(!work.holds(&folder("workshop")));
        assert!(folder("").holds(&note("a.md")));
        let a = note("a.md");
        assert!(a.holds(&a));
        assert!(!a.holds(&folder("a.md")));
    }

    #[test]
    fn nodes_drop_a_node_inside_another_and_repeats() {
        let nodes = Nodes::new([
            note("work/a.md"),
            folder("work/x"),
            folder("work"),
            note("b.md"),
            note("b.md"),
            folder("workshop"),
        ]);
        assert_eq!(
            nodes.iter().cloned().collect::<Vec<_>>(),
            [note("b.md"), folder("work"), folder("workshop")]
        );
        assert_eq!(Nodes::new([note("a.md"), folder("")]).0, [folder("")]);
    }
}
