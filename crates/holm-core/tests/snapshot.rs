#![cfg(test)]

use std::fs;

use holm_core::model::{NoteInfo, NoteSet, Order, Tag};
use holm_core::{Core, NotePath, Result, Snapshot, TagName};
use holm_core_test::{Folder, folder_path, note_path, tag_name};

fn titles(snapshot: &Snapshot) -> Vec<String> {
    snapshot
        .notes(NoteSet::All, Order::Title)
        .into_iter()
        .map(|note| note.title)
        .collect()
}

fn paths(notes: Vec<NoteInfo>) -> Vec<String> {
    holm_core_test::paths(notes.into_iter().map(|note| note.path))
}

fn folders(snapshot: &Snapshot) -> Vec<String> {
    holm_core_test::paths(snapshot.folders())
}

fn tag_names(snapshot: &Snapshot) -> Vec<TagName> {
    snapshot.tags().into_iter().map(|tag| tag.name).collect()
}

/// Checks that the write grew the revision by one, and gives the snapshot after it.
fn one_step<T>(core: &Core, write: impl FnOnce(&Core) -> Result<T>) -> (T, Snapshot) {
    let before = core.snapshot().revision();
    let result = write(core).unwrap();
    let snapshot = core.snapshot();
    assert_eq!(snapshot.revision(), before + 1);
    (result, snapshot)
}

/// Checks that the write left the revision as it was.
fn no_step<T>(core: &Core, write: impl FnOnce(&Core) -> Result<T>) -> Result<T> {
    let before = core.snapshot().revision();
    let result = write(core);
    assert_eq!(core.snapshot().revision(), before);
    result
}

#[test]
fn note_writes_show_in_the_snapshot_after_them() {
    let folder = Folder::new(&[("work/.keep.md", ""), ("Plans.md", "---\nid: P\n---\n# Plans\n")]);
    let core = &folder.core;

    let (note, snapshot) = one_step(core, |core| core.create(&folder_path("work"), "Untitled"));
    assert_eq!(
        paths(snapshot.notes(NoteSet::In(&folder_path("work")), Order::Title)),
        ["work/Untitled.md"]
    );

    let (_, snapshot) = one_step(core, |core| {
        core.save(&note.info.path, "# Ideas\n[plans](holm:P)\n", None)
    });
    assert_eq!(titles(&snapshot), ["Plans", "Untitled"]);
    assert_eq!(paths(snapshot.backlinks(&note_path("Plans.md"))), ["work/Untitled.md"]);

    let (info, snapshot) = one_step(core, |core| core.rename_note(&note_path("work/Untitled.md"), "Ideas"));
    assert_eq!(info.path, note_path("work/Ideas.md"));
    assert_eq!(
        paths(snapshot.notes(NoteSet::In(&folder_path("work")), Order::Title)),
        ["work/Ideas.md"]
    );

    let (_, snapshot) = one_step(core, |core| {
        core.move_note(&note_path("work/Ideas.md"), &folder_path(""))
    });
    assert_eq!(
        paths(snapshot.notes(NoteSet::In(&folder_path("")), Order::Title)),
        ["Ideas.md", "Plans.md"]
    );
    assert_eq!(paths(snapshot.backlinks(&note_path("Plans.md"))), ["Ideas.md"]);

    let (_, snapshot) = one_step(core, |core| {
        core.add_tag(&note_path("Ideas.md"), &tag_name("work/holm"))
    });
    assert_eq!(tag_names(&snapshot), ["work"]);
    assert_eq!(
        paths(snapshot.notes(NoteSet::WithTag(&tag_name("work")), Order::Title)),
        ["Ideas.md"]
    );

    let (_, snapshot) = one_step(core, |core| core.set_pinned(&note_path("Plans.md"), true));
    assert_eq!(titles(&snapshot), ["Plans", "Ideas"]);

    let (count, snapshot) = one_step(core, |core| core.rename_tag(&tag_name("work"), &tag_name("home")));
    assert_eq!(count, 1);
    assert_eq!(tag_names(&snapshot), ["home"]);

    let (_, snapshot) = one_step(core, |core| core.delete_tag(&tag_name("home")));
    assert_eq!(tag_names(&snapshot), [] as [TagName; 0]);

    let ((), snapshot) = one_step(core, |core| core.delete_note(&note_path("Ideas.md")));
    assert_eq!(titles(&snapshot), ["Plans"]);
    assert_eq!(snapshot.backlinks(&note_path("Plans.md")), []);

    let plans = core.read(&note_path("Plans.md")).unwrap();
    fs::remove_file(folder.path("Plans.md")).unwrap();
    let (_, snapshot) = one_step(core, |core| core.restore(&plans));
    assert_eq!(snapshot.note_with_id("P").unwrap().path, note_path("Plans.md"));
}

