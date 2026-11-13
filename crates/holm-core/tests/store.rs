//! Reading, listing and saving notes through `Core` on a real temp folder.

#![allow(clippy::unwrap_used, reason = "tests")]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::Folder;
use holm_core::model::Order;
use holm_core::{Core, Error};

fn paths(items: impl IntoIterator<Item = PathBuf>) -> Vec<String> {
    let mut paths = common::paths(items);
    paths.sort();
    paths
}

#[test]
fn open_fails_on_a_missing_folder() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing");

    let error = Core::open(&missing, dir.path()).unwrap_err();

    assert!(matches!(error, Error::NotesFolder { path, .. } if path == missing));
}

#[test]
fn open_fails_on_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("file");
    fs::write(&file, "").unwrap();

    assert!(matches!(
        Core::open(&file, dir.path()),
        Err(Error::NotesFolder { .. })
    ));
}

#[test]
fn notes_lists_only_md_files_and_skips_hidden_ones() {
    let folder = Folder::new(&[
        ("a.md", ""),
        ("work/holm/b.md", ""),
        ("attachments/c.md", ""),
        ("image.png", ""),
        ("README.MD", ""),
        (".hidden.md", ""),
        (".git/d.md", ""),
        ("work/.trash/e.md", ""),
    ]);

    let notes = folder.core.snapshot().notes(Order::LastChange);

    assert_eq!(
        paths(notes.into_iter().map(|n| n.path)),
        ["a.md", "attachments/c.md", "work/holm/b.md"]
    );
}

#[test]
fn notes_skips_a_note_that_is_not_text() {
    let folder = Folder::new(&[("a.md", "")]);
    fs::write(folder.dir.path().join("b.md"), [0xff, 0xfe]).unwrap();

    let notes = folder.core.snapshot().notes(Order::LastChange);

    assert_eq!(paths(notes.into_iter().map(|n| n.path)), ["a.md"]);
}

#[test]
fn folders_lists_nested_folders_without_attachments() {
    let folder = Folder::new(&[
        ("work/holm/b.md", ""),
        ("home/c.md", ""),
        ("attachments/image.png", ""),
        ("work/attachments/d.md", ""),
        (".git/e", ""),
    ]);

    let folders = folder
        .core
        .snapshot()
        .folders()
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();

    assert_eq!(
        paths(folders),
        ["home", "work", "work/attachments", "work/holm"]
    );
}

#[test]
fn read_gives_the_fields_and_the_body() {
    let folder = Folder::new(&[(
        "work/Plans.md",
        "---\nid: 01JC3X\ntags: [work, work/holm]\npinned: true\nauthor: me\n---\n# Big plans\n\nText.\n",
    )]);

    let note = folder.core.read("work/Plans.md").unwrap();

    assert_eq!(note.info.path, Path::new("work/Plans.md"));
    assert_eq!(note.info.id.unwrap().as_str(), "01JC3X");
    assert_eq!(note.info.title, "Big plans");
    assert_eq!(note.info.tags, ["work", "work/holm"]);
    assert!(note.info.pinned);
    assert_eq!(note.body, "# Big plans\n\nText.\n");
}

#[test]
fn read_takes_the_title_from_the_file_name_with_no_heading() {
    let folder = Folder::new(&[("Shopping list.md", "milk\n")]);

    let note = folder.core.read("Shopping list.md").unwrap();

    assert_eq!(note.info.title, "Shopping list");
    assert_eq!(note.info.id, None);
    assert_eq!(note.info.tags, Vec::<String>::new());
    assert!(!note.info.pinned);
}

#[test]
fn read_refuses_paths_outside_the_notes() {
    let folder = Folder::new(&[("a.md", ""), (".hidden.md", ""), ("a.txt", "")]);

    for path in ["../a.md", "/a.md", ".hidden.md", "a.txt", "./a.md", ""] {
        let result = folder.core.read(path);
        assert!(
            matches!(result, Err(Error::BadPath { .. })),
            "{path}: {result:?}"
        );
    }
}

