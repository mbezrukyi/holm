use std::time::SystemTime;

use super::*;
use crate::diff::Touched;
use crate::model::{Change, FileTarget, Source};
use crate::nodes::Node;
use crate::paths::{FolderPath, RawPath};

fn changes(before: &Facts, settled: Settled) -> Vec<Change> {
    let mut touched = Touched::default();
    touched.settled(&settled);
    let mut after = before.clone();
    after.apply(settled);
    let (_, list) = touched.list(Source::Outside, before, after);
    list.map_or_default(|list| list.changes)
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
        id: id.map(|id| NoteId::new(id.to_owned())),
        title: String::new(),
        preview: String::new(),
        tags: Vec::new(),
        pinned: false,
        modified: SystemTime::UNIX_EPOCH,
    }
}

fn facts(notes: &[(&str, Option<&str>, Stamp)]) -> Facts {
    Facts::new(
        notes
            .iter()
            .map(|(at, id, stamp)| {
                let hash = ContentHash::default();
                (info(at, *id), Vec::new(), *stamp, hash, LinkSource::default())
            })
            .collect(),
        Vec::new(),
    )
}

fn read(at: &str, id: Option<&str>, stamp: Stamp, owner: Owner) -> Read {
    Read {
        note: Note {
            info: info(at, id),
            body: String::new(),
        },
        links: Vec::new(),
        stamp,
        hash: ContentHash::default(),
        source: LinkSource::default(),
        owner,
    }
}

fn put_ids(settled: &Settled) -> Vec<(String, Option<&str>, bool)> {
    settled
        .put
        .iter()
        .map(|put| {
            let info = &put.note.info;
            (info.path.to_string(), info.id.as_ref().map(NoteId::as_str), put.copy)
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

    let settled = plan.decide(&facts, Vec::new());
    assert_eq!(settled.removed, Vec::<NotePath>::new());
    assert_eq!(settled.moved.len(), 1);
    assert_eq!(settled.moved[0].from, path("a.md"));
    assert_eq!(settled.moved[0].info, info("work/a.md", Some("A")));
    assert_eq!(
        changes(&facts, settled),
        [Change::Moved {
            from: path("a.md"),
            to: path("work/a.md"),
        }]
    );
}

#[test]
fn two_gone_files_with_one_name_and_stamp_are_no_move() {
    let facts = facts(&[("x/a.md", None, stamp(5)), ("y/a.md", None, stamp(5))]);
    let plan = plan(&facts, &Nodes::root(), vec![(path("z/a.md"), stamp(5))]);
    assert_eq!(plan.reads(), [path("z/a.md")]);

    let settled = plan.decide(&facts, vec![read("z/a.md", None, stamp(5), Owner::Absent)]);
    assert!(settled.moved.is_empty());
    assert_eq!(sorted(&settled.removed), ["x/a.md", "y/a.md"]);
    assert_eq!(put_ids(&settled), [("z/a.md".to_owned(), None, false)]);
}

#[test]
fn a_note_with_a_file_link_is_read_at_its_new_path() {
    let link = LinkTarget::File(FileTarget::Inside(RawPath::new("b.png").unwrap()));
    let facts = Facts::new(
        vec![(
            info("a.md", Some("A")),
            vec![link],
            stamp(5),
            ContentHash::default(),
            LinkSource::default(),
        )],
        Vec::new(),
    );
    let plan = plan(&facts, &Nodes::root(), vec![(path("work/a.md"), stamp(5))]);
    assert_eq!(plan.reads(), [path("work/a.md")]);

    let settled = plan.decide(&facts, vec![read("work/a.md", Some("A"), stamp(5), Owner::Absent)]);
    assert_eq!(settled.removed, [path("a.md")]);
    assert_eq!(put_ids(&settled), [("work/a.md".to_owned(), Some("A"), false)]);
    assert_eq!(
        changes(&facts, settled),
        [Change::Moved {
            from: path("a.md"),
            to: path("work/a.md"),
        }]
    );
}

#[test]
fn a_known_file_keeps_its_id_against_a_new_file() {
    let facts = facts(&[("z.md", Some("A"), stamp(5))]);
    let files = vec![(path("a.md"), stamp(7)), (path("z.md"), stamp(6))];
    let plan = plan(&facts, &Nodes::root(), files);
    assert_eq!(plan.reads(), [path("z.md"), path("a.md")]);

    let settled = plan.decide(
        &facts,
        vec![
            read("a.md", Some("A"), stamp(7), Owner::OnDisk),
            read("z.md", Some("A"), stamp(6), Owner::Absent),
        ],
    );
    assert_eq!(settled.removed, Vec::<NotePath>::new());
    assert_eq!(
        put_ids(&settled),
        [("z.md".to_owned(), Some("A"), false), ("a.md".to_owned(), None, true),]
    );
}

#[test]
fn a_new_file_takes_the_id_of_a_gone_file_outside_the_scope() {
    let facts = facts(&[("old.md", Some("A"), stamp(5))]);
    let nodes = Nodes::new([Node::Note(path("new.md"))]);
    let plan = plan(&facts, &nodes, vec![(path("new.md"), stamp(6))]);

    let settled = plan.decide(&facts, vec![read("new.md", Some("A"), stamp(6), Owner::Absent)]);
    assert_eq!(settled.removed, [path("old.md")]);
    assert_eq!(put_ids(&settled), [("new.md".to_owned(), Some("A"), false)]);
    assert_eq!(
        changes(&facts, settled),
        [Change::Moved {
            from: path("old.md"),
            to: path("new.md"),
        }]
    );
}

#[test]
fn a_failed_read_of_a_known_file_removes_its_note() {
    let facts = facts(&[("a.md", Some("A"), stamp(5))]);
    let plan = plan(&facts, &Nodes::root(), vec![(path("a.md"), stamp(6))]);
    assert_eq!(plan.reads(), [path("a.md")]);

    let settled = plan.decide(&facts, Vec::new());
    assert_eq!(settled.removed, [path("a.md")]);
    assert!(settled.put.is_empty());
    assert_eq!(changes(&facts, settled), [Change::Removed(path("a.md"))]);
}

#[test]
fn a_named_note_is_read_with_its_old_stamp() {
    let facts = facts(&[("a.md", Some("A"), stamp(5)), ("b.md", None, stamp(5))]);
    let nodes = Nodes::new([Node::Note(path("a.md")), Node::Folder(FolderPath::root())]);
    let named = Nodes::new([Node::Note(path("a.md"))]);
    let files = vec![(path("a.md"), stamp(5))];

    assert_eq!(plan(&facts, &named, files.clone()).reads(), [path("a.md")]);
    assert_eq!(
        plan(&facts, &nodes, files).reads(),
        Vec::<NotePath>::new(),
        "a folder keeps the stamp shortcut"
    );
}

#[test]
fn a_file_with_its_old_stamp_is_not_read() {
    let facts = facts(&[("a.md", Some("A"), stamp(5))]);
    let plan = plan(&facts, &Nodes::root(), vec![(path("a.md"), stamp(5))]);
    assert_eq!(plan.reads(), Vec::<NotePath>::new());
    assert!(plan.decide(&facts, Vec::new()).put.is_empty());
}
