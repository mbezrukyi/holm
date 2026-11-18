//! Speed of the core in a notes folder with many notes: the start, the lists, the search, saves
//! and a tag rename on the real disk.

#![allow(clippy::unwrap_used, reason = "benches")]

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::Command;

use holm_core::model::{NoteSet, Order};
use holm_core::{Core, NotePath};
use tempfile::TempDir;

const NOTES: usize = 5_000;
const FOLDERS: usize = 50;
const TAGS: usize = 100;

fn main() {
    divan::main();
}

/// A notes folder with [`NOTES`] notes in [`FOLDERS`] folders. Each note has an id, two tags,
/// two note links, a file link and about 2 KB of text.
fn notes_folder() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    for folder in 0..FOLDERS {
        fs::create_dir(dir.path().join(format!("folder {folder}"))).unwrap();
    }
    let words = "a line of plain words that a person may write in a note about work and home. ";
    for note in 0..NOTES {
        let text = format!(
            "---\nid: N{note:05}\ntags: [tag{}, area/tag{}]\n---\n# Note {note}\n\n{}\n\n\
            See [this](holm:N{:05}) and [that](holm:N{:05}), and [a file](attachments/f{note}.pdf).\n\n{}\n",
            note % TAGS,
            (note / 7) % TAGS,
            words.repeat(12),
            (note + 1) % NOTES,
            (note + 2) % NOTES,
            words.repeat(12),
        );
        let path = dir
            .path()
            .join(format!("folder {}", note % FOLDERS))
            .join(format!("Note {note}.md"));
        fs::write(path, text).unwrap();
    }
    dir
}

fn open(notes: &Path, index: &Path) -> Core {
    Core::open(notes, index).unwrap()
}

/// The first start: Holm reads each note and builds the index.
#[divan::bench(sample_count = 5)]
fn start_with_new_index(bencher: divan::Bencher<'_, '_>) {
    let notes = notes_folder();
    bencher
        .with_inputs(|| tempfile::tempdir().unwrap())
        .bench_local_values(|index| open(notes.path(), index.path()));
}

/// A later start: the index knows each note, so Holm only checks the file stamps.
#[divan::bench(sample_count = 20)]
fn start_with_index(bencher: divan::Bencher<'_, '_>) {
    let notes = notes_folder();
    let index = tempfile::tempdir().unwrap();
    drop(open(notes.path(), index.path()));
    bencher.bench_local(|| open(notes.path(), index.path()));
}

/// The sidebar reads the notes, the folders and the tags after each change in the notes folder.
#[divan::bench(sample_count = 50)]
fn sidebar_notes(bencher: divan::Bencher<'_, '_>) {
    let notes = notes_folder();
    let index = tempfile::tempdir().unwrap();
    let core = open(notes.path(), index.path());
    bencher.bench_local(|| core.snapshot().notes(NoteSet::All, Order::LastChange));
}

/// The sidebar keeps only the paths of the note list.
#[divan::bench(sample_count = 50)]
fn sidebar_paths(bencher: divan::Bencher<'_, '_>) {
    let notes = notes_folder();
    let index = tempfile::tempdir().unwrap();
    let core = open(notes.path(), index.path());
    bencher.bench_local(|| core.snapshot().paths(NoteSet::All, Order::LastChange));
}

#[divan::bench(sample_count = 50)]
fn sidebar_folders(bencher: divan::Bencher<'_, '_>) {
    let notes = notes_folder();
    let index = tempfile::tempdir().unwrap();
    let core = open(notes.path(), index.path());
    bencher.bench_local(|| core.snapshot().folders().cloned().collect::<Vec<_>>());
}

#[divan::bench(sample_count = 50)]
fn sidebar_tags(bencher: divan::Bencher<'_, '_>) {
    let notes = notes_folder();
    let index = tempfile::tempdir().unwrap();
    let core = open(notes.path(), index.path());
    bencher.bench_local(|| core.snapshot().tags());
}

