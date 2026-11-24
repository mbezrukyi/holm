use std::time::SystemTime;

use super::*;
use crate::Revision;
use crate::links::LinkSource;
use crate::store::{ContentHash, Stamp};
use crate::tag::TagName;

const STAMP: Stamp = Stamp {
    modified: 1,
    changed: 1,
    size: 1,
};

fn path(path: &str) -> NotePath {
    NotePath::new(path).unwrap()
}

fn folder(path: &str) -> FolderPath {
    FolderPath::new(path).unwrap()
}

fn info(at: &str, id: Option<&str>, tags: &[&str]) -> NoteInfo {
    NoteInfo {
        path: path(at),
        id: id.map(|id| NoteId::new(id.to_owned())),
        title: String::new(),
        preview: String::new(),
        tags: tags.iter().map(|tag| TagName::new(tag).unwrap()).collect(),
        pinned: false,
        modified: SystemTime::UNIX_EPOCH,
    }
}

fn facts(notes: &[NoteInfo], folders: &[&str]) -> Facts {
    Facts::new(
        notes
            .iter()
            .map(|note| {
                (
                    note.clone(),
                    Vec::new(),
                    STAMP,
                    ContentHash::default(),
                    LinkSource::default(),
                )
            })
            .collect(),
        folders.iter().map(|at| folder(at)).collect(),
    )
}

fn changes(before: &Facts, after: Facts, notes: &[&str], folders: &[&str], moves: Vec<Change>) -> Vec<Change> {
    let touched = Touched {
        notes: notes.iter().map(|at| path(at)).collect(),
        read: Vec::new(),
        folders: folders.iter().map(|at| folder(at)).collect(),
        moves,
    };
    let (_, list) = touched.list(Source::Holm, before, after);
    list.map_or_default(|list| list.changes)
}

#[test]
fn no_change_gives_no_list_and_keeps_the_revision() {
    let before = facts(&[info("a.md", None, &[])], &["work"]);
    let touched = Touched {
        notes: vec![path("a.md")],
        read: Vec::new(),
        folders: vec![folder("work")],
        moves: Vec::new(),
    };

    let (after, list) = touched.list(Source::Holm, &before, before.clone());

    assert!(list.is_none());
    assert_eq!(after.snapshot().revision(), before.snapshot().revision());
}

#[test]
fn a_list_carries_its_source_and_the_next_revision() {
    let before = facts(&[], &[]);
    let after = facts(&[info("a.md", None, &[])], &[]);
    let touched = Touched {
        notes: vec![path("a.md")],
        ..Touched::default()
    };

    let (after, list) = touched.list(Source::Outside, &before, after);

    let list = list.unwrap();
    assert_eq!(list.source, Source::Outside);
    assert_eq!(list.snapshot.revision(), before.snapshot().revision().next());
    assert_eq!(after.snapshot().revision(), list.snapshot.revision());
    assert_eq!(list.changes, [Change::Added(path("a.md"))]);
}

#[test]
fn notes_come_in_path_order_and_tags_once() {
    let before = facts(&[info("b.md", None, &["x"]), info("c.md", None, &[])], &[]);
    let after = facts(&[info("a.md", None, &["y"]), info("c.md", None, &["x"])], &[]);

    assert_eq!(
        changes(&before, after, &["a.md", "b.md", "c.md"], &[], Vec::new()),
        [
            Change::Added(path("a.md")),
            Change::Removed(path("b.md")),
            Change::Changed(path("c.md")),
            Change::Tags,
        ]
    );
}

#[test]
fn a_gone_note_and_a_new_note_with_one_id_are_a_move() {
    let before = facts(&[info("a.md", Some("A"), &[]), info("b.md", None, &[])], &[]);
    let after = facts(&[info("x/a.md", Some("A"), &[]), info("x/b.md", None, &[])], &[]);
    let notes = ["a.md", "b.md", "x/a.md", "x/b.md"];

    assert_eq!(
        changes(&before, after, &notes, &[], Vec::new()),
        [
            Change::Removed(path("b.md")),
            Change::Moved {
                from: path("a.md"),
                to: path("x/a.md"),
            },
            Change::Added(path("x/b.md")),
        ]
    );
}

#[test]
fn a_move_of_the_write_explains_its_notes() {
    let before = facts(&[info("a.md", None, &[]), info("work/b.md", None, &["x"])], &["work"]);
    let after = facts(&[info("c.md", None, &[]), info("job/b.md", None, &["x"])], &["job"]);
    let moves = vec![
        Change::Moved {
            from: path("a.md"),
            to: path("c.md"),
        },
        Change::FolderMoved {
            from: folder("work"),
            to: folder("job"),
        },
    ];
    let notes = ["a.md", "c.md", "work/b.md", "job/b.md"];

    assert_eq!(
        changes(&before, after, &notes, &["work", "job"], moves.clone()),
        [moves, vec![Change::Folders]].concat()
    );
}

/// Facts with one note at `a.md`, whose file holds `text`.
fn with_hash(text: &str) -> Facts {
    let note = (
        info("a.md", None, &[]),
        Vec::new(),
        STAMP,
        ContentHash::of(text),
        LinkSource::default(),
    );
    Facts::new(vec![note], Vec::new())
}

#[test]
fn a_new_content_hash_is_a_change_with_the_same_info() {
    let before = with_hash("one");

    assert_eq!(
        changes(&before, with_hash("two"), &["a.md"], &[], Vec::new()),
        [Change::Changed(path("a.md"))]
    );
    assert_eq!(changes(&before, with_hash("one"), &["a.md"], &[], Vec::new()), []);
}

#[test]
fn only_a_read_with_a_new_content_hash_marks_an_outside_change() {
    let before = with_hash("one");
    let touched = || Touched {
        notes: vec![path("a.md")],
        read: vec![path("a.md")],
        ..Touched::default()
    };

    let (same, _) = touched().list(Source::Outside, &before, with_hash("one"));
    assert!(!same.changed_after(&path("a.md"), Revision::default()));
    let (new, _) = touched().list(Source::Outside, &before, with_hash("two"));
    assert!(new.changed_after(&path("a.md"), Revision::default()));
}

#[test]
fn folders_change_only_inside_the_touched_folders() {
    let before = facts(&[], &["a", "b"]);
    let after = facts(&[], &["a", "a/x", "c"]);

    assert_eq!(
        changes(&before, after.clone(), &[], &["a"], Vec::new()),
        [Change::Folders]
    );
    assert_eq!(changes(&before, after, &[], &["d"], Vec::new()), []);
}
