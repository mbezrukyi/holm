//! Changing tags and pins through `Core` on a real temp folder.

#![allow(clippy::unwrap_used, reason = "tests")]

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt as _;

use common::{Folder, note_path};
use holm_core::Error;
use holm_core::model::{NoteSet, Order};

fn tags(folder: &Folder, path: &str) -> Vec<String> {
    folder.core.read(&note_path(path)).unwrap().info.tags
}

fn tagged(folder: &Folder, tag: &str) -> Vec<String> {
    let notes = folder
        .core
        .snapshot()
        .notes(NoteSet::WithTag(tag), Order::Title);
    common::paths(notes.into_iter().map(|n| n.path))
}

#[test]
fn add_tag_writes_a_clean_tag_and_keeps_the_body_other_fields_and_time() {
    let folder = Folder::new(&[("a.md", "---\nid: A\nauthor: me\n---\n# A\n")]);
    let time = folder.set_time("a.md", 1_000);

    folder.core.add_tag(&note_path("a.md"), "home").unwrap();
    let info = folder
        .core
        .add_tag(&note_path("a.md"), "my work / holm/")
        .unwrap();

    assert_eq!(info.tags, ["home", "my-work/holm"]);
    assert_eq!(info.modified, time);
    assert_eq!(
        folder.text("a.md"),
        "---\nid: A\ntags:\n  - home\n  - my-work/holm\nauthor: me\n---\n# A\n"
    );
    assert_eq!(tagged(&folder, "my-work"), ["a.md"]);
}

#[test]
fn add_tag_of_a_tag_the_note_has_does_not_write() {
    let text = "---\nid: A\ntags: [work, home]\n---\n# A\n";
    let folder = Folder::new(&[("a.md", text)]);

    folder.core.add_tag(&note_path("a.md"), "work").unwrap();

    assert_eq!(folder.text("a.md"), text);
}

#[test]
fn add_tag_with_nothing_left_fails() {
    let folder = Folder::new(&[("a.md", "# A\n")]);

    assert!(matches!(
        folder.core.add_tag(&note_path("a.md"), "//"),
        Err(Error::BadTag { .. })
    ));
    assert_eq!(folder.text("a.md"), "# A\n");
}

#[test]
fn remove_tag_keeps_the_other_tags_and_the_child_tags() {
    let folder = Folder::new(&[(
        "a.md",
        "---\nid: A\ntags: [work, work/holm, home]\n---\n# A\n",
    )]);
    let time = folder.set_time("a.md", 1_000);

    let info = folder.core.remove_tag(&note_path("a.md"), "work").unwrap();

    assert_eq!(info.tags, ["work/holm", "home"]);
    assert_eq!(info.modified, time);
}

#[test]
fn remove_tag_of_the_last_tag_removes_the_field() {
    let folder = Folder::new(&[("a.md", "---\nid: A\ntags: [work]\n---\n# A\n")]);

    folder.core.remove_tag(&note_path("a.md"), "work").unwrap();

    assert_eq!(folder.text("a.md"), "---\nid: A\n---\n# A\n");
    assert_eq!(tagged(&folder, "work"), [] as [String; 0]);
}

#[test]
fn remove_tag_of_a_tag_the_note_does_not_have_does_not_write() {
    let text = "---\nid: A\ntags: [work, home]\n---\n# A\n";
    let folder = Folder::new(&[("a.md", text)]);

    folder.core.remove_tag(&note_path("a.md"), "other").unwrap();

    assert_eq!(folder.text("a.md"), text);
}

#[test]
fn tag_writes_keep_a_tag_from_another_tool() {
    let folder = Folder::new(&[("a.md", "---\nid: A\n---\n# A\n")]);

    folder.core.add_tag(&note_path("a.md"), "a").unwrap();
    folder.write("a.md", "---\nid: A\ntags: [a, b]\n---\n# A\n");
    folder.core.remove_tag(&note_path("a.md"), "a").unwrap();

    assert_eq!(tags(&folder, "a.md"), ["b"]);
}

#[test]
fn set_pinned_pins_and_unpins_and_keeps_the_time() {
    let folder = Folder::new(&[("a.md", "---\nid: A\n---\n# A\n"), ("b.md", "# B\n")]);
    let time = folder.set_time("a.md", 1_000);

    let info = folder.core.set_pinned(&note_path("a.md"), true).unwrap();
    assert!(info.pinned);
    assert_eq!(info.modified, time);
    assert_eq!(folder.text("a.md"), "---\nid: A\npinned: true\n---\n# A\n");
    let first = folder.core.snapshot().notes(NoteSet::All, Order::Title)[0].clone();
    assert!(first.pinned);

    let info = folder.core.set_pinned(&note_path("a.md"), false).unwrap();
    assert!(!info.pinned);
    assert_eq!(folder.text("a.md"), "---\nid: A\n---\n# A\n");
}

