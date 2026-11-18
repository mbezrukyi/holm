//! Runs the plans of [`reconcile`] on the disk: it lists and reads the note files, parses
//! their links, and gives each copied note a new id.

use crate::error::Result;
use crate::facts::Facts;
use crate::model::{Note, NoteId};
use crate::nodes::Nodes;
use crate::reconcile::{self, Plan, Read, Settled};
use crate::relink::{self, LinkSource};
use crate::store::{Stamp, Store};

/// Brings the notes at or below each of `nodes` up to date with the files, from `facts`.
/// It reads only the files whose stamp changed. A file that can't be read is left out with a
/// log line.
///
/// # Errors
///
/// [`Error::Read`](crate::Error::Read) when the notes folder can't be read.
pub(crate) fn files(store: &Store, facts: &Facts, nodes: &Nodes) -> Result<Settled> {
    let mut found = Vec::new();
    for node in nodes {
        found.extend(store.files(node)?);
    }
    let plan = reconcile::plan(facts, nodes, found);
    let reads = plan
        .reads()
        .iter()
        .filter_map(|path| match store.read(path) {
            Ok((note, stamp)) => Some(read(store, facts, note, stamp)),
            Err(error) => {
                tracing::warn!(%error, "skipped a note");
                None
            }
        })
        .collect();
    Ok(settle(store, facts, plan, reads))
}

/// Settles notes that Holm just wrote, with their stamps, against `facts`.
pub(crate) fn wrote(store: &Store, facts: &Facts, notes: Vec<(Note, Stamp)>) -> Settled {
    let plan = reconcile::written(facts, notes.iter().map(|(note, _)| note.info.path.clone()));
    let reads = notes
        .into_iter()
        .map(|(note, stamp)| read(store, facts, note, stamp))
        .collect();
    settle(store, facts, plan, reads)
}

/// The read of `note`. Its links come from `facts` when its link source stays, else from
/// its body.
fn read(store: &Store, facts: &Facts, note: Note, stamp: Stamp) -> Read {
    let owner_on_disk = note
        .info
        .id
        .as_ref()
        .and_then(|id| facts.snapshot().note_with_id(id.as_str()))
        .filter(|owner| owner.path != note.info.path)
        .is_some_and(|owner| store.exists(&owner.path));
    let source = LinkSource::of(&note);
    let path = &note.info.path;
    let links = if facts.link_source(path) == Some(source) {
        facts.snapshot().links(path).to_vec()
    } else {
        relink::links(store.root(), &note)
    };
    Read {
        note,
        links,
        stamp,
        source,
        owner_on_disk,
    }
}

/// Settles `plan`, then writes a new id into each copy. A copy whose write fails stays
/// without an id, with a log line.
fn settle(store: &Store, facts: &Facts, plan: Plan, reads: Vec<Read>) -> Settled {
    let mut settled = plan.settle(facts, reads);
    for put in settled.put.iter_mut().filter(|put| put.copy) {
        let id = NoteId::generate();
        let path = &put.note.info.path;
        tracing::info!(new = %id, %path, "gave a copied note a new id");
        match store.edit_fields(path, |text| text.set_id(Some(id.as_str()))) {
            Ok((note, stamp)) => (put.note, put.stamp) = (note, stamp),
            Err(error) => tracing::warn!(%error, "kept a copied note without an id"),
        }
    }
    settled
}
