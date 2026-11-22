//! Renaming, moving and deleting notes and folders through `Core` on a real temp folder.

#![allow(clippy::unwrap_used, reason = "tests")]

mod common;

use std::fs;
use std::path::PathBuf;

use common::{Folder, folder_path, note_path};
use holm_core::model::{DeleteCount, LinkTarget, NoteInfo, NoteSet, Order};
use holm_core::{Error, NameProblem};

fn note(id: &str, body: &str) -> String {
    format!("---\nid: {id}\n---\n{body}")
}

fn paths(notes: Vec<NoteInfo>) -> Vec<String> {
    let mut paths = common::paths(notes.into_iter().map(|n| n.path));
    paths.sort();
    paths
}

fn all(folder: &Folder) -> Vec<String> {
    paths(folder.core.snapshot().notes(NoteSet::All, Order::Title))
}

/// Checks that the index after the change equals an index built from the files.
fn assert_index_matches_files(folder: &mut Folder, expected: &[&str]) {
    let before = folder.core.snapshot().notes(NoteSet::All, Order::Title);
    assert_eq!(paths(before.clone()), expected);
    fs::remove_file(folder.index_file()).unwrap();
    folder.reopen();
    assert_eq!(folder.core.snapshot().notes(NoteSet::All, Order::Title), before);
}

fn file_links(folder: &Folder, path: &str) -> Vec<PathBuf> {
    folder
        .core
        .snapshot()
        .links(&note_path(path))
        .to_vec()
        .into_iter()
        .filter_map(|link| match link.target {
            LinkTarget::File(file) => Some(file),
            _ => None,
        })
        .collect()
}

#[test]
fn rename_note_renames_the_file_and_keeps_its_text_and_time() {
    let text = note("A", "# Holm ideas\n\nText\n");
    let mut folder = Folder::new(&[("work/Untitled.md", &text)]);
    let time = folder.set_time("work/Untitled.md", 1_000);

    let info = folder
        .core
        .rename_note(&note_path("work/Untitled.md"), "Plans")
        .unwrap();

    assert_eq!(info.path, note_path("work/Plans.md"));
    assert_eq!(info.title, "Plans");
    assert_eq!(info.modified, time);
    assert_eq!(folder.text("work/Plans.md"), text);
    assert!(!folder.path("work/Untitled.md").exists());
    assert_index_matches_files(&mut folder, &["work/Plans.md"]);
}

#[test]
fn rename_note_refuses_a_taken_or_bad_title_and_moves_nothing() {
    let folder = Folder::new(&[("Plans.md", "# Plans\n"), ("a.md", "# A\n")]);
    let problem = |title: &str| match folder.core.rename_note(&note_path("a.md"), title) {
        Err(Error::BadName { problem, .. }) => Some(problem),
        _ => None,
    };

    assert!(matches!(
        folder.core.rename_note(&note_path("a.md"), "plans"),
        Err(Error::NameTaken { .. })
    ));
    assert_eq!(problem("  "), Some(NameProblem::Empty));
    assert_eq!(problem("a/b"), Some(NameProblem::Slash));
    assert_eq!(problem(".a"), Some(NameProblem::Dot));
    assert_eq!(folder.files(), ["Plans.md", "a.md"]);
}

#[test]
fn rename_note_takes_its_own_name_with_other_capital_letters() {
    let folder = Folder::new(&[("Plans.md", "# Plans\n")]);

    assert_eq!(
        folder.core.rename_note(&note_path("Plans.md"), "Plans").unwrap().path,
        note_path("Plans.md")
    );
    assert_eq!(
        folder.core.rename_note(&note_path("Plans.md"), "plans").unwrap().path,
        note_path("plans.md")
    );
    assert_eq!(folder.files(), ["plans.md"]);
}

#[test]
fn a_save_never_renames_the_note() {
    let folder = Folder::new(&[("Plans.md", "# Plans\n")]);

    folder
        .core
        .save(&note_path("Plans.md"), "# Spring plans\n\nText\n", None)
        .unwrap();

    assert_eq!(folder.files(), ["Plans.md"]);
    assert_eq!(all(&folder), ["Plans.md"]);
}