#[test]
fn link_id_shows_the_new_id_in_the_snapshot() {
    let folder = Folder::new(&[("a.md", "# A\n")]);

    let (id, snapshot) = one_step(&folder.core, |core| core.link_id(&note_path("a.md")));

    assert_eq!(snapshot.note_with_id(id.as_str()).unwrap().path, note_path("a.md"));
}

#[test]
fn folder_writes_show_in_the_snapshot_after_them() {
    let folder = Folder::new(&[("work/a.md", "# A\n"), ("work/old/b.md", "# B\n")]);
    let core = &folder.core;

    let (new, snapshot) = one_step(core, |core| core.create_folder(&folder_path(""), "Home"));
    assert_eq!(new, folder_path("Home"));
    assert_eq!(folders(&snapshot), ["Home", "work", "work/old"]);

    let (_, snapshot) = one_step(core, |core| core.rename_folder(&folder_path("work"), "job"));
    assert_eq!(folders(&snapshot), ["Home", "job", "job/old"]);
    assert_eq!(
        paths(snapshot.notes(NoteSet::In(&folder_path("job/old")), Order::Title)),
        ["job/old/b.md"]
    );

    let (_, snapshot) = one_step(core, |core| {
        core.move_folder(&folder_path("job/old"), &folder_path("Home"))
    });
    assert_eq!(folders(&snapshot), ["Home", "Home/old", "job"]);

    let ((), snapshot) = one_step(core, |core| core.delete_folder(&folder_path("Home")));
    assert_eq!(folders(&snapshot), ["job"]);
    assert_eq!(titles(&snapshot), ["a"]);
}

#[test]
fn a_change_through_the_feed_shows_in_the_snapshot() {
    let folder = Folder::with_feed(&[("a.md", "# A\n")]);
    let before = folder.core.snapshot().revision();

    folder.write("work/b.md", "---\ntags: [home]\n---\n# B\n");
    folder.feed(["work"]);

    let snapshot = folder.core.snapshot();
    assert_eq!(snapshot.revision(), before + 1);
    assert_eq!(titles(&snapshot), ["a", "b"]);
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
    let note = folder.core.read(&note_path("A.md")).unwrap();

    let written = no_step(&folder.core, |core| core.save(&note_path("A.md"), &note.body, None));

    assert_eq!(written.unwrap(), note);
    no_step(&folder.core, |core| core.add_tag(&note_path("A.md"), &tag_name("x"))).unwrap();
    no_step(&folder.core, |core| core.remove_tag(&note_path("A.md"), &tag_name("y"))).unwrap();
    no_step(&folder.core, |core| core.rename_note(&note_path("A.md"), "A")).unwrap();
}

#[test]
fn a_failed_write_keeps_the_revision() {
    let folder = Folder::new(&[("a.md", "# A\n")]);

    let written = no_step(&folder.core, |core| {
        core.add_tag(&note_path("missing.md"), &tag_name("x"))
    });
    assert!(written.is_err());
    let written = no_step(&folder.core, |core| core.create_folder(&folder_path("missing"), "x"));
    assert!(written.is_err());
}

#[test]
fn an_old_snapshot_stays_as_it_was() {
    let folder = Folder::new(&[("A.md", "")]);
    let old = folder.core.snapshot();

    folder.core.create(&folder_path(""), "B").unwrap();

    assert_eq!(titles(&old), ["A"]);
    assert_eq!(titles(&folder.core.snapshot()), ["A", "B"]);
}

