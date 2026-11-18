//! The rules that bring the facts up to date with the note files: moves, gone notes and id
//! clashes.
//!
//! It does no I/O. The core lists and reads the files, and runs what it settles.

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;

use crate::facts::Facts;
use crate::model::{Link, LinkTarget, Note, NoteId, NoteInfo};
use crate::nodes::Nodes;
use crate::paths::NotePath;
use crate::snapshot::Snapshot;
use crate::store::Stamp;

/// What brings the notes of a scope up to date: the moves, the gone notes and the files to read.
#[derive(Debug)]
pub(crate) struct Plan {
    moves: Vec<(NotePath, NotePath, Stamp)>,
    gone: Vec<NotePath>,
    reads: Vec<NotePath>,
    /// The note files in the scope now.
    found: HashSet<NotePath>,
}

/// A note file that the core read for a [`Plan`].
#[derive(Debug)]
pub(crate) struct Read {
    pub(crate) note: Note,
    pub(crate) stamp: Stamp,
    /// Whether a note file is at the path where the facts keep the id of the note, when that
    /// path is another one.
    pub(crate) owner_on_disk: bool,
}

/// What a [`Plan`] changes in the facts.
#[derive(Debug, Default)]
pub(crate) struct Settled {
    /// The known notes that are gone.
    pub(crate) removed: Vec<NotePath>,
    /// The notes that moved with no change.
    pub(crate) moved: Vec<Moved>,
    /// The notes that were read, known paths first.
    pub(crate) put: Vec<Put>,
}

/// A note at a new path, with the facts of its old path.
#[derive(Debug)]
pub(crate) struct Moved {
    pub(crate) from: NotePath,
    pub(crate) info: NoteInfo,
    pub(crate) links: Vec<Link>,
    pub(crate) stamp: Stamp,
}

/// A note that was read.
#[derive(Debug)]
pub(crate) struct Put {
    pub(crate) note: Note,
    pub(crate) stamp: Stamp,
    /// Whether the facts know a note at this path.
    pub(crate) known: bool,
    /// Whether the note is a copy: another note file keeps its id. A copy has no id here, so
    /// the core can give it a new one.
    pub(crate) copy: bool,
}

/// The plan for the notes at or below each of `nodes`, where `files` are the note files now.
///
/// A known path that is gone and a new path are one move when they have the same file name and
/// stamp, and no other gone or new path has both. A folder rename keeps the stamps of the files
/// in it. A note with a file link is read again at its new path instead, since a relative file
/// link points to another file after a move.
///
/// It reads each new file, and each known file whose stamp changed. Known files come first,
/// so a known file keeps its id when a new file has the same id.
pub(crate) fn plan(facts: &Facts, nodes: &Nodes, files: Vec<(NotePath, Stamp)>) -> Plan {
    let known = facts.stamps_in(nodes);
    let found: HashMap<NotePath, Stamp> = files.into_iter().collect();
    let left: Vec<(&NotePath, Stamp)> = known
        .iter()
        .filter(|(path, _)| !found.contains_key(*path))
        .map(|(path, stamp)| (path, *stamp))
        .collect();
    let joined: Vec<(&NotePath, Stamp)> = found
        .iter()
        .filter(|(path, _)| !known.contains_key(*path))
        .map(|(path, stamp)| (path, *stamp))
        .collect();
    let mut moves = Vec::new();
    let mut done = HashSet::new();
    for (from, to) in pairs(&left, &joined) {
        let file_link = facts
            .snapshot()
            .links(from)
            .iter()
            .any(|link| matches!(link.target, LinkTarget::File(_)));
        if !file_link {
            done.insert(from);
            done.insert(to);
            moves.push((from.clone(), to.clone(), found[to]));
        }
    }
    let gone = left
        .iter()
        .filter(|(path, _)| !done.contains(path))
        .map(|(path, _)| (*path).clone())
        .collect();
    let mut reads: Vec<(bool, &NotePath)> = found
        .iter()
        .filter(|(path, stamp)| known.get(*path) != Some(stamp) && !done.contains(path))
        .map(|(path, _)| (!known.contains_key(path), path))
        .collect();
    reads.sort_unstable();
    let reads = reads.into_iter().map(|(_, path)| path.clone()).collect();
    Plan {
        moves,
        gone,
        reads,
        found: found.into_keys().collect(),
    }
}

