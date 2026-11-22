//! A note list of the sidebar: the paths of its notes, and the snapshot that knows each one.

use std::ops::Range;
use std::sync::Arc;

use holm_core::model::{NoteInfo, NoteSet, Order, SearchResults};
use holm_core::{NotePath, Snapshot};

use crate::list_cursor::{self, Toward};

/// A place in a [`NoteList`], which is the place in its uniform list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NoteIndex(usize);

impl NoteIndex {
    /// For the uniform list and an element id.
    pub(crate) fn get(self) -> usize {
        self.0
    }

    #[must_use]
    pub(crate) fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// `None` for the first place.
    pub(crate) fn previous(self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}

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

    pub(crate) fn first(&self) -> Option<&NotePath> {
        self.paths.first()
    }

    pub(crate) fn path(&self, at: NoteIndex) -> Option<&NotePath> {
        self.paths.get(at.0)
    }

    /// The notes at the places `range` of the uniform list.
    pub(crate) fn notes_in(&self, range: Range<usize>) -> impl Iterator<Item = (NoteIndex, &NoteInfo)> {
        range.filter_map(|at| Some((NoteIndex(at), self.snapshot.note(self.paths.get(at)?)?)))
    }

    pub(crate) fn position(&self, path: &NotePath) -> Option<NoteIndex> {
        self.paths.iter().position(|known| known == path).map(NoteIndex)
    }

    /// The note after `at`, or before it toward [`Toward::Previous`], and wrapping.
    pub(crate) fn step(&self, at: Option<NoteIndex>, toward: Toward) -> Option<NoteIndex> {
        list_cursor::step(at.map(|at| at.0), self.len(), toward, |_| true).map(NoteIndex)
    }

    pub(crate) fn notes(&self) -> impl Iterator<Item = &NoteInfo> {
        self.paths.iter().filter_map(|path| self.snapshot.note(path))
    }
}