#[test]
fn read_fails_on_a_missing_note_and_on_bytes_that_are_not_text() {
    let folder = Folder::new(&[]);
    fs::write(folder.dir.path().join("bad.md"), [b'a', 0xff]).unwrap();

    assert!(matches!(
        folder.core.read("missing.md"),
        Err(Error::Read { .. })
    ));
    assert!(matches!(
        folder.core.read("bad.md"),
        Err(Error::NotUtf8 { .. })
    ));
}

#[test]
fn save_adds_an_id_on_the_first_write_and_keeps_it() {
    let folder = Folder::new(&[("a.md", "# A\n")]);

    let saved = folder.core.save("a.md", "# A\n\nMore.\n").result.unwrap();

    let id = saved.info.id.clone().unwrap();
    assert_eq!(id.as_str().len(), 26, "a ULID");
    assert_eq!(
        folder.text("a.md"),
        format!("---\nid: {id}\n---\n# A\n\nMore.\n")
    );
    assert_eq!(
        folder.core.save("a.md", "# A\n").result.unwrap().info.id,
        Some(id)
    );
}

#[test]
fn save_writes_the_body_and_keeps_the_frontmatter_of_the_file() {
    let folder = Folder::new(&[(
        "a.md",
        "---\nauthor: me\nid: X1\ntags: [work]\n# a comment\n---\nBody\n",
    )]);
    folder.core.read("a.md").unwrap();
    folder.write(
        "a.md",
        "---\nauthor: me\nid: X1\ntags: [work, home]\npinned: true\n# a comment\n---\nBody\n",
    );

    let saved = folder.core.save("a.md", "New body\n").result.unwrap();

    assert_eq!(
        folder.text("a.md"),
        "---\nauthor: me\nid: X1\ntags: [work, home]\npinned: true\n# a comment\n---\nNew body\n"
    );
    assert_eq!(saved.info.tags, ["work", "home"]);
    assert!(saved.info.pinned);
}

#[test]
fn save_keeps_the_old_time_when_it_only_adds_the_id() {
    let folder = Folder::new(&[("a.md", "Body\n")]);
    let time = folder.set_time("a.md", 1_000_000);

    let saved = folder.core.save("a.md", "Body\n").result.unwrap();

    assert!(saved.info.id.is_some());
    assert_eq!(saved.info.modified, time);
}

#[test]
fn save_gives_a_new_time_when_the_body_changes() {
    let folder = Folder::new(&[("a.md", "---\nid: X1\n---\nBody\n")]);
    let time = folder.set_time("a.md", 1_000_000);

    let saved = folder.core.save("a.md", "New body\n").result.unwrap();

    assert!(saved.info.modified > time);
}

#[test]
fn save_with_no_change_does_not_write() {
    let text = "---\nid: X1\ntags: [a, b]\n---\nBody\n";
    let folder = Folder::new(&[("a.md", text)]);
    let time = folder.set_time("a.md", 1_000_000);

    folder.core.save("a.md", "Body\n").result.unwrap();

    assert_eq!(folder.text("a.md"), text, "the flow list stays");
    assert_eq!(folder.core.read("a.md").unwrap().info.modified, time);
}

#[test]
fn save_adds_a_final_line_break_with_the_line_ending_of_the_file() {
    let folder = Folder::new(&[("a.md", "---\r\nid: X1\r\n---\r\nBody\r\n")]);

    folder.core.save("a.md", "Body\r\nMore").result.unwrap();

    assert_eq!(
        folder.text("a.md"),
        "---\r\nid: X1\r\n---\r\nBody\r\nMore\r\n"
    );
}