/// The plan for notes that Holm just wrote at `paths`: it reads each of them, known paths first.
pub(crate) fn written(facts: &Facts, paths: impl IntoIterator<Item = NotePath>) -> Plan {
    let mut reads: Vec<(bool, NotePath)> = paths
        .into_iter()
        .map(|path| (facts.snapshot().note(&path).is_none(), path))
        .collect();
    reads.sort_unstable();
    reads.dedup();
    let reads: Vec<NotePath> = reads.into_iter().map(|(_, path)| path).collect();
    Plan {
        moves: Vec::new(),
        gone: Vec::new(),
        found: reads.iter().cloned().collect(),
        reads,
    }
}

impl Plan {
    /// The files to read, in order.
    pub(crate) fn reads(&self) -> &[NotePath] {
        &self.reads
    }

    /// What the plan changes in `facts`, with `reads` from the files of [`Plan::reads`].
    /// A path to read with no read failed, so its note is gone. A read of another path does
    /// nothing.
    ///
    /// Each note keeps its id, unless an earlier note in the order keeps it, or a note file at
    /// another path that the facts know. When that file is gone, its note is gone too.
    pub(crate) fn settle(self, facts: &Facts, reads: Vec<Read>) -> Settled {
        let snapshot = facts.snapshot();
        let mut reads: HashMap<NotePath, Read> = reads
            .into_iter()
            .map(|read| (read.note.info.path.clone(), read))
            .collect();
        let mut owners = Owners::new(snapshot);
        let mut settled = Settled::default();
        for (from, to, stamp) in self.moves {
            let Some(old) = snapshot.note(&from) else {
                continue;
            };
            owners.free(&from);
            let info = NoteInfo {
                path: to,
                ..old.clone()
            };
            if let Some(id) = &info.id {
                owners.claim(id, &info.path);
            }
            settled.moved.push(Moved {
                links: snapshot.links(&from).to_vec(),
                from,
                info,
                stamp,
            });
        }
        for path in self.gone {
            owners.free(&path);
            settled.removed.push(path);
        }
        for path in self.reads {
            let known = snapshot.note(&path).is_some();
            owners.free(&path);
            let Some(Read {
                mut note,
                stamp,
                owner_on_disk,
            }) = reads.remove(&path)
            else {
                if known {
                    settled.removed.push(path);
                }
                continue;
            };
            let mut copy = false;
            if let Some(id) = &note.info.id
                && let Some((owner, claimed)) = owners.owner(id)
                && owner != path
            {
                if claimed || self.found.contains(&owner) || owner_on_disk {
                    copy = true;
                } else {
                    owners.free(&owner);
                    settled.removed.push(owner);
                }
            }
            if copy {
                note.info.id = None;
            } else if let Some(id) = &note.info.id {
                owners.claim(id, &path);
            }
            settled.put.push(Put {
                note,
                stamp,
                known,
                copy,
            });
        }
        settled
    }
}

/// The owner of each id while a plan settles: the facts, with the paths that the plan freed
/// and the ids it claimed.
struct Owners<'a> {
    snapshot: &'a Snapshot,
    freed: HashSet<NotePath>,
    claimed: HashMap<NoteId, NotePath>,
}

impl<'a> Owners<'a> {
    fn new(snapshot: &'a Snapshot) -> Self {
        Self {
            snapshot,
            freed: HashSet::new(),
            claimed: HashMap::new(),
        }
    }

    /// The path that keeps `id`, and whether the plan claimed it there.
    fn owner(&self, id: &NoteId) -> Option<(NotePath, bool)> {
        if let Some(path) = self.claimed.get(id) {
            return Some((path.clone(), true));
        }
        self.snapshot
            .note_with_id(id.as_str())
            .filter(|owner| !self.freed.contains(&owner.path))
            .map(|owner| (owner.path.clone(), false))
    }

    /// Drops the facts of `path`, so its old id is free.
    fn free(&mut self, path: &NotePath) {
        self.freed.insert(path.clone());
    }

    fn claim(&mut self, id: &NoteId, path: &NotePath) {
        self.claimed.insert(id.clone(), path.clone());
    }
}

