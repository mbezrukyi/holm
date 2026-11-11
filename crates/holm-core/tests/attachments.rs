//! Copying files into `attachments/` through `Core` on a real temp folder.

#![allow(clippy::unwrap_used, reason = "tests")]

mod common;

use std::fs;
use std::path::Path;

use common::Folder;
use holm_core::Error;

#[test]
fn attach_file_copies_the_file_and_returns_the_path_from_the_note() {
    let folder = Folder::new(&[("a.md", "# A\n"), ("work/b.md", "# B\n")]);
    let outside = tempfile::tempdir().unwrap();
    let file = outside.path().join("photo.jpg");
    fs::write(&file, b"jpg").unwrap();

    let from_root = folder.core.attach_file("a.md", &file).unwrap();
    let from_work = folder.core.attach_file("work/b.md", &file).unwrap();

    assert_eq!(from_root, Path::new("attachments/photo.jpg"));
    assert_eq!(from_work, Path::new("../attachments/photo 2.jpg"));
    assert_eq!(
        fs::read(folder.path("attachments/photo.jpg")).unwrap(),
        b"jpg"
    );
    assert_eq!(
        fs::read(folder.path("attachments/photo 2.jpg")).unwrap(),
        b"jpg"
    );
    assert_eq!(fs::read(&file).unwrap(), b"jpg");
    assert_eq!(folder.text("a.md"), "# A\n");
}

#[test]
fn attach_file_names_a_hidden_file_and_a_file_with_no_extension() {
    let folder = Folder::new(&[("a.md", "# A\n")]);
    let outside = tempfile::tempdir().unwrap();
    let hidden = outside.path().join(".config");
    let plain = outside.path().join("README");
    fs::write(&hidden, "").unwrap();
    fs::write(&plain, "").unwrap();

    assert_eq!(
        folder.core.attach_file("a.md", &hidden).unwrap(),
        Path::new("attachments/-config")
    );
    assert_eq!(
        folder.core.attach_file("a.md", &plain).unwrap(),
        Path::new("attachments/README")
    );
    assert_eq!(
        folder.core.attach_file("a.md", &plain).unwrap(),
        Path::new("attachments/README 2")
    );
}

#[test]
fn attach_file_refuses_a_folder_a_missing_file_and_a_bad_note_path() {
    let folder = Folder::new(&[("a.md", "# A\n")]);
    let outside = tempfile::tempdir().unwrap();
    let file = outside.path().join("x.png");
    fs::write(&file, "").unwrap();

    let results = [
        folder.core.attach_file("a.md", outside.path()),
        folder
            .core
            .attach_file("a.md", outside.path().join("gone.png")),
    ];
    for result in results {
        assert!(matches!(result, Err(Error::Read { .. })), "{result:?}");
    }
    let result = folder.core.attach_file("../a.md", &file);
    assert!(matches!(result, Err(Error::BadPath { .. })), "{result:?}");
    assert!(!folder.path("attachments").exists());
}

#[test]
fn attach_file_fails_when_attachments_is_a_file() {
    let folder = Folder::new(&[("a.md", "# A\n"), ("attachments", "")]);
    let outside = tempfile::tempdir().unwrap();
    let file = outside.path().join("x.png");
    fs::write(&file, "").unwrap();

    let result = folder.core.attach_file("a.md", &file);

    assert!(matches!(result, Err(Error::Write { .. })), "{result:?}");
}

#[test]
fn paste_image_names_the_file_by_the_time() {
    let folder = Folder::new(&[("work/a.md", "# A\n")]);

    let path = folder.core.paste_image("work/a.md", "png", b"png").unwrap();

    let name = path
        .strip_prefix("../attachments")
        .unwrap()
        .to_str()
        .unwrap();
    // Like `Pasted image 2026-11-08 15-30-12.png`.
    let time = name
        .strip_prefix("Pasted image ")
        .and_then(|rest| rest.strip_suffix(".png"))
        .unwrap();
    let shape: String = time
        .chars()
        .map(|c| if c.is_ascii_digit() { '0' } else { c })
        .collect();
    assert_eq!(shape, "0000-00-00 00-00-00", "{name}");
    assert_eq!(
        fs::read(folder.path("attachments").join(name)).unwrap(),
        b"png"
    );
}

#[test]
fn attachments_do_not_show_as_folders_or_notes() {
    let folder = Folder::new(&[("a.md", "# A\n")]);

    folder.core.paste_image("a.md", "png", b"png").unwrap();

    assert_eq!(
        folder.core.folders().unwrap(),
        [] as [std::path::PathBuf; 0]
    );
    assert_eq!(
        folder
            .core
            .notes(holm_core::model::Order::Title)
            .unwrap()
            .len(),
        1
    );
}