#[test]
fn save_keeps_the_permissions_and_leaves_no_temp_file() {
    use std::os::unix::fs::PermissionsExt as _;

    let folder = Folder::new(&[("a.md", "Body\n")]);
    let path = folder.dir.path().join("a.md");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

    folder.core.save("a.md", "New\n").result.unwrap();

    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o640);
    assert_eq!(folder.files(), ["a.md"]);
}

#[test]
fn save_fails_on_a_missing_note() {
    let folder = Folder::new(&[("a.md", "Body\n")]);
    fs::remove_file(folder.dir.path().join("a.md")).unwrap();

    assert!(matches!(
        folder.core.save("a.md", "New\n").result,
        Err(Error::Read { .. })
    ));
    assert_eq!(folder.files(), Vec::<String>::new());
}

#[test]
fn restore_writes_a_deleted_note_again_with_its_id_tags_and_pin() {
    let folder = Folder::new(&[(
        "a.md",
        "---\nid: X1\ntags: [work]\npinned: true\n---\n# A\n",
    )]);
    let mut note = folder.core.read("a.md").unwrap();
    fs::remove_file(folder.path("a.md")).unwrap();
    note.body = "# A\n\nMine.\n".to_owned();

    let restored = folder.core.restore(&note).result.unwrap();

    assert_eq!(restored.info.id, note.info.id);
    assert_eq!(restored.info.tags, ["work"]);
    assert!(restored.info.pinned);
    assert_eq!(restored.body, "# A\n\nMine.\n");
    assert_eq!(folder.files(), ["a.md"]);
}

#[test]
fn restore_saves_the_body_over_a_file_that_is_back() {
    let folder = Folder::new(&[("a.md", "---\nid: X1\ntags: [mine]\n---\n# A\n")]);
    let mut note = folder.core.read("a.md").unwrap();
    folder.write("a.md", "---\nid: X1\ntags: [theirs]\n---\n# Theirs\n");
    note.body = "# Mine\n".to_owned();

    folder.core.restore(&note).result.unwrap();

    assert_eq!(
        folder.text("a.md"),
        "---\nid: X1\ntags: [theirs]\n---\n# Mine\n"
    );
}

#[test]
fn create_makes_untitled_notes_with_a_number_on_a_clash() {
    let folder = Folder::new(&[("Untitled 2.md", "taken\n"), ("work/x.md", "")]);

    let first = folder.core.create("").result.unwrap();
    let second = folder.core.create("").result.unwrap();
    let nested = folder.core.create("work").result.unwrap();

    assert_eq!(first.info.path, Path::new("Untitled.md"));
    assert_eq!(second.info.path, Path::new("Untitled 3.md"));
    assert_eq!(nested.info.path, Path::new("work/Untitled.md"));
    assert_eq!(folder.text("Untitled 2.md"), "taken\n");

    let id = first.info.id.unwrap();
    assert_ne!(Some(&id), second.info.id.as_ref());
    assert_eq!(folder.text("Untitled.md"), format!("---\nid: {id}\n---\n"));
    assert_eq!(first.info.title, "Untitled");
    assert_eq!(first.body, "");
}

#[test]
fn create_fails_in_a_missing_or_hidden_folder() {
    let folder = Folder::new(&[]);

    assert!(matches!(
        folder.core.create("missing").result,
        Err(Error::Write { .. })
    ));
    assert!(matches!(
        folder.core.create(".git").result,
        Err(Error::BadPath { .. })
    ));
    assert!(matches!(
        folder.core.create("../x").result,
        Err(Error::BadPath { .. })
    ));
}

#[test]
fn the_handle_works_from_many_threads() {
    let folder = Folder::new(&[]);

    let handles: Vec<_> = (0..8)
        .map(|_| {
            let core = folder.core.clone();
            std::thread::spawn(move || core.create("").result.unwrap().info.path)
        })
        .collect();
    let mut created: Vec<PathBuf> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    created.sort();
    created.dedup();

    assert_eq!(created.len(), 8);
    assert_eq!(folder.core.snapshot().notes(Order::LastChange).len(), 8);
}
