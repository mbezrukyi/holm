use holm_core::testing::note_path;

use super::*;

fn save(path: &str, body: &str) -> NoteText {
    NoteText::Save {
        path: note_path(path),
        body: body.to_owned(),
    }
}

fn failed_read() -> holm_core::Result<Note> {
    Err(holm_core::Error::Read {
        path: "A.md".into(),
        source: std::io::ErrorKind::NotFound.into(),
    })
}

fn read_note(path: &str) -> Note {
    Note {
        info: NoteInfo {
            path: note_path(path),
            id: None,
            title: String::new(),
            preview: String::new(),
            tags: Vec::new(),
            pinned: false,
            modified: std::time::SystemTime::UNIX_EPOCH,
        },
        body: String::new(),
    }
}

fn set(draft: &mut Draft, path: &str, body: &str) {
    assert!(draft.set(note_path(path), body.into()).is_none());
}

fn one_write(draft: &mut Draft) -> DraftSave {
    let mut writes = draft.writes();
    assert_eq!(writes.len(), 1);
    writes.pop().unwrap()
}

#[test]
fn a_refused_write_holds_the_text_until_its_release() {
    let mut draft = Draft::default();
    set(&mut draft, "A.md", "one");
    let write = one_write(&mut draft);

    assert_eq!(draft.refused(write.id, write.version), Written::Changed);
    assert!(draft.unsaved());
    assert!(draft.kept_back());
    assert!(draft.writes().is_empty());

    draft.release(&note_path("A.md"));
    assert_eq!(one_write(&mut draft).text, save("A.md", "one"));
}

#[test]
fn a_refused_write_of_a_left_text_tells_nothing() {
    let mut draft = Draft::default();
    set(&mut draft, "A.md", "one");
    let old = draft.set(note_path("B.md"), "two".into()).unwrap();

    assert_eq!(draft.refused(old.id, old.version), Written::Stale);
    assert!(draft.kept_back());
    assert_eq!(one_write(&mut draft).text, save("B.md", "two"));
}

#[test]
fn the_line_holds_one_write_of_a_text() {
    let mut draft = Draft::default();
    assert!(draft.writes().is_empty(), "no text");
    set(&mut draft, "A.md", "one");
    let write = one_write(&mut draft);
    assert_eq!(write.text, save("A.md", "one"));
    assert!(draft.writes().is_empty());

    assert_eq!(draft.written(write.id, write.version, None), Written::Saved);
    assert!(!draft.unsaved());
    assert!(draft.writes().is_empty());
}

#[test]
fn a_newer_text_stays_after_the_write_of_an_older_one() {
    let mut draft = Draft::default();
    set(&mut draft, "A.md", "one");
    let write = one_write(&mut draft);
    set(&mut draft, "A.md", "two");

    assert_eq!(draft.written(write.id, write.version, None), Written::Saved);
    assert!(draft.unsaved());
    assert_eq!(one_write(&mut draft).text, save("A.md", "two"));
}

#[test]
fn a_failed_write_keeps_the_text_for_the_next_write() {
    let mut draft = Draft::default();
    set(&mut draft, "A.md", "one");
    let write = one_write(&mut draft);

    assert_eq!(
        draft.written(write.id, write.version, Some("disk full")),
        Written::Failed { first: true }
    );
    assert!(draft.unsaved());
    assert_eq!(draft.not_saved(), Some("disk full"));
    let again = one_write(&mut draft);
    assert_eq!(again.text, save("A.md", "one"));

    assert_eq!(draft.written(again.id, again.version, None), Written::Saved);
    assert_eq!(draft.not_saved(), None);
}

#[test]
fn only_the_first_failed_write_is_first() {
    let mut draft = Draft::default();
    set(&mut draft, "A.md", "one");
    let write = one_write(&mut draft);
    assert_eq!(
        draft.written(write.id, write.version, Some("disk full")),
        Written::Failed { first: true }
    );
    let again = one_write(&mut draft);
    assert_eq!(
        draft.written(again.id, again.version, Some("no access")),
        Written::Failed { first: false }
    );
    assert_eq!(draft.not_saved(), Some("no access"), "the last reason");
}

