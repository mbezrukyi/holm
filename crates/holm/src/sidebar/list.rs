//! A note list of the sidebar: the paths of its notes, and the snapshot that knows each one.

use std::sync::Arc;

use holm_core::model::{NoteInfo, NoteSet, Order, SearchResults};
use holm_core::{NotePath, Snapshot};

/// The notes of one list, in the order of the list. Each path has a note in the snapshot.
///
/// It is cheap to clone, so it can move from a background task to the sidebar.
#[derive(Clone, Debug, Default)]
pub(crate) struct NoteList {
    snapshot: Snapshot,
    paths: Arc<[NotePath]>,
}

impl NoteList {
    /// In the order of the results.
    pub(crate) fn new(results: SearchResults) -> Self {
        let (snapshot, paths) = results.into_parts();
        Self {
            snapshot,
            paths: paths.into(),
        }
    }

    /// Every note of `snapshot`, in `order`.
    pub(crate) fn all(snapshot: Snapshot, order: Order) -> Self {
        let paths = snapshot.paths(NoteSet::All, order).into();
        Self { snapshot, paths }
    }

    /// The notes of this list in `set`, in the same order.
    pub(crate) fn filter(&self, set: NoteSet<'_>) -> Self {
        Self {
            paths: self.snapshot.keep(&self.paths, set).into(),
            snapshot: self.snapshot.clone(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.paths.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    pub(crate) fn path(&self, at: usize) -> Option<&NotePath> {
        self.paths.get(at)
    }

    pub(crate) fn note(&self, at: usize) -> Option<&NoteInfo> {
        self.snapshot.note(self.paths.get(at)?)
    }

    pub(crate) fn position(&self, path: &NotePath) -> Option<usize> {
        self.paths.iter().position(|known| known == path)
    }

    pub(crate) fn notes(&self) -> impl Iterator<Item = &NoteInfo> {
        self.paths.iter().filter_map(|path| self.snapshot.note(path))
    }
}