#[test]
fn rename_tag_renames_child_tags_and_keeps_times() {
    let folder = Folder::new(&[
        ("a.md", "---\ntags: [work, home]\n---\n# A\n"),
        ("b.md", "---\ntags: [work/holm]\n---\n# B\n"),
        ("c.md", "---\ntags: [workshop]\n---\n# C\n"),
    ]);
    let time = folder.set_time("b.md", 1_000);

    assert_eq!(folder.core.tag_rename_count("work", "job").unwrap(), 2);
    assert_eq!(folder.core.rename_tag("work", "job").unwrap(), 2);

    assert_eq!(tags(&folder, "a.md"), ["job", "home"]);
    assert_eq!(tags(&folder, "b.md"), ["job/holm"]);
    assert_eq!(tags(&folder, "c.md"), ["workshop"]);
    assert_eq!(
        folder.core.read(&note_path("b.md")).unwrap().info.modified,
        time
    );
    assert_eq!(tagged(&folder, "work"), [] as [String; 0]);
    assert_eq!(tagged(&folder, "job"), ["a.md", "b.md"]);
}

#[test]
fn rename_tag_changes_every_note_in_many_folders_and_leaves_no_temp_file() {
    let files: Vec<(String, String)> = (0..30)
        .map(|n| {
            let text = format!("---\ntags: [work, n{n}]\n---\n# Note {n}\n");
            (format!("f{}/n{n}.md", n % 3), text)
        })
        .collect();
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let folder = Folder::new(&files);

    assert_eq!(folder.core.rename_tag("work", "job").unwrap(), 30);

    for n in 0..30 {
        let path = format!("f{}/n{n}.md", n % 3);
        assert_eq!(tags(&folder, &path), ["job".to_owned(), format!("n{n}")]);
    }
    assert_eq!(tagged(&folder, "job").len(), 30);
    assert_eq!(folder.names("f0").len(), 10);
}

#[test]
fn rename_tag_to_an_existing_tag_merges_them() {
    let folder = Folder::new(&[
        ("a.md", "---\ntags: [work, job]\n---\n# A\n"),
        ("b.md", "---\ntags: [job/holm, work/holm]\n---\n# B\n"),
    ]);

    assert_eq!(folder.core.rename_tag("work", "job").unwrap(), 2);

    assert_eq!(tags(&folder, "a.md"), ["job"]);
    assert_eq!(tags(&folder, "b.md"), ["job/holm"]);
}

#[test]
fn rename_tag_cleans_the_new_name() {
    let folder = Folder::new(&[("a.md", "---\ntags: [work]\n---\n# A\n")]);

    folder.core.rename_tag("work", " my job / ").unwrap();

    assert_eq!(tags(&folder, "a.md"), ["my-job"]);
}

#[test]
fn rename_tag_to_the_same_name_changes_nothing() {
    let text = "---\ntags: [work, work/holm]\n---\n# A\n";
    let folder = Folder::new(&[("a.md", text)]);

    assert_eq!(folder.core.tag_rename_count("work", "work").unwrap(), 0);
    assert_eq!(folder.core.rename_tag("work", "work").unwrap(), 0);
    assert_eq!(folder.text("a.md"), text);
}

#[test]
fn delete_tag_removes_the_tag_and_its_child_tags() {
    let folder = Folder::new(&[
        ("a.md", "---\nid: A\ntags: [work, home]\n---\n# A\n"),
        ("b.md", "---\nid: B\ntags:\n  - work/holm\n---\n# B\n"),
        ("c.md", "---\nid: C\ntags: [workshop]\n---\n# C\n"),
    ]);

    assert_eq!(folder.core.tag_delete_count("work").unwrap(), 2);
    assert_eq!(folder.core.delete_tag("work").unwrap(), 2);

    assert_eq!(tags(&folder, "a.md"), ["home"]);
    assert_eq!(folder.text("b.md"), "---\nid: B\n---\n# B\n");
    assert_eq!(tags(&folder, "c.md"), ["workshop"]);
    let names: Vec<String> = folder
        .core
        .snapshot()
        .tags()
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(names, ["home", "workshop"]);
}

#[test]
fn a_tag_that_no_note_has_changes_nothing() {
    let folder = Folder::new(&[("a.md", "---\ntags: [work]\n---\n# A\n")]);

    assert_eq!(folder.core.tag_delete_count("home").unwrap(), 0);
    assert_eq!(folder.core.delete_tag("home").unwrap(), 0);
    assert_eq!(tags(&folder, "a.md"), ["work"]);
}

#[test]
fn bad_tag_names_are_refused() {
    let folder = Folder::new(&[("a.md", "---\ntags: [work]\n---\n# A\n")]);

    for result in [
        folder.core.rename_tag("work", " / "),
        folder.core.rename_tag("", "job"),
        folder.core.rename_tag("work/", "job"),
        folder.core.delete_tag(""),
        folder.core.tag_delete_count("a//b"),
    ] {
        assert!(matches!(result, Err(Error::BadTag { .. })), "{result:?}");
    }
    assert_eq!(tags(&folder, "a.md"), ["work"]);
}

#[test]
fn a_note_that_cant_change_keeps_its_tags_and_the_others_still_change() {
    let folder = Folder::new(&[
        ("a.md", "---\ntags: [work]\n---\n# A\n"),
        ("locked/b.md", "---\ntags: [work]\n---\n# B\n"),
    ]);
    let locked = folder.path("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();

    let result = folder.core.rename_tag("work", "job");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(matches!(result, Err(Error::Write { .. })), "{result:?}");
    assert_eq!(tags(&folder, "a.md"), ["job"]);
    assert_eq!(tags(&folder, "locked/b.md"), ["work"]);
}