#[test]
fn paths_first_matching_keep_and_counts_agree_with_the_note_lists() {
    let mut folder = Folder::new(&[
        ("b.md", "#work\n"),
        ("A.md", "#work/holm\n"),
        ("c.md", "---\npinned: true\n---\n#home\n"),
        ("work/d.md", "#work\n"),
        ("work/E.md", "#workshop\n"),
    ]);
    for (path, secs) in [("b.md", 30), ("A.md", 10), ("c.md", 20), ("work/d.md", 50)] {
        folder.set_time(path, secs);
    }
    folder.reopen();
    let snapshot = folder.core.snapshot();
    let sets = [
        NoteSet::All,
        NoteSet::In(&folder_path("")),
        NoteSet::In(&folder_path("work")),
        NoteSet::In(&folder_path("gone")),
        NoteSet::WithTag(&tag_name("work")),
        NoteSet::WithTag(&tag_name("home")),
        NoteSet::WithTag(&tag_name("gone")),
    ];

    for set in sets {
        for order in [Order::LastChange, Order::Title] {
            let notes = snapshot.notes(set, order);
            let expected: Vec<NotePath> = notes.iter().map(|note| note.path.clone()).collect();
            assert_eq!(snapshot.paths(set, order), expected, "{set:?} {order:?}");
            assert_eq!(snapshot.first(set, order), notes.first(), "{set:?} {order:?}");
            assert_eq!(snapshot.matching(set).count(), notes.len(), "{set:?}");
        }
        let all = snapshot.paths(NoteSet::All, Order::Title);
        assert_eq!(snapshot.keep(&all, set), snapshot.paths(set, Order::Title), "{set:?}");
        if let NoteSet::In(folder) = set {
            assert_eq!(
                snapshot.folder_counts().get(folder).copied().unwrap_or(0),
                snapshot.matching(set).count(),
                "{set:?}"
            );
        }
    }
    assert_eq!(snapshot.folder_counts().len(), 2);
    assert_eq!(
        holm_core_test::paths(snapshot.paths(NoteSet::All, Order::LastChange)),
        ["c.md", "work/E.md", "work/d.md", "b.md", "A.md"]
    );
}

#[test]
fn the_walk_of_the_nested_tags_gives_each_tag_before_its_child_tags() {
    let folder = Folder::new(&[
        ("a.md", "---\ntags: [work/holm, home]\n---\n"),
        ("b.md", "---\ntags: [work/deep/x]\n---\n"),
    ]);
    let tags = folder.core.snapshot().tags();
    let names: Vec<&str> = Tag::walk(&tags).map(|tag| tag.name.as_str()).collect();
    assert_eq!(names, ["home", "work", "work/deep", "work/deep/x", "work/holm"]);
}

#[test]
fn note_or_first_gives_the_note_while_it_exists_or_else_the_first_note() {
    let folder = Folder::new(&[("b.md", ""), ("A.md", "")]);
    let snapshot = folder.core.snapshot();
    let path = |note: Option<&NoteInfo>| note.map(|note| note.path.to_string());

    let b = note_path("b.md");
    assert_eq!(
        path(snapshot.note_or_first(Some(&b), Order::Title)),
        Some("b.md".into())
    );
    let gone = note_path("gone.md");
    assert_eq!(
        path(snapshot.note_or_first(Some(&gone), Order::Title)),
        Some("A.md".into())
    );
    assert_eq!(path(snapshot.note_or_first(None, Order::Title)), Some("A.md".into()));
    let empty = Folder::new(&[]).core.snapshot();
    assert_eq!(empty.note_or_first(Some(&b), Order::Title), None);
}

#[test]
fn newest_and_backlinks_put_the_newest_change_first_with_no_pins_on_top() {
    let mut folder = Folder::new(&[
        ("target.md", "---\nid: T\n---\n"),
        ("old.md", "---\npinned: true\n---\n[t](holm:T)\n"),
        ("b.md", "[t](holm:T)\n"),
        ("a.md", "[t](holm:T)\n"),
        ("a b.md", "[t](holm:T)\n"),
        ("a/z.md", "[t](holm:T)\n"),
    ]);
    for (path, secs) in [
        ("target.md", 5),
        ("old.md", 10),
        ("b.md", 20),
        ("a.md", 20),
        ("a b.md", 20),
        ("a/z.md", 20),
    ] {
        folder.set_time(path, secs);
    }
    folder.reopen();
    let snapshot = folder.core.snapshot();
    let linked = ["a/z.md", "a b.md", "a.md", "b.md", "old.md"];

    assert_eq!(paths(snapshot.backlinks(&note_path("target.md"))), linked);
    assert_eq!(
        paths(snapshot.newest(NoteSet::All)),
        [&linked[..], &["target.md"]].concat()
    );
}
