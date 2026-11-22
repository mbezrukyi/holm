//! The nodes that a change from another tool touches.

use crate::nodes::{Node, Nodes};
use crate::paths::{FolderPath, RawPath};
use crate::snapshot::Snapshot;
use crate::store::{AtPath, Store};

/// The nodes for outside changes at `paths`: what is there now, and what was there in `before`.
pub(super) fn nodes(store: &Store, before: &Snapshot, paths: impl IntoIterator<Item = RawPath>) -> Nodes {
    Nodes::new(paths.into_iter().flat_map(|path| at(store, before, &path)))
}

/// A path that is gone, and that `before` knows as no note, may have been a folder that `before` does not list, like
/// the attachment folder. So it gives a folder node.
fn at(store: &Store, before: &Snapshot, path: &RawPath) -> Vec<Node> {
    if path.is_root() {
        return vec![Node::Folder(FolderPath::root())];
    }
    let Some((folder, name)) = FolderPath::split_raw(path) else {
        return Vec::new();
    };
    let mut nodes = Vec::new();
    let now = match store.at(&folder, &name) {
        AtPath::Note(note, _) => Some(Node::Note(note)),
        AtPath::Folder(folder) => Some(Node::Folder(folder)),
        AtPath::Other | AtPath::Missing => None,
    };
    let note = folder.note_file(&name).filter(|note| before.note(note).is_some());
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