#[test]
fn a_renamed_note_keeps_its_id_and_its_backlinks() {
    let folder = Folder::new(&[
        ("a.md", &note("A", "# Target\n")),
        ("b.md", &note("B", "[x](holm:A)\n")),
    ]);

    folder.core.rename_note(&note_path("a.md"), "Target").unwrap();

    let backlinks = folder.core.snapshot().backlinks(&note_path("Target.md"));
    assert_eq!(paths(backlinks), ["b.md"]);
    assert_eq!(
        folder
            .core
            .read(&note_path("Target.md"))
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
        .move_note(&note_path("Plans.md"), &folder_path("work/holm"))
        .unwrap();

    assert_eq!(info.path, note_path("work/holm/Plans.md"));
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

    folder.core.move_note(&note_path("a.md"), &folder_path("work")).unwrap();

    assert_eq!(folder.text("work/a.md"), text);
    let mode = fs::metadata(folder.path("work/a.md")).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn move_note_into_a_taken_name_fails_and_moves_nothing() {
    let folder = Folder::new(&[
        ("a.md", "one\n"),
        ("B.md", "b\n"),
        ("work/a.md", "two\n"),
        ("work/b.md", ""),
    ]);

    for from in ["a.md", "B.md"] {
        let error = folder
            .core
            .move_note(&note_path(from), &folder_path("work"))
            .unwrap_err();
        assert!(matches!(error, Error::NameTaken { .. }), "{error:?}");
    }

    assert_eq!(folder.text("a.md"), "one\n");
    assert_eq!(folder.text("work/a.md"), "two\n");
    assert_eq!(all(&folder), ["B.md", "a.md", "work/a.md", "work/b.md"]);
}

#[test]
fn move_note_to_its_own_folder_does_nothing() {
    let folder = Folder::new(&[("work/a.md", "one\n")]);

    let info = folder
        .core
        .move_note(&note_path("work/a.md"), &folder_path("work"))
        .unwrap();

    assert_eq!(info.path, note_path("work/a.md"));
    assert_eq!(all(&folder), ["work/a.md"]);
}

#[test]
fn move_note_to_a_missing_folder_fails_and_keeps_the_note() {
    let folder = Folder::new(&[("a.md", "one\n")]);

    let error = folder
        .core
        .move_note(&note_path("a.md"), &folder_path("missing"))
        .unwrap_err();

    assert!(matches!(error, Error::Write { .. }), "{error:?}");
    assert_eq!(folder.text("a.md"), "one\n");
    assert_eq!(all(&folder), ["a.md"]);
}

#[test]
fn create_folder_keeps_the_typed_name_trimmed() {
    let folder = Folder::new(&[("work/a.md", "")]);

    let core = &folder.core;
    assert_eq!(
        core.create_folder(&folder_path(""), "Ideas: 2026?").unwrap(),
        folder_path("Ideas: 2026?")
    );
    assert_eq!(
        core.create_folder(&folder_path("work"), "attachments").unwrap(),
        folder_path("work/attachments")
    );
    assert_eq!(
        core.create_folder(&folder_path(""), "  Plans  ").unwrap(),
        folder_path("Plans")
    );
    assert!(matches!(
        core.create_folder(&folder_path("missing"), "x"),
        Err(Error::Write { .. })
    ));
    assert_eq!(
        common::paths(core.snapshot().folders().collect::<Vec<_>>()),
        ["Ideas: 2026?", "Plans", "work", "work/attachments"]
    );
}

#[test]
fn create_folder_refuses_a_taken_name() {
    let folder = Folder::new(&[("work/a.md", ""), ("Plans/b.md", ""), ("notes.md", "")]);

    for name in ["work", "Work", "plans", "NOTES.md"] {
        let error = folder.core.create_folder(&folder_path(""), name).unwrap_err();
        assert!(matches!(error, Error::NameTaken { .. }), "{name}: {error:?}");
    }
    assert_eq!(folder.names(""), ["Plans", "notes.md", "work"]);
}

#[test]
fn create_folder_refuses_a_bad_name() {
    let folder = Folder::new(&[("a.md", "")]);
    let long = "x".repeat(256);

    for (name, problem) in [
        ("", NameProblem::Empty),
        ("  ", NameProblem::Empty),
        ("a/b", NameProblem::Slash),
        (".git", NameProblem::Dot),
        ("attachments", NameProblem::Attachments),
        ("Attachments", NameProblem::Attachments),
        (long.as_str(), NameProblem::TooLong),
    ] {
        let error = folder.core.create_folder(&folder_path(""), name).unwrap_err();
        assert!(
            matches!(error, Error::BadName { problem: found, .. } if found == problem),
            "{name}: {error:?}"
        );
    }
    assert!(folder.core.create_folder(&folder_path(""), &"x".repeat(255)).is_ok());
    assert_eq!(folder.core.snapshot().folders().count(), 1);
}

#[test]
fn rename_folder_refuses_a_taken_name_but_takes_its_own_in_other_capitals() {
    let folder = Folder::new(&[("plans/a.md", ""), ("work/b.md", "")]);

    for name in ["work", "Work"] {
        let error = folder.core.rename_folder(&folder_path("plans"), name).unwrap_err();
        assert!(matches!(error, Error::NameTaken { .. }), "{name}: {error:?}");
    }
    let error = folder.core.rename_folder(&folder_path("plans"), "a/b").unwrap_err();
    assert!(matches!(error, Error::BadName { .. }), "{error:?}");
    assert_eq!(folder.names(""), ["plans", "work"]);

    let new = folder.core.rename_folder(&folder_path("plans"), "Plans").unwrap();

    assert_eq!(new, folder_path("Plans"));
    assert_eq!(folder.names(""), ["Plans", "work"]);
    assert_eq!(all(&folder), ["Plans/a.md", "work/b.md"]);
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

    let new = folder.core.rename_folder(&folder_path("work"), "Job").unwrap();

    assert_eq!(new, folder_path("Job"));
    let fixed = "![a](../attachments/a.png) [b](b.md) [self](b.md)\n";
    assert_eq!(folder.text("Job/a.md"), note("A", fixed));
    assert_eq!(
        paths(folder.core.snapshot().backlinks(&note_path("Job/a.md"))),
        ["other.md"]
    );
    assert_eq!(
        common::paths(folder.core.snapshot().folders().collect::<Vec<_>>()),
        ["Job", "Job/sub"]
    );
    assert_index_matches_files(&mut folder, &["Job/a.md", "Job/b.md", "Job/sub/b.md", "other.md"]);
}

#[test]
fn move_folder_fixes_links_to_files_outside_it() {
    let text = note("A", "![a](../attachments/a.png) [b](sub/b.md)\n");
    let mut folder = Folder::new(&[("work/a.md", &text), ("work/sub/b.md", ""), ("archive/x.md", "")]);
    let time = folder.set_time("work/a.md", 1_000);

    let new = folder
        .core
        .move_folder(&folder_path("work"), &folder_path("archive"))
        .unwrap();

    assert_eq!(new, folder_path("archive/work"));
    assert_eq!(
        folder.text("archive/work/a.md"),
        note("A", "![a](../../attachments/a.png) [b](sub/b.md)\n")
    );
    assert_eq!(
        folder.core.read(&note_path("archive/work/a.md")).unwrap().info.modified,
        time
    );
    assert_index_matches_files(
        &mut folder,
        &["archive/work/a.md", "archive/work/sub/b.md", "archive/x.md"],
    );
}

#[test]
fn move_folder_into_a_taken_name_fails_and_moves_nothing() {
    let folder = Folder::new(&[
        ("work/a.md", "one\n"),
        ("plans/b.md", ""),
        ("archive/work/a.md", "two\n"),
        ("archive/Plans/c.md", ""),
    ]);

    for from in ["work", "plans"] {
        let error = folder
            .core
            .move_folder(&folder_path(from), &folder_path("archive"))
            .unwrap_err();
        assert!(matches!(error, Error::NameTaken { .. }), "{from}: {error:?}");
    }

    assert_eq!(folder.text("work/a.md"), "one\n");
    assert_eq!(folder.text("archive/work/a.md"), "two\n");
    assert_eq!(folder.names("archive"), ["Plans", "work"]);
    assert_eq!(folder.names("plans"), ["b.md"]);
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
                core.move_folder(&folder_path(from), &folder_path(to)),
                Err(Error::BadPath { .. })
            ),
            "{from} to {to}"
        );
    }
    assert!(matches!(
        core.rename_folder(&folder_path("attachments"), "x"),
        Err(Error::BadPath { .. })
    ));
    assert!(matches!(
        core.rename_folder(&folder_path("missing"), "x"),
        Err(Error::Read { .. })
    ));
    assert!(matches!(
        core.rename_folder(&folder_path("work/sub/a.md"), "x"),
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
        folder.core.note_delete_count(&note_path("a.md")),
        DeleteCount {
            notes: 1,
            attachments: 1
        }
    );
    folder.core.delete_note(&note_path("a.md")).unwrap();

    assert!(!folder.path("a.md").exists());
    assert!(!folder.path("attachments/only.png").exists());
    assert!(folder.path("attachments/shared.png").exists());
    assert!(folder.path("attachments/x.md").exists());
    assert!(folder.path("docs/doc.pdf").exists());
    assert_index_matches_files(&mut folder, &["attachments/x.md", "work/b.md"]);
}

