//! Renaming, moving and deleting notes and folders through `Core` on a real temp folder.

#![allow(clippy::unwrap_used, reason = "tests")]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::Folder;
use holm_core::Error;
use holm_core::model::{DeleteCount, LinkTarget, NoteInfo, Order};

fn note(id: &str, body: &str) -> String {
    format!("---\nid: {id}\n---\n{body}")
}

fn paths(notes: Vec<NoteInfo>) -> Vec<String> {
    let mut paths = common::paths(notes.into_iter().map(|n| n.path));
    paths.sort();
    paths
}

fn all(folder: &Folder) -> Vec<String> {
    paths(folder.core.snapshot().notes(Order::Title))
}

/// Checks that the index after the change equals an index built from the files.
fn assert_index_matches_files(folder: &mut Folder, expected: &[&str]) {
    let before = folder.core.snapshot().notes(Order::Title);
    assert_eq!(paths(before.clone()), expected);
    fs::remove_file(folder.index_file()).unwrap();
    folder.reopen();
    assert_eq!(folder.core.snapshot().notes(Order::Title), before);
}

fn file_links(folder: &Folder, path: &str) -> Vec<PathBuf> {
    folder
        .core
        .snapshot()
        .links(path)
        .to_vec()
        .into_iter()
        .filter_map(|link| match link.target {
            LinkTarget::File(file) => Some(file),
            _ => None,
        })
        .collect()
}

#[test]
fn rename_to_title_renames_the_file_and_keeps_its_text_and_time() {
    let text = note("A", "# Holm ideas\n\nText\n");
    let mut folder = Folder::new(&[("work/Untitled.md", &text)]);
    let time = folder.set_time("work/Untitled.md", 1_000);

    let info = folder
        .core
        .rename_to_title("work/Untitled.md")
        .result
        .unwrap();

    assert_eq!(info.path, Path::new("work/Holm ideas.md"));
    assert_eq!(info.modified, time);
    assert_eq!(folder.text("work/Holm ideas.md"), text);
    assert!(!folder.path("work/Untitled.md").exists());
    assert_index_matches_files(&mut folder, &["work/Holm ideas.md"]);
}

#[test]
fn rename_to_title_replaces_only_a_slash_and_a_leading_dot() {
    let folder = Folder::new(&[
        ("a.md", "# Plans 1/2: *big*?\n"),
        ("b.md", "# .config notes\n"),
    ]);

    assert_eq!(
        folder.core.rename_to_title("a.md").result.unwrap().path,
        Path::new("Plans 1-2: big?.md")
    );
    assert_eq!(
        folder.core.rename_to_title("b.md").result.unwrap().path,
        Path::new("-config notes.md")
    );
}

#[test]
fn rename_to_title_adds_a_number_on_a_clash() {
    let folder = Folder::new(&[
        ("Plans.md", "# Plans\n"),
        ("a.md", "# Plans\n"),
        ("b.md", "# Plans\n"),
    ]);

    assert_eq!(
        folder.core.rename_to_title("a.md").result.unwrap().path,
        Path::new("Plans 2.md")
    );
    assert_eq!(
        folder.core.rename_to_title("b.md").result.unwrap().path,
        Path::new("Plans 3.md")
    );
    assert_eq!(all(&folder), ["Plans 2.md", "Plans 3.md", "Plans.md"]);
}

#[test]
fn a_note_whose_name_matches_its_title_stays() {
    let folder = Folder::new(&[
        ("Plans.md", "# Plans\n"),
        ("Plans 2.md", "# Plans\n"),
        ("Notes.md", "No title\n"),
    ]);

    for path in ["Plans.md", "Plans 2.md", "Notes.md"] {
        assert_eq!(
            folder.core.rename_to_title(path).result.unwrap().path,
            Path::new(path)
        );
    }
    assert_eq!(folder.files(), ["Notes.md", "Plans 2.md", "Plans.md"]);
}