/// The gone paths and new paths that are one file at a new place: the same file name
/// and stamp, with no other gone or new path that has both.
fn pairs<'a>(
    left: &[(&'a NotePath, Stamp)],
    joined: &[(&'a NotePath, Stamp)],
) -> Vec<(&'a NotePath, &'a NotePath)> {
    type Key<'a> = (&'a OsStr, Stamp);
    type Sides<'a> = (Vec<&'a NotePath>, Vec<&'a NotePath>);
    let mut sides: HashMap<Key<'a>, Sides<'a>> = HashMap::new();
    for (path, stamp) in left {
        sides.entry((path.name(), *stamp)).or_default().0.push(path);
    }
    for (path, stamp) in joined {
        if let Some(side) = sides.get_mut(&(path.name(), *stamp)) {
            side.1.push(path);
        }
    }
    sides
        .into_values()
        .filter_map(|(from, to)| match (from.as_slice(), to.as_slice()) {
            (&[from], &[to]) => Some((from, to)),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use std::path::PathBuf;
    use std::time::SystemTime;

    use super::*;
    use crate::diff::Touched;
    use crate::model::{Change, Source};
    use crate::nodes::Node;

    /// The change list that `settled` makes in `before`, as a batch from outside.
    fn changes(before: &Facts, settled: Settled) -> Vec<Change> {
        let mut touched = Touched::default();
        touched.settled(&settled);
        let mut after = before.clone();
        let links = vec![Vec::new(); settled.put.len()];
        after.settle(settled, links);
        let (_, list) = touched.list(Source::Outside, before, after);
        list.map(|list| list.changes).unwrap_or_default()
    }

    fn path(path: &str) -> NotePath {
        NotePath::new(path).unwrap()
    }

    fn stamp(size: i64) -> Stamp {
        Stamp {
            modified: 1,
            changed: 1,
            size,
        }
    }

    fn info(at: &str, id: Option<&str>) -> NoteInfo {
        NoteInfo {
            path: path(at),
            id: id.map(|id| NoteId::from_field(id.to_owned())),
            title: String::new(),
            preview: String::new(),
            tags: Vec::new(),
            pinned: false,
            modified: SystemTime::UNIX_EPOCH,
        }
    }

    /// Facts with a note at each path, with its id and stamp.
    fn facts(notes: &[(&str, Option<&str>, Stamp)]) -> Facts {
        Facts::new(
            notes
                .iter()
                .map(|(at, id, stamp)| (info(at, *id), Vec::new(), *stamp))
                .collect(),
            Vec::new(),
        )
    }

    fn read(at: &str, id: Option<&str>, stamp: Stamp, owner_on_disk: bool) -> Read {
        Read {
            note: Note {
                info: info(at, id),
                body: String::new(),
            },
            stamp,
            owner_on_disk,
        }
    }

    fn put_ids(settled: &Settled) -> Vec<(String, Option<&str>, bool)> {
        settled
            .put
            .iter()
            .map(|put| {
                let info = &put.note.info;
                (
                    info.path.to_string(),
                    info.id.as_ref().map(NoteId::as_str),
                    put.copy,
                )
            })
            .collect()
    }

    fn sorted(paths: &[NotePath]) -> Vec<String> {
        let mut paths: Vec<String> = paths.iter().map(ToString::to_string).collect();
        paths.sort();
        paths
    }

    #[test]
    fn a_gone_path_and_a_new_path_with_one_name_and_stamp_are_a_move() {
        let facts = facts(&[("a.md", Some("A"), stamp(5))]);
        let plan = plan(&facts, &Nodes::root(), vec![(path("work/a.md"), stamp(5))]);
        assert_eq!(plan.reads(), Vec::<NotePath>::new());

        let settled = plan.settle(&facts, Vec::new());
        assert_eq!(settled.removed, Vec::<NotePath>::new());
        assert_eq!(settled.moved.len(), 1);
        assert_eq!(settled.moved[0].from, path("a.md"));
        assert_eq!(settled.moved[0].info, info("work/a.md", Some("A")));
        assert_eq!(
            changes(&facts, settled),
            [Change::Moved {
                from: path("a.md"),
                note: info("work/a.md", Some("A")),
            }]
        );
    }

    #[test]
    fn two_gone_files_with_one_name_and_stamp_are_no_move() {
        let facts = facts(&[("x/a.md", None, stamp(5)), ("y/a.md", None, stamp(5))]);
        let plan = plan(&facts, &Nodes::root(), vec![(path("z/a.md"), stamp(5))]);
        assert_eq!(plan.reads(), [path("z/a.md")]);

        let settled = plan.settle(&facts, vec![read("z/a.md", None, stamp(5), false)]);
        assert!(settled.moved.is_empty());
        assert_eq!(sorted(&settled.removed), ["x/a.md", "y/a.md"]);
        assert_eq!(put_ids(&settled), [("z/a.md".to_owned(), None, false)]);
    }

    #[test]
    fn a_note_with_a_file_link_is_read_at_its_new_path() {
        let link = Link {
            target: LinkTarget::File(PathBuf::from("b.png")),
            embedded: true,
        };
        let facts = Facts::new(
            vec![(info("a.md", Some("A")), vec![link], stamp(5))],
            Vec::new(),
        );
        let plan = plan(&facts, &Nodes::root(), vec![(path("work/a.md"), stamp(5))]);
        assert_eq!(plan.reads(), [path("work/a.md")]);

        let settled = plan.settle(&facts, vec![read("work/a.md", Some("A"), stamp(5), false)]);
        assert_eq!(settled.removed, [path("a.md")]);
        assert_eq!(
            put_ids(&settled),
            [("work/a.md".to_owned(), Some("A"), false)]
        );
        assert_eq!(
            changes(&facts, settled),
            [Change::Moved {
                from: path("a.md"),
                note: info("work/a.md", Some("A")),
            }]
        );
    }

    #[test]
    fn a_known_file_keeps_its_id_against_a_new_file() {
        let facts = facts(&[("z.md", Some("A"), stamp(5))]);
        let files = vec![(path("a.md"), stamp(7)), (path("z.md"), stamp(6))];
        let plan = plan(&facts, &Nodes::root(), files);
        assert_eq!(plan.reads(), [path("z.md"), path("a.md")]);

        let settled = plan.settle(
            &facts,
            vec![
                read("a.md", Some("A"), stamp(7), true),
                read("z.md", Some("A"), stamp(6), false),
            ],
        );
        assert_eq!(settled.removed, Vec::<NotePath>::new());
        assert_eq!(
            put_ids(&settled),
            [
                ("z.md".to_owned(), Some("A"), false),
                ("a.md".to_owned(), None, true),
            ]
        );
    }

    #[test]
    fn a_new_file_takes_the_id_of_a_gone_file_outside_the_scope() {
        let facts = facts(&[("old.md", Some("A"), stamp(5))]);
        let nodes = Nodes::new([Node::Note(path("new.md"))]);
        let plan = plan(&facts, &nodes, vec![(path("new.md"), stamp(6))]);

        let settled = plan.settle(&facts, vec![read("new.md", Some("A"), stamp(6), false)]);
        assert_eq!(settled.removed, [path("old.md")]);
        assert_eq!(put_ids(&settled), [("new.md".to_owned(), Some("A"), false)]);
        assert_eq!(
            changes(&facts, settled),
            [Change::Moved {
                from: path("old.md"),
                note: info("new.md", Some("A")),
            }]
        );
    }

    #[test]
    fn a_failed_read_of_a_known_file_removes_its_note() {
        let facts = facts(&[("a.md", Some("A"), stamp(5))]);
        let plan = plan(&facts, &Nodes::root(), vec![(path("a.md"), stamp(6))]);
        assert_eq!(plan.reads(), [path("a.md")]);

        let settled = plan.settle(&facts, Vec::new());
        assert_eq!(settled.removed, [path("a.md")]);
        assert!(settled.put.is_empty());
        assert_eq!(changes(&facts, settled), [Change::Removed(path("a.md"))]);
    }

    #[test]
    fn a_file_with_its_old_stamp_is_not_read() {
        let facts = facts(&[("a.md", Some("A"), stamp(5))]);
        let plan = plan(&facts, &Nodes::root(), vec![(path("a.md"), stamp(5))]);
        assert_eq!(plan.reads(), Vec::<NotePath>::new());
        assert!(plan.settle(&facts, Vec::new()).put.is_empty());
    }
}