#[test]
fn a_saved_open_text_keeps_the_reason_of_a_failed_left_text() {
    let mut draft = Draft::default();
    set(&mut draft, "A.md", "one");
    let old = draft.set(note_path("B.md"), "two".into()).unwrap();
    let open = one_write(&mut draft);
    let _ = draft.written(old.id, old.version, Some("disk full"));
    assert_eq!(draft.written(open.id, open.version, None), Written::Saved);
    assert_eq!(draft.not_saved(), Some("disk full"));
}

#[test]
fn the_end_of_a_write_of_dropped_text_means_nothing() {
    let mut draft = Draft::default();
    set(&mut draft, "A.md", "one");
    let write = one_write(&mut draft);
    draft.drop_text(&note_path("A.md"));
    assert_eq!(
        draft.written(write.id, write.version, Some("disk full")),
        Written::Stale
    );
    assert!(!draft.failed());
    assert!(!draft.unsaved());
}

#[test]
fn a_text_of_another_note_gives_the_write_of_the_old_text_first() {
    let mut draft = Draft::default();
    set(&mut draft, "A.md", "one");
    let old = draft.set(note_path("B.md"), "two".into()).unwrap();
    assert_eq!(old.text, save("A.md", "one"));
    assert_eq!(one_write(&mut draft).text, save("B.md", "two"));

    assert_eq!(
        draft.written(old.id, old.version, Some("disk full")),
        Written::Failed { first: true }
    );
    assert!(draft.unsaved_note(&note_path("A.md")));
    assert!(draft.failed());
    let writes: Vec<NoteText> = draft.writes().into_iter().map(|write| write.text).collect();
    assert_eq!(writes, [save("A.md", "one")], "B is in the line");
}

#[test]
fn the_old_text_is_one_write_when_its_save_is_in_the_line() {
    let mut draft = Draft::default();
    set(&mut draft, "A.md", "one");
    let write = one_write(&mut draft);
    assert!(draft.set(note_path("B.md"), "two".into()).is_none());

    assert_eq!(draft.written(write.id, write.version, None), Written::Stale);
    assert!(!draft.unsaved_note(&note_path("A.md")));
    assert!(draft.unsaved_note(&note_path("B.md")));
}

#[test]
fn a_left_text_comes_back_when_the_editor_takes_its_note_again() {
    let mut draft = Draft::default();
    set(&mut draft, "A.md", "one");
    let old = draft.set(note_path("B.md"), "two".into()).unwrap();
    let _ = draft.set(note_path("A.md"), "three".into());

    assert_eq!(draft.written(old.id, old.version, None), Written::Saved);
    assert!(draft.unsaved_note(&note_path("A.md")), "a newer body came");
    assert_eq!(
        draft.take(),
        [save("B.md", "two"), save("A.md", "three")],
        "the left texts first"
    );
}

#[test]
fn each_text_follows_a_move_of_its_note() {
    let mut draft = Draft::default();
    set(&mut draft, "A.md", "one");
    let _ = draft.set(note_path("B.md"), "two".into());
    let moved = |from: &str, to: &str| Change::Moved {
        from: note_path(from),
        to: note_path(to),
    };
    draft.follow(Source::Holm, &moved("A.md", "work/A.md"), &Snapshot::default());
    draft.follow(Source::Holm, &moved("B.md", "work/B.md"), &Snapshot::default());
    assert_eq!(draft.take(), [save("work/A.md", "one"), save("work/B.md", "two")]);
}

#[test]
fn a_held_text_gives_no_write_until_its_release() {
    let mut draft = Draft::default();
    set(&mut draft, "A.md", "one");
    draft.hold(&note_path("A.md"));
    set(&mut draft, "A.md", "two");
    assert!(draft.writes().is_empty());
    assert!(draft.unsaved());
    assert!(draft.kept_back());
    assert!(!draft.failed());

    draft.release(&note_path("A.md"));
    assert!(!draft.kept_back());
    let write = one_write(&mut draft);
    assert_eq!(write.text, save("A.md", "two"));
    assert_eq!(draft.written(write.id, write.version, None), Written::Saved);
    assert!(!draft.unsaved());
}

