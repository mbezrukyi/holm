//! Each write of `Core` names in its change list every note and folder that it changed on disk.

#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::support::Folder;
use holm_core::Core;
use holm_core::model::Change;
use holm_core::testing::{folder_path, note_path, tag_name};

/// Each folder and note file below `root`, by its path from `root`. A note maps to its text.
fn files(root: &Path, dir: &Path, found: &mut BTreeMap<String, Option<String>>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.strip_prefix(root).unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            found.insert(name, None);
            files(root, &path, found);
        } else if path.extension().is_some_and(|ext| ext == "md") {
            found.insert(name, Some(fs::read_to_string(&path).unwrap()));
        }
    }
}

fn disk(folder: &Folder) -> BTreeMap<String, Option<String>> {
    let mut found = BTreeMap::new();
    files(folder.dir.path(), folder.dir.path(), &mut found);
    found
}

fn names(change: &Change, path: &str) -> bool {
    let below = |folder: &dyn std::fmt::Display| path.starts_with(&format!("{folder}/"));
    match change {
        Change::Added(at) | Change::Changed(at) | Change::Removed(at) => at.to_string() == path,
        Change::Moved { from, to } => from.to_string() == path || to.to_string() == path,
        Change::FolderMoved { from, to } => below(from) || below(to),
        _ => false,
    }
}

const FILES: &[(&str, &str)] = &[
    (
        "Plans.md",
        "---\nid: P\ntags: [work, work/holm]\n---\n# Plans\n![](attachments/a.png)\n",
    ),
    ("Ideas.md", "# Ideas\n"),
    ("work/Notes.md", "---\nid: N\ntags: [work]\n---\n[a](../Ideas.md)\n"),
    ("work/holm/Todo.md", "# Todo\n"),
    ("home/Shop.md", "# Shop\n"),
    ("attachments/a.png", "png"),
];

/// Runs `write` on a new folder, and checks that its change lists name each change on disk.
fn check(name: &str, write: impl FnOnce(&Core)) {
    check_after(name, |_| {}, write);
}

/// Runs `prepare` and then `write` on a new folder, and checks the change lists of `write`.
fn check_after(name: &str, prepare: impl FnOnce(&Folder), write: impl FnOnce(&Core)) {
    let folder = Folder::with_feed(FILES);
    prepare(&folder);
    let (_, mut lists) = folder.core.follow();
    let before = disk(&folder);

    write(&folder.core);

    let after = disk(&folder);
    let mut changes = Vec::new();
    while let Some(list) = lists.try_next() {
        changes.extend(list.changes);
    }
    let mut seen = false;
    for (path, text) in before.iter().chain(&after) {
        if text.is_some() && before.get(path) != after.get(path) {
            seen = true;
            assert!(
                changes.iter().any(|change| names(change, path)),
                "{name}: no change names {path} in {changes:?}"
            );
        }
    }
    let folders = |files: &BTreeMap<String, Option<String>>| {
        files
            .iter()
            .filter(|(_, text)| text.is_none())
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>()
    };
    if folders(&before) != folders(&after) {
        seen = true;
        assert!(changes.contains(&Change::Folders), "{name}: no Folders in {changes:?}");
    }
    assert!(seen, "{name}: the write changed nothing on disk");
}

#[test]
fn each_write_names_every_note_and_folder_it_changed() {
    check("save", |core| {
        core.save(&note_path("Ideas.md"), "# More ideas\n", None).unwrap();
    });
    let deleted = Folder::with_feed(FILES).core.read(&note_path("Ideas.md")).unwrap();
    check_after(
        "restore",
        |folder| {
            fs::remove_file(folder.path("Ideas.md")).unwrap();
            folder.feed(["Ideas.md"]);
        },
        |core| {
            core.restore(&deleted).unwrap();
        },
    );
    check("create", |core| {
        core.create(&folder_path("work"), "New").unwrap();
    });
    check("rename_note", |core| {
        core.rename_note(&note_path("work/Notes.md"), "Log").unwrap();
    });
    check("move_note", |core| {
        core.move_note(&note_path("work/Notes.md"), &folder_path("home"))
            .unwrap();
    });
    check("create_folder", |core| {
        core.create_folder(&folder_path("work"), "new").unwrap();
    });
    check("rename_folder", |core| {
        core.rename_folder(&folder_path("work"), "job").unwrap();
    });
    check("move_folder", |core| {
        core.move_folder(&folder_path("work/holm"), &folder_path("home"))
            .unwrap();
    });
    check("delete_note", |core| {
        core.delete_note(&note_path("Plans.md")).unwrap();
    });
    check("delete_folder", |core| {
        core.delete_folder(&folder_path("work")).unwrap();
    });
    check("link_id", |core| {
        core.link_id(&note_path("Ideas.md")).unwrap();
    });
    check("add_tag", |core| {
        core.add_tag(&note_path("Ideas.md"), &tag_name("home")).unwrap();
    });
    check("remove_tag", |core| {
        core.remove_tag(&note_path("Plans.md"), &tag_name("work")).unwrap();
    });
    check("set_pinned", |core| {
        core.set_pinned(&note_path("Ideas.md"), true).unwrap();
    });
    check("rename_tag", |core| {
        core.rename_tag(&tag_name("work"), &tag_name("job")).unwrap();
    });
    check("delete_tag", |core| {
        core.delete_tag(&tag_name("work")).unwrap();
    });
}
