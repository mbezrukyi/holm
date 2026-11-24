use std::ffi::OsStr;
use std::path::Path;

use holm_core::testing::{folder_path as folder, note_path as note};
use holm_core::{FolderPath, NotePath};
use proptest::prelude::*;

fn plain() -> impl Strategy<Value = String> {
    let part = "[ !#+,.a-bA-Bé-]{1,4}".prop_filter("not hidden", |part| !part.starts_with('.'));
    prop::collection::vec(part, 1..4).prop_map(|parts| parts.join("/"))
}

proptest! {
    #[test]
    fn the_order_is_the_order_of_path_cmp(a in plain(), b in plain()) {
        prop_assert_eq!(folder(&a).cmp(&folder(&b)), Path::new(&a).cmp(Path::new(&b)));
        let (a, b) = (format!("{a}.md"), format!("{b}.md"));
        prop_assert_eq!(note(&a).cmp(&note(&b)), Path::new(&a).cmp(Path::new(&b)));
    }

    #[test]
    fn the_order_is_the_order_of_path_cmp_with_a_shared_start(
        start in plain(),
        a in plain(),
        b in plain(),
    ) {
        let (a, b) = (format!("{start}{a}"), format!("{start}/{b}"));
        prop_assert_eq!(folder(&a).cmp(&folder(&b)), Path::new(&a).cmp(Path::new(&b)));
        let (a, b) = (format!("{a}.md"), format!("{b}.md"));
        prop_assert_eq!(note(&a).cmp(&note(&b)), Path::new(&a).cmp(Path::new(&b)));
        prop_assert_eq!(note(&b).cmp(&note(&a)), Path::new(&b).cmp(Path::new(&a)));
    }
}

#[test]
fn a_deeper_note_can_come_first() {
    assert!(note("a/b/c.md") < note("a/b.md"));
    assert!(Path::new("a/b/c.md") < Path::new("a/b.md"));
}

#[test]
fn a_note_reads_as_its_whole_path() {
    assert_eq!(note("work/Plans.md").as_ref(), Path::new("work/Plans.md"));
    assert_eq!(note("A.md").as_ref(), Path::new("A.md"));
}

#[test]
fn in_folder_keeps_the_name() {
    let plans = note("work/Plans.md");
    assert_eq!(plans.in_folder(&folder("home")), note("home/Plans.md"));
    assert_eq!(plans.in_folder(&FolderPath::root()), note("Plans.md"));
}

#[test]
fn a_bad_path_gives_no_value() {
    let bad = [
        "/a.md",
        "./a.md",
        "a/./b.md",
        "../a.md",
        "a/../b.md",
        "a//b.md",
        "a.md/",
        ".a.md",
        "a/.b/c.md",
        ".md",
        "a/.md",
        "a\0b.md",
    ];
    for path in bad {
        assert_eq!(NotePath::new(path), None, "{path:?}");
        assert_eq!(FolderPath::new(path), None, "{path:?}");
    }
    assert_eq!(NotePath::new(""), None);
    assert_eq!(NotePath::new("a/b.txt"), None);
    assert_eq!(NotePath::new("a.md.txt"), None);
}

#[test]
fn a_part_has_at_most_255_bytes() {
    let part = |len: usize| "a".repeat(len);
    assert!(FolderPath::new(format!("x/{}", part(255))).is_some());
    assert!(FolderPath::new(format!("x/{}/y", part(256))).is_none());
    assert!(NotePath::new(format!("{}.md", part(252))).is_some());
    assert!(NotePath::new(format!("{}.md", part(253))).is_none());
}

#[test]
fn a_plain_path_gives_a_value() {
    assert!(NotePath::new("a b/c.d.md").is_some());
    assert!(FolderPath::new("").is_some_and(|root| root.is_root()));
    assert!(FolderPath::new("a b/c.md").is_some());
    assert!(FolderPath::new("attachments").is_some());
}

#[test]
fn a_note_knows_its_folder_title_and_name() {
    let plans = note("work/2026/Plans.md");
    assert_eq!(*plans.folder(), folder("work/2026"));
    assert_eq!(plans.title(), "Plans");
    assert_eq!(plans.name(), "Plans.md");
    assert_eq!(*note("A.md").folder(), FolderPath::root());
    let parents: Vec<_> = plans.parents().collect();
    assert_eq!(parents, [folder("work/2026"), folder("work"), FolderPath::root()]);
}

#[test]
fn a_folder_knows_its_folder_name_depth_and_parents() {
    let year = folder("work/2026");
    assert_eq!(year.folder(), Some(folder("work")));
    assert_eq!(folder("work").folder(), Some(FolderPath::root()));
    assert_eq!(FolderPath::root().folder(), None);
    assert_eq!(year.name(), Some(OsStr::new("2026")));
    assert_eq!(FolderPath::root().name(), None);
    assert_eq!(year.last_part(), "2026");
    assert_eq!(FolderPath::root().last_part(), "");
    assert_eq!(year.depth(), 2);
    assert_eq!(FolderPath::root().depth(), 0);
    let parents: Vec<_> = year.parents().collect();
    assert_eq!(parents, [folder("work"), FolderPath::root()]);
    assert_eq!(FolderPath::root().parents().count(), 0);
}

#[test]
fn in_and_below_compare_whole_parts() {
    let work = folder("work");
    assert!(note("work/a.md").is_in(&work));
    assert!(!note("work/x/a.md").is_in(&work));
    assert!(note("work/x/a.md").is_below(&work));
    assert!(!note("workshop/a.md").is_below(&work));
    assert!(note("a.md").is_in(&FolderPath::root()));
    assert!(folder("work/x").is_in(&work));
    assert!(!work.is_below(&work));
    assert!(work.is_below(&FolderPath::root()));
    assert!(!FolderPath::root().is_below(&FolderPath::root()));
}

#[test]
fn a_folder_moves_into_no_folder_of_its_own() {
    let work = folder("work");
    assert!(work.can_move_into(&FolderPath::root()));
    assert!(work.can_move_into(&folder("workshop")));
    assert!(!work.can_move_into(&work));
    assert!(!work.can_move_into(&folder("work/x")));
    assert!(!FolderPath::root().can_move_into(&work));
}

#[test]
fn moved_follows_a_folder_move() {
    let (from, to) = (folder("work"), folder("old/job"));
    assert_eq!(note("work/x/a.md").moved(&from, &to), Some(note("old/job/x/a.md")));
    assert_eq!(note("workshop/a.md").moved(&from, &to), None);
    assert_eq!(from.moved(&from, &to), Some(to.clone()));
    assert_eq!(folder("work/x").moved(&from, &to), Some(folder("old/job/x")));
    assert_eq!(
        note("a/b.md").moved(&folder("a"), &FolderPath::root()),
        Some(note("b.md"))
    );
}
