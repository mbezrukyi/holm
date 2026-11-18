//! The note files and the index rows, kept in step.

use std::path::{Path, PathBuf};

use crate::Result;
use crate::facts::Facts;
use crate::index::{Index, NoteRows, Rows};
use crate::model::DeleteCount;
use crate::nodes::{Node, Nodes};
use crate::paths::FolderPath;
use crate::reconcile::Settled;
use crate::scan;
use crate::snapshot::Snapshot;
use crate::store::Store;

/// The note files and the index rows.
#[derive(Debug)]
pub(crate) struct Disk {
    pub(crate) store: Store,
    index: Index,
}

impl Disk {
    /// Opens the index in `index_dir` for the notes folder of `store`. It returns the disk
    /// and the facts, built from the index rows and the files. It brings the rows up to date.
    /// When the rows can't be read, it starts from no facts and rebuilds the index.
    pub(crate) fn open(store: Store, index_dir: &Path) -> Result<(Self, Facts)> {
        let mut disk = Self {
            store,
            index: Index::open(index_dir)?,
        };
        let facts = disk.start()?;
        Ok((disk, facts))
    }

    fn start(&mut self) -> Result<Facts> {
        let (entries, readable) = match self.index.entries() {
            Ok(entries) => (entries, true),
            Err(error) => {
                tracing::warn!(%error, "cannot read the index, so Holm rebuilds it");
                (Vec::new(), false)
            }
        };
        let mut facts = Facts::new(entries, self.store.folders(&FolderPath::root())?);
        let settled = scan::files(&self.store, &facts, &Nodes::root())?;
        if readable {
            self.write_rows(&settled);
        } else {
            self.rebuild_from(&settled);
        }
        facts.apply(settled);
        Ok(facts)
    }

    /// Writes the rows of `settled` into the index. When SQLite fails, it rebuilds the index,
    /// so it never fails.
    pub(crate) fn write_rows(&mut self, settled: &Settled) {
        let rows = Rows {
            removed: &settled.removed,
            moved: settled
                .moved
                .iter()
                .map(|moved| (&moved.from, &moved.info.path))
                .collect(),
            put: put_rows(settled),
        };
        if let Err(error) = self.index.write(&rows) {
            tracing::warn!(%error, "the index failed, so Holm rebuilds it");
            self.rebuild();
        }
    }

    /// Runs `op` on the index. When SQLite fails, it rebuilds the index and runs `op` again.
    pub(crate) fn read_index<T>(&mut self, op: impl Fn(&mut Index) -> Result<T>) -> Result<T> {
        op(&mut self.index).or_else(|error| {
            tracing::warn!(%error, "the index failed, so Holm rebuilds it");
            self.rebuild();
            op(&mut self.index)
        })
    }

    /// Rebuilds the index from every note file, read with no facts. A failure leaves the
    /// index as it is, with a log line.
    fn rebuild(&mut self) {
        match scan::files(&self.store, &Facts::default(), &Nodes::root()) {
            Ok(settled) => self.rebuild_from(&settled),
            Err(error) => tracing::warn!(%error, "cannot rebuild the index"),
        }
    }

    /// Rebuilds the index with the rows of `settled`, which holds every note file.
    fn rebuild_from(&mut self, settled: &Settled) {
        if let Err(error) = self.index.rebuild(&put_rows(settled)) {
            tracing::warn!(%error, "cannot rebuild the index");
        }
    }

    /// The count of a delete of `node` in `snapshot`, and the attachment files it removes.
    pub(crate) fn removal(&self, snapshot: &Snapshot, node: &Node) -> (DeleteCount, Vec<PathBuf>) {
        let removal = snapshot.removal(node);
        let files = self.store.attachments(removal.files);
        let count = DeleteCount {
            notes: removal.notes,
            attachments: files.len(),
        };
        (count, files)
    }
}

/// The index rows of each put note of `settled`.
fn put_rows(settled: &Settled) -> Vec<NoteRows<'_>> {
    settled
        .put
        .iter()
        .map(|put| NoteRows {
            note: &put.note,
            links: &put.links,
            stamp: put.stamp,
            source: put.source,
        })
        .collect()
}