#[test]
fn only_the_exact_attachments_folder_is_for_attachments() {
    let folder = Folder::new(&[
        ("a.md", &note("A", "![x](Attachments/x.png) ![y](attachments/y.png)\n")),
        ("Attachments/x.png", "png"),
        ("attachments/y.png", "png"),
        ("work/b.md", ""),
    ]);
    let core = &folder.core;

    assert_eq!(common::paths(core.snapshot().folders()), ["Attachments", "work"]);
    assert!(matches!(
        core.rename_folder(&folder_path("work"), "Attachments"),
        Err(Error::BadName {
            problem: NameProblem::Attachments,
            ..
        })
    ));
    assert_eq!(
        core.note_delete_count(&note_path("a.md")),
        DeleteCount {
            notes: 1,
            attachments: 1
        }
    );
    core.delete_note(&note_path("a.md")).unwrap();
    assert!(folder.path("Attachments/x.png").exists());
    assert!(!folder.path("attachments/y.png").exists());

    let moved = core
        .move_folder(&folder_path("Attachments"), &folder_path("work"))
        .unwrap();
    assert_eq!(moved, folder_path("work/Attachments"));
    core.delete_folder(&moved).unwrap();
    assert!(!folder.path("work/Attachments").exists());
}

#[test]
fn delete_note_of_a_gone_note_is_no_error() {
    let folder = Folder::new(&[]);

    folder.core.delete_note(&note_path("gone.md")).unwrap();
}

