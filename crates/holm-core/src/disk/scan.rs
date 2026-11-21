//! Runs the plans of [`reconcile`] on the disk: it lists and reads the note files, parses
//! their links, and gives each copied note a new id.

use crate::error::Result;
use crate::facts::Facts;
use crate::links::{self, LinkSource};
use crate::model::{Note, NoteId};
use crate::nodes::Nodes;
use crate::reconcile::{self, Plan, Read, Settled};
use crate::store::{ContentHash, Stamp, Store};

/// Brings the notes at or below each of `nodes` up to date with the files, from `facts`.
/// It reads only the files whose stamp changed, and the notes that `nodes` name. A file that can't be read is left out with a
/// log line.
///
/// # Errors
///
/// [`Error::Read`](crate::Error::Read) when the notes folder can't be read.
pub(super) fn files(store: &Store, facts: &Facts, nodes: &Nodes) -> Result<Settled> {
    let mut found = Vec::new();
    for node in nodes {
        found.extend(store.files(node)?);
    }
    let plan = reconcile::plan(facts, nodes, found);
    let reads = plan
        .reads()
        .iter()
        .filter_map(|path| match store.read(path) {
            Ok((note, stamp, hash)) => Some(read(store, facts, note, stamp, hash)),
            Err(error) => {
                tracing::warn!(%error, "skipped a note");
                None
            }
        })
        .collect();
    Ok(decide_with_ids(store, facts, plan, reads))
}

/// Settles notes that Holm just wrote, with their stamps and content hashes, against `facts`.
pub(super) fn wrote(
    store: &Store,
    facts: &Facts,
    notes: Vec<(Note, Stamp, ContentHash)>,
) -> Settled {
    let plan = reconcile::written(facts, notes.iter().map(|(note, ..)| note.info.path.clone()));
    let reads = notes
        .into_iter()
        .map(|(note, stamp, hash)| read(store, facts, note, stamp, hash))
        .collect();
    decide_with_ids(store, facts, plan, reads)
}

/// The read of `note`. Its links come from `facts` when its link source stays, else from
/// its body.
fn read(store: &Store, facts: &Facts, note: Note, stamp: Stamp, hash: ContentHash) -> Read {
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
        links::links(store.root(), &note)
    };
    Read {
        note,
        links,
        stamp,
        hash,
        source,
        owner_on_disk,
    }
}

/// Settles `plan`, then writes a new id into each copy. A copy whose write fails stays
/// without an id, with a log line.
fn decide_with_ids(store: &Store, facts: &Facts, plan: Plan, reads: Vec<Read>) -> Settled {
    let mut settled = plan.decide(facts, reads);
    for put in settled.put.iter_mut().filter(|put| put.copy) {
        let id = NoteId::generate();
        let path = &put.note.info.path;
        tracing::info!(new = %id, %path, "gave a copied note a new id");
        match store.edit_fields(path, |text| text.set_id(Some(id.as_str()))) {
            Ok((note, stamp, hash)) => (put.note, put.stamp, put.hash) = (note, stamp, hash),
            Err(error) => tracing::warn!(%error, "kept a copied note without an id"),
        }
    }
    settled
}