#[test]
fn a_held_text_gives_no_write_when_the_editor_takes_another_note() {
    let mut draft = Draft::default();
    set(&mut draft, "A.md", "one");
    draft.hold(&note_path("A.md"));
    assert!(draft.set(note_path("B.md"), "two".into()).is_none());
    assert_eq!(one_write(&mut draft).text, save("B.md", "two"));
    assert!(draft.kept_back());
}

#[test]
fn a_drop_ends_the_hold_with_the_text() {
    let mut draft = Draft::default();
    set(&mut draft, "A.md", "one");
    draft.hold(&note_path("A.md"));
    draft.drop_text(&note_path("A.md"));
    assert!(!draft.kept_back());

    set(&mut draft, "A.md", "two");
    assert_eq!(one_write(&mut draft).text, save("A.md", "two"));
}

#[test]
fn the_hold_follows_a_move_of_its_note() {
    let mut draft = Draft::default();
    set(&mut draft, "A.md", "one");
    draft.hold(&note_path("A.md"));
    draft.follow(
        Source::Outside,
        &Change::Moved {
            from: note_path("A.md"),
            to: note_path("work/A.md"),
        },
        &Snapshot::default(),
    );
    assert!(draft.writes().is_empty());
    draft.release(&note_path("work/A.md"));
    assert_eq!(one_write(&mut draft).text, save("work/A.md", "one"));
}

#[test]
fn a_failed_read_gives_the_text_back_with_its_hold() {
    let mut draft = Draft::default();
    set(&mut draft, "A.md", "one");
    draft.hold(&note_path("A.md"));
    let id = draft.drop_for_read(&note_path("A.md")).unwrap();
    assert!(!draft.unsaved());
    assert!(draft.writes().is_empty());

    draft.read_ended(id, &failed_read());
    assert!(draft.unsaved_note(&note_path("A.md")));
    assert!(draft.writes().is_empty(), "the hold came back");
    draft.release(&note_path("A.md"));
    assert_eq!(one_write(&mut draft).text, save("A.md", "one"));
}

#[test]
fn a_read_that_works_drops_the_text() {
    let mut draft = Draft::default();
    set(&mut draft, "A.md", "one");
    let id = draft.drop_for_read(&note_path("A.md")).unwrap();
    draft.read_ended(id, &Ok(read_note("A.md")));
    assert!(!draft.unsaved());
    assert_eq!(draft.take(), []);
}

#[test]
fn a_newer_text_during_a_failed_read_stays_and_takes_the_hold() {
    let mut draft = Draft::default();
    set(&mut draft, "A.md", "one");
    draft.hold(&note_path("A.md"));
    let id = draft.drop_for_read(&note_path("A.md")).unwrap();
    set(&mut draft, "A.md", "one two");

    draft.read_ended(id, &failed_read());
    assert!(draft.writes().is_empty(), "the newer text is held");
    assert_eq!(draft.take(), [save("A.md", "one two")]);
}

#[test]
fn a_drop_during_a_read_keeps_the_text_away() {
    let mut draft = Draft::default();
    set(&mut draft, "A.md", "one");
    let id = draft.drop_for_read(&note_path("A.md")).unwrap();
    draft.drop_text(&note_path("A.md"));
    draft.read_ended(id, &failed_read());
    assert!(!draft.unsaved());
}

#[test]
fn a_text_in_a_read_follows_a_move_of_its_note() {
    let mut draft = Draft::default();
    set(&mut draft, "A.md", "one");
    let id = draft.drop_for_read(&note_path("A.md")).unwrap();
    draft.follow(
        Source::Outside,
        &Change::Moved {
            from: note_path("A.md"),
            to: note_path("work/A.md"),
        },
        &Snapshot::default(),
    );
    draft.read_ended(id, &failed_read());
    assert_eq!(draft.take(), [save("work/A.md", "one")]);
}

#[test]
fn the_last_writes_at_quit_take_a_held_text() {
    let mut draft = Draft::default();
    set(&mut draft, "A.md", "one");
    draft.hold(&note_path("A.md"));
    assert_eq!(draft.take(), [save("A.md", "one")]);
}