/// A search key, with the results in the order of the list. `"work ho"` matches every note,
/// and `"note 50"` 11 notes. `"tag"` matches every note through its tags, and `"tag work"`
/// through a tag and the text.
#[divan::bench(sample_count = 50, args = ["work ho", "note 50", "tag work", "tag"])]
fn search(bencher: divan::Bencher<'_, '_>, text: &str) {
    let notes = notes_folder();
    let index = tempfile::tempdir().unwrap();
    let core = open(notes.path(), index.path());
    bencher.bench_local(|| {
        let found = core.search(text).unwrap();
        core.snapshot().sorted(found, Order::LastChange)
    });
}

/// An autosave: the note file and its index row. The 2 KB note is one of the folder.
/// The bigger ones are new notes with headings, lists, links and code.
#[divan::bench(sample_count = 50, args = [2_000, 200_000, 1_000_000])]
fn save(bencher: divan::Bencher<'_, '_>, size: usize) {
    let notes = notes_folder();
    let index = tempfile::tempdir().unwrap();
    let core = open(notes.path(), index.path());
    let path = if size > 2_000 {
        fs::write(notes.path().join("Big.md"), big_note(size)).unwrap();
        "Big.md"
    } else {
        "folder 1/Note 1.md"
    };
    let path = &NotePath::new(path).unwrap();
    let mut body = core.read(path).unwrap().body;
    bencher.bench_local(|| {
        body.push_str("word ");
        core.save(path, &body).unwrap()
    });
}

/// A note text of about `size` bytes.
fn big_note(size: usize) -> String {
    let mut text = String::from("---\nid: BIG\ntags: [big]\n---\n# Big\n\n");
    let mut part = 0;
    while text.len() < size {
        writeln!(
            text,
            "## Part {part}\n\nSome *plain* words with **strong** text, `code` and a \
            [link](holm:N{:05}). A line about work and home.\n\n\
            - one item\n- two items with [a file](attachments/f{part}.pdf)\n  - a nested item\n\n\
            ```rust\nfn part_{part}() {{}}\n```\n",
            part % NOTES,
        )
        .unwrap();
        part += 1;
    }
    text
}

/// A tag change from the tag row: the frontmatter and the index row. The body stays.
#[divan::bench(sample_count = 50)]
fn add_or_remove_tag(bencher: divan::Bencher<'_, '_>) {
    let notes = notes_folder();
    let index = tempfile::tempdir().unwrap();
    let core = open(notes.path(), index.path());
    let mut flip = false;
    bencher.bench_local(|| {
        flip = !flip;
        let path = &NotePath::new("folder 1/Note 1.md").unwrap();
        if flip {
            core.add_tag(path, "new").unwrap()
        } else {
            core.remove_tag(path, "new").unwrap()
        }
    });
}

/// A tag rename on 1,000 notes in 10 folders, back and forth. It runs on the disk of `target/`,
/// not on tmpfs, because the time goes to the syncs.
#[divan::bench(sample_count = 20)]
fn rename_tag_on_disk(bencher: divan::Bencher<'_, '_>) {
    let disk = env!("CARGO_TARGET_TMPDIR");
    let notes = tempfile::tempdir_in(disk).unwrap();
    let index = tempfile::tempdir_in(disk).unwrap();
    let file_system = Command::new("stat")
        .args(["-f", "-c", "%T"])
        .arg(notes.path())
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_default();
    eprintln!("rename_tag_on_disk: {disk} is {file_system}");
    for folder in 0..10 {
        fs::create_dir(notes.path().join(format!("folder {folder}"))).unwrap();
    }
    let words = "a line of plain words that a person may write in a note about work and home. ";
    for note in 0..1_000 {
        let text = format!(
            "---\nid: N{note:05}\ntags: [old, tag{note}]\n---\n# Note {note}\n\n{}\n",
            words.repeat(24),
        );
        let path = notes
            .path()
            .join(format!("folder {}", note % 10))
            .join(format!("Note {note}.md"));
        fs::write(path, text).unwrap();
    }
    let core = open(notes.path(), index.path());
    let mut flip = false;
    bencher.bench_local(|| {
        flip = !flip;
        let (from, to) = if flip { ("old", "new") } else { ("new", "old") };
        assert_eq!(core.rename_tag(from, to).unwrap(), 1_000);
    });
}
