use std::fs;

use holm_core::Core;
use holm_core::testing::{folder_path, tag_name};
use tempfile::TempDir;

use super::*;

/// A notes folder with four notes, and its tree in title order.
fn lists() -> Lists {
    let dir = TempDir::new().unwrap();
    let notes = dir.path().join("notes");
    for folder in ["home", "work/deep"] {
        fs::create_dir_all(notes.join(folder)).unwrap();
    }
    let tags = |tags: &str| format!("---\ntags: [{tags}]\n---\n");
    fs::write(notes.join("a.md"), tags("work")).unwrap();
    fs::write(notes.join("work/b.md"), tags("work/holm")).unwrap();
    fs::write(notes.join("work/c.md"), "").unwrap();
    fs::write(notes.join("work/deep/d.md"), tags("workshop")).unwrap();
    let (core, _) = Core::open_with_feed(&notes, dir.path().join("index")).unwrap();
    Lists::new(&core.snapshot(), Order::Title)
}

fn names(rows: &[TreeRow]) -> Vec<(String, usize, usize)> {
    rows.iter()
        .map(|row| (row.name.clone(), row.depth, row.count))
        .collect()
}

fn owned(rows: &[(&str, usize, usize)]) -> Vec<(String, usize, usize)> {
    rows.iter()
        .map(|(name, depth, count)| ((*name).to_owned(), *depth, *count))
        .collect()
}

#[test]
fn closed_items_hide_their_children() {
    let rows = lists().rows(&HashSet::new(), None);
    assert_eq!(
        names(&rows),
        owned(&[
            ("All notes", 0, 4),
            ("home", 0, 0),
            ("work", 0, 2),
            ("work", 0, 2),
            ("workshop", 0, 1)
        ])
    );
    assert!(rows[2].has_children && !rows[2].open);
    assert!(!rows[1].has_children);
}

#[test]
fn open_items_show_their_children() {
    let open = HashSet::from([Item::Folder(folder_path("work")), Item::Tag(tag_name("work"))]);
    let rows = lists().rows(&open, None);
    assert_eq!(
        names(&rows),
        owned(&[
            ("All notes", 0, 4),
            ("home", 0, 0),
            ("work", 0, 2),
            ("deep", 1, 1),
            ("work", 0, 2),
            ("holm", 1, 1),
            ("workshop", 0, 1)
        ])
    );
    assert!(rows[2].open);
}

#[test]
fn an_open_folder_without_child_rows_is_closed() {
    let open = HashSet::from([Item::Folder(folder_path("home"))]);
    let rows = lists().rows(&open, None);
    assert!(!rows[1].has_children && !rows[1].open);

    let rows = lists().rows(&open, Some(&folder_path("home")));
    assert!(rows[1].has_children && rows[1].open);
}

#[test]
fn labels_come_before_the_first_folder_and_the_first_tag() {
    let tree = lists().tree(&HashSet::new(), None);
    assert_eq!(
        tree.entries,
        [
            Entry::Row(RowIndex(0)),
            Entry::Label(Label::Folders),
            Entry::Row(RowIndex(1)),
            Entry::Row(RowIndex(2)),
            Entry::Label(Label::Tags),
            Entry::Row(RowIndex(3)),
            Entry::Row(RowIndex(4)),
        ]
    );
    assert_eq!(tree.entry_of(&Item::Tag(tag_name("work"))), Some(EntryIndex(5)));
    assert_eq!(tree.row_of(&Item::Tag(tag_name("work"))), Some(RowIndex(3)));
}

