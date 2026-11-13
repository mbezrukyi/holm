//! The snapshot after each write and each outside change, and its revision.

#![allow(clippy::unwrap_used, reason = "tests")]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::Folder;
use holm_core::model::{NoteInfo, Order};
use holm_core::{Core, Snapshot, Written};

fn titles(snapshot: &Snapshot) -> Vec<String> {
    snapshot
        .notes(Order::Title)
        .into_iter()
        .map(|note| note.title)
        .collect()
}

fn paths(notes: Vec<NoteInfo>) -> Vec<String> {
    common::paths(notes.into_iter().map(|note| note.path))
}

fn folders(snapshot: &Snapshot) -> Vec<String> {
    common::paths(snapshot.folders().map(Path::to_path_buf))
}

fn tag_names(snapshot: &Snapshot) -> Vec<String> {
    snapshot.tags().into_iter().map(|tag| tag.name).collect()
}

/// Checks that the write grew the revision by one, and that the core gives out its snapshot.
fn one_step<T>(core: &Core, write: impl FnOnce(&Core) -> Written<T>) -> (T, Snapshot) {
    let before = core.snapshot().revision();
    let written = write(core);
    let result = written.result.unwrap();
    assert_eq!(written.snapshot.revision(), before + 1);
    assert_eq!(core.snapshot().revision(), before + 1);
    (result, written.snapshot)
}

/// Checks that the write left the revision as it was.
fn no_step<T: std::fmt::Debug>(core: &Core, write: impl FnOnce(&Core) -> Written<T>) -> Written<T> {
    let before = core.snapshot().revision();
    let written = write(core);
    assert_eq!(written.snapshot.revision(), before);
    assert_eq!(core.snapshot().revision(), before);
    written
}

#[test]
fn note_writes_show_in_the_snapshot_after_them() {
    let folder = Folder::new(&[
        ("work/.keep.md", ""),
        ("Plans.md", "---\nid: P\n---\n# Plans\n"),
    ]);
    let core = &folder.core;

    let (note, snapshot) = one_step(core, |core| core.create("work"));
    assert_eq!(
        paths(snapshot.notes_in("work", Order::Title)),
        ["work/Untitled.md"]
    );

    let mut text = note.clone();
    text.body = "# Ideas\n[plans](holm:P)\n".to_owned();
    let (_, snapshot) = one_step(core, |core| core.save(&text));
    assert_eq!(titles(&snapshot), ["Ideas", "Plans"]);
    assert_eq!(
        paths(snapshot.backlinks("Plans.md", Order::Title)),
        ["work/Untitled.md"]
    );

    let (info, snapshot) = one_step(core, |core| core.rename_to_title("work/Untitled.md"));
    assert_eq!(info.path, Path::new("work/Ideas.md"));
    assert_eq!(
        paths(snapshot.notes_in("work", Order::Title)),
        ["work/Ideas.md"]
    );

    let (_, snapshot) = one_step(core, |core| core.move_note("work/Ideas.md", ""));
    assert_eq!(
        paths(snapshot.notes_in("", Order::Title)),
        ["Ideas.md", "Plans.md"]
    );
    assert_eq!(
        paths(snapshot.backlinks("Plans.md", Order::Title)),
        ["Ideas.md"]
    );

    let (_, snapshot) = one_step(core, |core| core.set_tags("Ideas.md", &["work/holm"]));
    assert_eq!(tag_names(&snapshot), ["work"]);
    assert_eq!(
        paths(snapshot.notes_with_tag("work", Order::Title)),
        ["Ideas.md"]
    );

    let (_, snapshot) = one_step(core, |core| core.set_pinned("Plans.md", true));
    assert_eq!(titles(&snapshot), ["Plans", "Ideas"]);

    let (count, snapshot) = one_step(core, |core| core.rename_tag("work", "job"));
    assert_eq!(count, 1);
    assert_eq!(tag_names(&snapshot), ["job"]);

    let (_, snapshot) = one_step(core, |core| core.delete_tag("job"));
    assert_eq!(tag_names(&snapshot), Vec::<String>::new());

    let ((), snapshot) = one_step(core, |core| core.delete_note("Ideas.md"));
    assert_eq!(titles(&snapshot), ["Plans"]);
    assert_eq!(snapshot.backlinks("Plans.md", Order::Title), []);

    let plans = core.read("Plans.md").unwrap();
    fs::remove_file(folder.path("Plans.md")).unwrap();
    let (_, snapshot) = one_step(core, |core| core.restore(&plans));
    assert_eq!(
        snapshot.note_with_id("P").unwrap().path,
        Path::new("Plans.md")
    );
}

