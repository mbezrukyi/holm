//! Changing tags and pins through `Core` on a real temp folder.

#![allow(clippy::unwrap_used, reason = "tests")]

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt as _;

use common::Folder;
use holm_core::Error;
use holm_core::model::Order;

fn tags(folder: &Folder, path: &str) -> Vec<String> {
    folder.core.read(path).unwrap().info.tags
}

fn tagged(folder: &Folder, tag: &str) -> Vec<String> {
    let notes = folder.core.snapshot().notes_with_tag(tag, Order::Title);
    common::paths(notes.into_iter().map(|n| n.path))
}

#[test]
fn set_tags_writes_clean_tags_and_keeps_the_body_other_fields_and_time() {
    let folder = Folder::new(&[("a.md", "---\nid: A\nauthor: me\n---\n# A\n")]);
    let time = folder.set_time("a.md", 1_000);

    let info = folder
        .core
        .set_tags("a.md", &["my work / holm/", "home", "home", "//"])
        .result
        .unwrap();

    assert_eq!(info.tags, ["my-work/holm", "home"]);
    assert_eq!(info.modified, time);
    assert_eq!(
        folder.text("a.md"),
        "---\nid: A\ntags:\n  - my-work/holm\n  - home\nauthor: me\n---\n# A\n"
    );
    assert_eq!(tagged(&folder, "my-work"), ["a.md"]);
}

#[test]
fn set_tags_with_no_tags_removes_the_field() {
    let folder = Folder::new(&[("a.md", "---\nid: A\ntags: [work]\n---\n# A\n")]);

    folder.core.set_tags::<&str>("a.md", &[]).result.unwrap();

    assert_eq!(folder.text("a.md"), "---\nid: A\n---\n# A\n");
    assert_eq!(tagged(&folder, "work"), [] as [String; 0]);
}

#[test]
fn set_tags_with_the_same_tags_does_not_write() {
    let text = "---\nid: A\ntags: [work, home]\n---\n# A\n";
    let folder = Folder::new(&[("a.md", text)]);

    folder
        .core
        .set_tags("a.md", &["work", "home"])
        .result
        .unwrap();

    assert_eq!(folder.text("a.md"), text);
}

#[test]
fn set_pinned_pins_and_unpins_and_keeps_the_time() {
    let folder = Folder::new(&[("a.md", "---\nid: A\n---\n# A\n"), ("b.md", "# B\n")]);
    let time = folder.set_time("a.md", 1_000);

    let info = folder.core.set_pinned("a.md", true).result.unwrap();
    assert!(info.pinned);
    assert_eq!(info.modified, time);
    assert_eq!(folder.text("a.md"), "---\nid: A\npinned: true\n---\n# A\n");
    let first = folder.core.snapshot().notes(Order::Title)[0].clone();
    assert!(first.pinned);

    let info = folder.core.set_pinned("a.md", false).result.unwrap();
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
    assert_eq!(folder.core.rename_tag("work", "job").result.unwrap(), 2);

    assert_eq!(tags(&folder, "a.md"), ["job", "home"]);
    assert_eq!(tags(&folder, "b.md"), ["job/holm"]);
    assert_eq!(tags(&folder, "c.md"), ["workshop"]);
    assert_eq!(folder.core.read("b.md").unwrap().info.modified, time);
    assert_eq!(tagged(&folder, "work"), [] as [String; 0]);
    assert_eq!(tagged(&folder, "job"), ["a.md", "b.md"]);
}

#[test]
fn rename_tag_to_an_existing_tag_merges_them() {
    let folder = Folder::new(&[
        ("a.md", "---\ntags: [work, job]\n---\n# A\n"),
        ("b.md", "---\ntags: [job/holm, work/holm]\n---\n# B\n"),
    ]);

    assert_eq!(folder.core.rename_tag("work", "job").result.unwrap(), 2);

    assert_eq!(tags(&folder, "a.md"), ["job"]);
    assert_eq!(tags(&folder, "b.md"), ["job/holm"]);
}

#[test]
fn rename_tag_cleans_the_new_name() {
    let folder = Folder::new(&[("a.md", "---\ntags: [work]\n---\n# A\n")]);

    folder.core.rename_tag("work", " my job / ").result.unwrap();

    assert_eq!(tags(&folder, "a.md"), ["my-job"]);
}

#[test]
fn rename_tag_to_the_same_name_changes_nothing() {
    let text = "---\ntags: [work, work/holm]\n---\n# A\n";
    let folder = Folder::new(&[("a.md", text)]);

    assert_eq!(folder.core.tag_rename_count("work", "work").unwrap(), 0);
    assert_eq!(folder.core.rename_tag("work", "work").result.unwrap(), 0);
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
    assert_eq!(folder.core.delete_tag("work").result.unwrap(), 2);

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
    assert_eq!(folder.core.delete_tag("home").result.unwrap(), 0);
    assert_eq!(tags(&folder, "a.md"), ["work"]);
}

#[test]
fn bad_tag_names_are_refused() {
    let folder = Folder::new(&[("a.md", "---\ntags: [work]\n---\n# A\n")]);

    for result in [
        folder.core.rename_tag("work", " / ").result,
        folder.core.rename_tag("", "job").result,
        folder.core.rename_tag("work/", "job").result,
        folder.core.delete_tag("").result,
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

    let result = folder.core.rename_tag("work", "job").result;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(matches!(result, Err(Error::Write { .. })), "{result:?}");
    assert_eq!(tags(&folder, "a.md"), ["job"]);
    assert_eq!(tags(&folder, "locked/b.md"), ["work"]);
}