#[test]
fn a_new_folder_is_the_first_child_line_of_its_parent() {
    let root = lists().tree(&HashSet::new(), Some(&FolderPath::root()));
    assert_eq!(
        &root.entries[1..3],
        [Entry::Label(Label::Folders), Entry::NewFolder { depth: 0 }]
    );
    assert_eq!(root.entries[3], Entry::Row(RowIndex(1)));

    let open = HashSet::from([Item::Folder(folder_path("home"))]);
    let home = lists().tree(&open, Some(&folder_path("home")));
    assert_eq!(
        &home.entries[2..4],
        [Entry::Row(RowIndex(1)), Entry::NewFolder { depth: 1 }]
    );
    assert!(home.rows[1].open);
}

#[test]
fn a_new_folder_with_no_folders_gets_the_folder_label() {
    let tree = Lists::default().tree(&HashSet::new(), Some(&FolderPath::root()));
    assert_eq!(
        tree.entries,
        [
            Entry::Row(RowIndex(0)),
            Entry::Label(Label::Folders),
            Entry::NewFolder { depth: 0 }
        ]
    );
}

#[test]
fn lists_hold_their_notes() {
    let lists = lists();
    let paths = |item: Item| -> Vec<String> { lists.list(&item).notes().map(|note| note.path.to_string()).collect() };
    assert_eq!(paths(Item::All).len(), 4);
    assert_eq!(paths(Item::Folder(folder_path("work"))), ["work/b.md", "work/c.md"]);
    assert_eq!(paths(Item::Tag(tag_name("work"))), ["a.md", "work/b.md"]);
    assert_eq!(paths(Item::Tag(tag_name("work/holm"))), ["work/b.md"]);
}

#[test]
fn items_know_their_parent_and_folder() {
    assert_eq!(
        Item::Folder(folder_path("work/deep")).parent(),
        Some(Item::Folder(folder_path("work")))
    );
    assert_eq!(Item::Folder(folder_path("work")).parent(), None);
    assert_eq!(
        Item::Tag(tag_name("work/holm")).parent(),
        Some(Item::Tag(tag_name("work")))
    );
    assert_eq!(Item::Tag(tag_name("work/holm")).folder(), FolderPath::root());
    assert!(lists().has(&Item::Tag(tag_name("work/holm"))));
    assert!(!lists().has(&Item::Folder(folder_path("gone"))));
}

#[test]
fn a_rename_moves_the_item_and_its_children() {
    let folder = |path: &str| Item::Folder(folder_path(path));
    let tag = |name: &str| Item::Tag(tag_name(name));
    let work = Change::FolderMoved {
        from: folder_path("work"),
        to: folder_path("job"),
    };
    assert_eq!(folder("work").moved(&work), Some(folder("job")));
    assert_eq!(folder("work/deep").moved(&work), Some(folder("job/deep")));
    assert_eq!(folder("workshop").moved(&work), None);
    assert_eq!(tag("work").moved(&work), None);
    assert_eq!(Item::All.moved(&work), None);
    let work = Change::TagMoved {
        from: tag_name("work"),
        to: tag_name("home/job"),
    };
    assert_eq!(tag("work").moved(&work), Some(tag("home/job")));
    assert_eq!(tag("work/holm").moved(&work), Some(tag("home/job/holm")));
    assert_eq!(tag("workshop").moved(&work), None);
    assert_eq!(folder("work").moved(&work), None);
    assert_eq!(tag("home/job/holm").name(), "holm");
}

#[test]
fn a_gone_item_falls_back_to_its_nearest_parent() {
    let lists = lists();
    let nearest = |item: Item| lists.nearest(&item);
    assert_eq!(
        nearest(Item::Folder(folder_path("work/deep"))),
        Item::Folder(folder_path("work/deep"))
    );
    assert_eq!(
        nearest(Item::Folder(folder_path("work/gone/deeper"))),
        Item::Folder(folder_path("work"))
    );
    assert_eq!(nearest(Item::Folder(folder_path("gone"))), Item::All);
    assert_eq!(nearest(Item::Tag(tag_name("work/gone"))), Item::Tag(tag_name("work")));
    assert_eq!(nearest(Item::Tag(tag_name("gone/holm"))), Item::All);
}
