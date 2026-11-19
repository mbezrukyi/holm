//! The nodes that a change from another tool touches.

use std::path::{Path, PathBuf};

use crate::nodes::{Node, Nodes};
use crate::paths::FolderPath;
use crate::snapshot::Snapshot;
use crate::store::{Entry, Store};

/// The nodes for outside changes at the raw paths `paths`: what is there now, and what was
/// there in `before`.
pub(super) fn nodes(
    store: &Store,
    before: &Snapshot,
    paths: impl IntoIterator<Item = PathBuf>,
) -> Nodes {
    Nodes::new(paths.into_iter().flat_map(|path| at(store, before, &path)))
}

/// The nodes for an outside change at the raw path `path`. The empty path is the whole notes
/// folder.
///
/// A path that is gone, and that `before` knows as no note, may have been a folder that
/// `before` does not list, like the attachment folder. So it gives a folder node.
fn at(store: &Store, before: &Snapshot, path: &Path) -> Vec<Node> {
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