#[test]
fn a_renamed_note_keeps_its_id_and_its_backlinks() {
    let folder = Folder::new(&[
        ("a.md", &note("A", "# Target\n")),
        ("b.md", &note("B", "[x](holm:A)\n")),
    ]);

    folder.core.rename_to_title("a.md").result.unwrap();

    let backlinks = folder.core.snapshot().backlinks("Target.md", Order::Title);
    assert_eq!(paths(backlinks), ["b.md"]);
    assert_eq!(
        folder
            .core
            .read("Target.md")
            .unwrap()
            .info
            .id
            .unwrap()
            .as_str(),
        "A"
    );
}

#[test]
fn move_note_fixes_its_relative_file_links() {
    let body = "\
# Plans

![shot](attachments/shot.png) and [doc](<docs/My file.pdf> \"Title\")
[up](./work/x.md) [abs](/etc/hosts) [web](https://example.com) [note](holm:B)
";
    let mut folder = Folder::new(&[("Plans.md", &note("A", body)), ("work/holm/keep.md", "")]);
    let time = folder.set_time("Plans.md", 1_000);
    let before = file_links(&folder, "Plans.md");

    let info = folder
        .core
        .move_note("Plans.md", "work/holm")
        .result
        .unwrap();

    assert_eq!(info.path, Path::new("work/holm/Plans.md"));
    assert_eq!(info.modified, time);
    let fixed = "\
# Plans

![shot](../../attachments/shot.png) and [doc](<../../docs/My file.pdf> \"Title\")
[up](../x.md) [abs](/etc/hosts) [web](https://example.com) [note](holm:B)
";
    assert_eq!(folder.text("work/holm/Plans.md"), note("A", fixed));
    assert_eq!(file_links(&folder, "work/holm/Plans.md"), before);
    assert_index_matches_files(&mut folder, &["work/holm/Plans.md", "work/holm/keep.md"]);
}

#[test]
fn move_note_keeps_the_text_and_permissions_when_no_link_changes() {
    use std::os::unix::fs::PermissionsExt as _;

    let text = "---\nid: A\nextra: \"kept\"\n---\r\n[x](/abs/file.pdf)\r\n";
    let folder = Folder::new(&[("a.md", text), ("work/b.md", "")]);
    fs::set_permissions(folder.path("a.md"), fs::Permissions::from_mode(0o600)).unwrap();

    folder.core.move_note("a.md", "work").result.unwrap();

    assert_eq!(folder.text("work/a.md"), text);
    let mode = fs::metadata(folder.path("work/a.md"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn move_note_adds_a_number_on_a_clash() {
    let folder = Folder::new(&[("a.md", "one\n"), ("work/a.md", "two\n")]);

    let info = folder.core.move_note("a.md", "work").result.unwrap();

    assert_eq!(info.path, Path::new("work/a 2.md"));
    assert_eq!(folder.text("work/a.md"), "two\n");
    assert_eq!(folder.text("work/a 2.md"), "one\n");
}

#[test]
fn move_note_to_its_own_folder_does_nothing() {
    let folder = Folder::new(&[("work/a.md", "one\n")]);

    let info = folder.core.move_note("work/a.md", "work").result.unwrap();

    assert_eq!(info.path, Path::new("work/a.md"));
    assert_eq!(all(&folder), ["work/a.md"]);
}

#[test]
fn move_note_to_a_missing_folder_fails_and_keeps_the_note() {
    let folder = Folder::new(&[("a.md", "one\n")]);

    let error = folder.core.move_note("a.md", "missing").result.unwrap_err();

    assert!(matches!(error, Error::Write { .. }), "{error:?}");
    assert_eq!(folder.text("a.md"), "one\n");
    assert_eq!(all(&folder), ["a.md"]);
}

#[test]
fn create_folder_cleans_the_name_and_adds_a_number_on_a_clash() {
    let folder = Folder::new(&[("work/a.md", "")]);

    let core = &folder.core;
    assert_eq!(
        core.create_folder("", "Ideas").result.unwrap(),
        Path::new("Ideas")
    );
    assert_eq!(
        core.create_folder("", " Ideas ").result.unwrap(),
        Path::new("Ideas 2")
    );
    assert_eq!(
        core.create_folder("", "work").result.unwrap(),
        Path::new("work 2")
    );
    assert_eq!(
        core.create_folder("work", "a/b").result.unwrap(),
        Path::new("work/a-b")
    );
    assert_eq!(
        core.create_folder("", ".git").result.unwrap(),
        Path::new("-git")
    );
    assert_eq!(
        core.create_folder("", "attachments").result.unwrap(),
        Path::new("attachments 2")
    );
    assert!(matches!(
        core.create_folder("", "  ").result,
        Err(Error::BadPath { .. })
    ));
    assert!(matches!(
        core.create_folder("missing", "x").result,
        Err(Error::Write { .. })
    ));
    assert_eq!(
        common::paths(
            core.snapshot()
                .folders()
                .map(Path::to_path_buf)
                .collect::<Vec<_>>()
        ),
        [
            "-git",
            "Ideas",
            "Ideas 2",
            "attachments 2",
            "work",
            "work/a-b",
            "work 2"
        ]
    );
}

#[test]
fn rename_folder_moves_its_notes_and_keeps_links_that_still_work() {
    let inside = "![a](../attachments/a.png) [b](b.md) [self](../work/b.md)\n";
    let mut folder = Folder::new(&[
        ("work/a.md", &note("A", inside)),
        ("work/sub/b.md", ""),
        ("work/b.md", ""),
        ("other.md", &note("O", "[a](holm:A)\n")),
    ]);

    let new = folder.core.rename_folder("work", "Job").result.unwrap();

    assert_eq!(new, Path::new("Job"));
    let fixed = "![a](../attachments/a.png) [b](b.md) [self](b.md)\n";
    assert_eq!(folder.text("Job/a.md"), note("A", fixed));
    assert_eq!(
        paths(folder.core.snapshot().backlinks("Job/a.md", Order::Title)),
        ["other.md"]
    );
    assert_eq!(
        common::paths(
            folder
                .core
                .snapshot()
                .folders()
                .map(Path::to_path_buf)
                .collect::<Vec<_>>()
        ),
        ["Job", "Job/sub"]
    );
    assert_index_matches_files(
        &mut folder,
        &["Job/a.md", "Job/b.md", "Job/sub/b.md", "other.md"],
    );
}

#[test]
fn move_folder_fixes_links_to_files_outside_it() {
    let text = note("A", "![a](../attachments/a.png) [b](sub/b.md)\n");
    let mut folder = Folder::new(&[
        ("work/a.md", &text),
        ("work/sub/b.md", ""),
        ("archive/x.md", ""),
    ]);
    let time = folder.set_time("work/a.md", 1_000);

    let new = folder.core.move_folder("work", "archive").result.unwrap();

    assert_eq!(new, Path::new("archive/work"));
    assert_eq!(
        folder.text("archive/work/a.md"),
        note("A", "![a](../../attachments/a.png) [b](sub/b.md)\n")
    );
    assert_eq!(
        folder.core.read("archive/work/a.md").unwrap().info.modified,
        time
    );
    assert_index_matches_files(
        &mut folder,
        &["archive/work/a.md", "archive/work/sub/b.md", "archive/x.md"],
    );
}

#[test]
fn move_folder_adds_a_number_on_a_clash() {
    let folder = Folder::new(&[("work/a.md", "one\n"), ("archive/work/a.md", "two\n")]);

    let new = folder.core.move_folder("work", "archive").result.unwrap();

    assert_eq!(new, Path::new("archive/work 2"));
    assert_eq!(folder.text("archive/work/a.md"), "two\n");
    assert_eq!(folder.text("archive/work 2/a.md"), "one\n");
}

#[test]
fn some_folders_can_not_move() {
    let folder = Folder::new(&[("work/sub/a.md", ""), ("attachments/a.png", "")]);
    let core = &folder.core;

    for (from, to) in [
        ("work", "work"),
        ("work", "work/sub"),
        ("attachments", "work"),
        ("", "work"),
    ] {
        assert!(
            matches!(
                core.move_folder(from, to).result,
                Err(Error::BadPath { .. })
            ),
            "{from} to {to}"
        );
    }
    assert!(matches!(
        core.rename_folder("attachments", "x").result,
        Err(Error::BadPath { .. })
    ));
    assert!(matches!(
        core.rename_folder("missing", "x").result,
        Err(Error::Read { .. })
    ));
    assert!(matches!(
        core.rename_folder("work/sub/a.md", "x").result,
        Err(Error::Read { .. })
    ));
    assert_eq!(all(&folder), ["work/sub/a.md"]);
}

#[test]
fn delete_note_deletes_only_attachments_that_no_other_note_links() {
    let mut folder = Folder::new(&[
        (
            "a.md",
            &note(
                "A",
                "![only](attachments/only.png) ![shared](attachments/shared.png) \
                 [outside](docs/doc.pdf) ![note](attachments/x.md) ![gone](attachments/gone.png)\n",
            ),
        ),
        ("work/b.md", &note("B", "[s](../attachments/shared.png)\n")),
        ("attachments/only.png", "png"),
        ("attachments/shared.png", "png"),
        ("attachments/x.md", ""),
        ("docs/doc.pdf", "pdf"),
    ]);

    assert_eq!(
        folder.core.note_delete_count("a.md").unwrap(),
        DeleteCount {
            notes: 1,
            attachments: 1
        }
    );
    folder.core.delete_note("a.md").result.unwrap();

    assert!(!folder.path("a.md").exists());
    assert!(!folder.path("attachments/only.png").exists());
    assert!(folder.path("attachments/shared.png").exists());
    assert!(folder.path("attachments/x.md").exists());
    assert!(folder.path("docs/doc.pdf").exists());
    assert_index_matches_files(&mut folder, &["attachments/x.md", "work/b.md"]);
}

#[test]
fn delete_note_of_a_gone_note_is_no_error() {
    let folder = Folder::new(&[]);

    folder.core.delete_note("gone.md").result.unwrap();
}

#[test]
fn delete_folder_deletes_its_notes_and_their_attachments() {
    let mut folder = Folder::new(&[
        ("work/a.md", &note("A", "![a](../attachments/a.png)\n")),
        (
            "work/sub/b.md",
            &note("B", "![b](../../attachments/b.png)\n"),
        ),
        ("work/image.png", "png"),
        ("keep.md", &note("K", "![b](attachments/b.png)\n")),
        ("attachments/a.png", "png"),
        ("attachments/b.png", "png"),
    ]);

    assert_eq!(
        folder.core.folder_delete_count("work").unwrap(),
        DeleteCount {
            notes: 2,
            attachments: 1
        }
    );
    folder.core.delete_folder("work").result.unwrap();

    assert!(!folder.path("work").exists());
    assert!(!folder.path("attachments/a.png").exists());
    assert!(folder.path("attachments/b.png").exists());
    assert_eq!(
        folder
            .core
            .snapshot()
            .folders()
            .map(Path::to_path_buf)
            .collect::<Vec<_>>(),
        Vec::<PathBuf>::new()
    );
    assert_index_matches_files(&mut folder, &["keep.md"]);
}

#[test]
fn some_folders_can_not_be_deleted() {
    let folder = Folder::new(&[("a.md", ""), ("attachments/a.png", "")]);

    for path in ["", "attachments", "../x"] {
        assert!(
            matches!(
                folder.core.delete_folder(path).result,
                Err(Error::BadPath { .. })
            ),
            "{path}"
        );
    }
    assert!(folder.path("a.md").exists());
    assert!(folder.path("attachments/a.png").exists());
}

#[test]
fn link_id_adds_only_the_id_line_and_keeps_the_time() {
    let mut folder = Folder::new(&[
        ("a.md", "---\ntags: [x]\n---\n# A\n"),
        ("b.md", &note("B", "")),
    ]);
    let time = folder.set_time("a.md", 1_000);
    folder.reopen();

    let id = folder.core.link_id("a.md").result.unwrap();

    assert_eq!(
        folder.text("a.md"),
        format!("---\nid: {id}\ntags: [x]\n---\n# A\n")
    );
    assert_eq!(
        fs::metadata(folder.path("a.md"))
            .unwrap()
            .modified()
            .unwrap(),
        time
    );
    assert_eq!(folder.core.link_id("a.md").result.unwrap(), id);
    assert_eq!(folder.core.link_id("b.md").result.unwrap().as_str(), "B");
    assert_eq!(folder.text("b.md"), note("B", ""));
    let indexed = folder.core.snapshot().notes(Order::Title);
    assert_eq!(indexed[0].id.as_ref(), Some(&id));
}