#[test]
fn delete_folder_deletes_its_notes_and_their_attachments() {
    let mut folder = Folder::new(&[
        ("work/a.md", &note("A", "![a](../attachments/a.png)\n")),
        ("work/sub/b.md", &note("B", "![b](../../attachments/b.png)\n")),
        ("work/image.png", "png"),
        ("keep.md", &note("K", "![b](attachments/b.png)\n")),
        ("attachments/a.png", "png"),
        ("attachments/b.png", "png"),
    ]);

    assert_eq!(
        folder.core.folder_delete_count(&folder_path("work")),
        DeleteCount {
            notes: 2,
            attachments: 1
        }
    );
    folder.core.delete_folder(&folder_path("work")).unwrap();

    assert!(!folder.path("work").exists());
    assert!(!folder.path("attachments/a.png").exists());
    assert!(folder.path("attachments/b.png").exists());
    assert_eq!(
        folder
            .core
            .snapshot()
            .folders()
            .map(common::path_buf)
            .collect::<Vec<_>>(),
        Vec::<PathBuf>::new()
    );
    assert_index_matches_files(&mut folder, &["keep.md"]);
}

#[test]
fn some_folders_can_not_be_deleted() {
    let folder = Folder::new(&[("a.md", ""), ("attachments/a.png", "")]);

    for path in ["", "attachments"] {
        assert!(
            matches!(
                folder.core.delete_folder(&folder_path(path)),
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
    let mut folder = Folder::new(&[("a.md", "---\ntags: [x]\n---\n# A\n"), ("b.md", &note("B", ""))]);
    let time = folder.set_time("a.md", 1_000);
    folder.reopen();

    let id = folder.core.link_id(&note_path("a.md")).unwrap();

    assert_eq!(folder.text("a.md"), format!("---\nid: {id}\ntags: [x]\n---\n# A\n"));
    assert_eq!(fs::metadata(folder.path("a.md")).unwrap().modified().unwrap(), time);
    assert_eq!(folder.core.link_id(&note_path("a.md")).unwrap(), id);
    assert_eq!(folder.core.link_id(&note_path("b.md")).unwrap().as_str(), "B");
    assert_eq!(folder.text("b.md"), note("B", ""));
    let indexed = folder.core.snapshot().notes(NoteSet::All, Order::Title);
    assert_eq!(indexed[0].id.as_ref(), Some(&id));
}
