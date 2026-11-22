use std::fs;
use std::path::Path;

use holm_core::Error;
use holm_core_test::{Folder, note_path};

#[test]
fn attach_file_copies_the_file_and_returns_the_path_from_the_note() {
    let folder = Folder::new(&[("a.md", "# A\n"), ("work/b.md", "# B\n")]);
    let outside = tempfile::tempdir().unwrap();
    let file = outside.path().join("photo.jpg");
    fs::write(&file, b"jpg").unwrap();

    let path = folder.core.attach_file(&note_path("work/b.md"), &file).unwrap();

    assert_eq!(path, Path::new("../attachments/photo-f8146b6c.jpg"));
    assert_eq!(fs::read(folder.path("attachments/photo-f8146b6c.jpg")).unwrap(), b"jpg");
    assert_eq!(fs::read(&file).unwrap(), b"jpg");
    assert_eq!(folder.text("work/b.md"), "# B\n");
}

#[test]
fn the_same_file_twice_is_one_copy_and_two_files_are_two() {
    let folder = Folder::new(&[("a.md", "# A\n")]);
    let outside = tempfile::tempdir().unwrap();
    let file = outside.path().join("photo.jpg");
    let other = outside.path().join("other/photo.jpg");
    fs::write(&file, b"jpg").unwrap();
    fs::create_dir(outside.path().join("other")).unwrap();
    fs::write(&other, b"other jpg").unwrap();

    let first = folder.core.attach_file(&note_path("a.md"), &file).unwrap();
    let again = folder.core.attach_file(&note_path("a.md"), &file).unwrap();
    let second = folder.core.attach_file(&note_path("a.md"), &other).unwrap();

    assert_eq!(first, Path::new("attachments/photo-f8146b6c.jpg"));
    assert_eq!(again, first);
    assert_eq!(second, Path::new("attachments/photo-6abacb38.jpg"));
    let mut names: Vec<_> = fs::read_dir(folder.path("attachments"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    assert_eq!(names, ["photo-6abacb38.jpg", "photo-f8146b6c.jpg"]);
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
        folder.core.attach_file(&note_path("a.md"), &hidden).unwrap(),
        Path::new("attachments/-config-e3b0c442")
    );
    assert_eq!(
        folder.core.attach_file(&note_path("a.md"), &plain).unwrap(),
        Path::new("attachments/README-e3b0c442")
    );
}

#[test]
fn attach_file_refuses_a_folder_and_a_missing_file() {
    let folder = Folder::new(&[("a.md", "# A\n")]);
    let outside = tempfile::tempdir().unwrap();
    let file = outside.path().join("x.png");
    fs::write(&file, "").unwrap();

    let results = [
        folder.core.attach_file(&note_path("a.md"), outside.path()),
        folder
            .core
            .attach_file(&note_path("a.md"), outside.path().join("gone.png")),
    ];
    for result in results {
        assert!(matches!(result, Err(Error::Read { .. })), "{result:?}");
    }
    assert!(!folder.path("attachments").exists());
}

#[test]
fn attach_file_fails_when_attachments_is_a_file() {
    let folder = Folder::new(&[("a.md", "# A\n"), ("attachments", "")]);
    let outside = tempfile::tempdir().unwrap();
    let file = outside.path().join("x.png");
    fs::write(&file, "").unwrap();

    let result = folder.core.attach_file(&note_path("a.md"), &file);

    assert!(matches!(result, Err(Error::Write { .. })), "{result:?}");
}

#[test]
fn paste_image_names_the_file_by_its_content() {
    let folder = Folder::new(&[("work/a.md", "# A\n")]);

    let path = folder.core.paste_image(&note_path("work/a.md"), "png", b"png").unwrap();

    assert_eq!(path, Path::new("../attachments/Pasted image-8f8cbb7d.png"));
    assert_eq!(
        fs::read(folder.path("attachments/Pasted image-8f8cbb7d.png")).unwrap(),
        b"png"
    );
}

#[test]
fn attachments_do_not_show_as_folders_or_notes() {
    let folder = Folder::new(&[("a.md", "# A\n")]);

    folder.core.paste_image(&note_path("a.md"), "png", b"png").unwrap();

    assert_eq!(
        folder
            .core
            .snapshot()
            .folders()
            .map(holm_core_test::path_buf)
            .collect::<Vec<_>>(),
        [] as [std::path::PathBuf; 0]
    );
    assert_eq!(
        folder
            .core
            .snapshot()
            .notes(holm_core::model::NoteSet::All, holm_core::model::Order::Title)
            .len(),
        1
    );
}