#[test]
fn link_id_shows_the_new_id_in_the_snapshot() {
    let folder = Folder::new(&[("a.md", "# A\n")]);

    let (id, snapshot) = one_step(&folder.core, |core| core.link_id("a.md"));

    assert_eq!(
        snapshot.note_with_id(id.as_str()).unwrap().path,
        Path::new("a.md")
    );
}

#[test]
fn folder_writes_show_in_the_snapshot_after_them() {
    let folder = Folder::new(&[("work/a.md", "# A\n"), ("work/old/b.md", "# B\n")]);
    let core = &folder.core;

    let (new, snapshot) = one_step(core, |core| core.create_folder("", "Home"));
    assert_eq!(new, PathBuf::from("Home"));
    assert_eq!(folders(&snapshot), ["Home", "work", "work/old"]);

    let (_, snapshot) = one_step(core, |core| core.rename_folder("work", "job"));
    assert_eq!(folders(&snapshot), ["Home", "job", "job/old"]);
    assert_eq!(
        paths(snapshot.notes_in("job/old", Order::Title)),
        ["job/old/b.md"]
    );

    let (_, snapshot) = one_step(core, |core| core.move_folder("job/old", "Home"));
    assert_eq!(folders(&snapshot), ["Home", "Home/old", "job"]);

    let ((), snapshot) = one_step(core, |core| core.delete_folder("Home"));
    assert_eq!(folders(&snapshot), ["job"]);
    assert_eq!(titles(&snapshot), ["A"]);
}

#[test]
fn a_change_through_the_feed_shows_in_the_snapshot() {
    let folder = Folder::with_feed(&[("a.md", "# A\n")]);
    let before = folder.core.snapshot().revision();

    folder.write("work/b.md", "---\ntags: [home]\n---\n# B\n");
    folder.feed(["work"]);

    let snapshot = folder.core.snapshot();
    assert_eq!(snapshot.revision(), before + 1);
    assert_eq!(titles(&snapshot), ["A", "B"]);
    assert_eq!(folders(&snapshot), ["work"]);
    assert_eq!(tag_names(&snapshot), ["home"]);
}

#[test]
fn a_feed_batch_with_no_change_keeps_the_revision() {
    let folder = Folder::with_feed(&[("a.md", "# A\n")]);
    let before = folder.core.snapshot().revision();

    folder.feed(["a.md", "other.txt", ""]);

    assert_eq!(folder.core.snapshot().revision(), before);
}

#[test]
fn a_save_with_the_same_text_keeps_the_revision() {
    let folder = Folder::new(&[("A.md", "---\nid: A\ntags: [x]\n---\n# A\n")]);
    let note = folder.core.read("A.md").unwrap();

    let written = no_step(&folder.core, |core| core.save(&note));

    assert_eq!(written.result.unwrap(), note);
    no_step(&folder.core, |core| core.set_tags("A.md", &["x"]))
        .result
        .unwrap();
    no_step(&folder.core, |core| core.rename_to_title("A.md"))
        .result
        .unwrap();
}

#[test]
fn a_failed_write_keeps_the_revision() {
    let folder = Folder::new(&[("a.md", "# A\n")]);

    let written = no_step(&folder.core, |core| core.set_tags("missing.md", &["x"]));
    assert!(written.result.is_err());
    let written = no_step(&folder.core, |core| core.create_folder("missing", "x"));
    assert!(written.result.is_err());
    let written = no_step(&folder.core, |core| core.rename_tag("x", ""));
    assert!(written.result.is_err());
}

#[test]
fn an_old_snapshot_stays_as_it_was() {
    let folder = Folder::new(&[("a.md", "# A\n")]);
    let old = folder.core.snapshot();

    folder.core.create("").result.unwrap();

    assert_eq!(titles(&old), ["A"]);
    assert_eq!(titles(&folder.core.snapshot()), ["A", "Untitled"]);
}
