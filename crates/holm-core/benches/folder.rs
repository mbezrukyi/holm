//! Speed of the core in a notes folder with many notes: the start, the lists, the search and a save.

#![allow(clippy::unwrap_used, reason = "benches")]

use std::fs;
use std::path::Path;

use holm_core::Core;
use holm_core::model::Order;
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
    bencher.bench_local(|| core.snapshot().notes(Order::LastChange));
}

#[divan::bench(sample_count = 50)]
fn sidebar_folders(bencher: divan::Bencher<'_, '_>) {
    let notes = notes_folder();
    let index = tempfile::tempdir().unwrap();
    let core = open(notes.path(), index.path());
    bencher.bench_local(|| {
        core.snapshot()
            .folders()
            .map(Path::to_path_buf)
            .collect::<Vec<_>>()
    });
}

#[divan::bench(sample_count = 50)]
fn sidebar_tags(bencher: divan::Bencher<'_, '_>) {
    let notes = notes_folder();
    let index = tempfile::tempdir().unwrap();
    let core = open(notes.path(), index.path());
    bencher.bench_local(|| core.snapshot().tags());
}

#[divan::bench(sample_count = 50)]
fn search(bencher: divan::Bencher<'_, '_>) {
    let notes = notes_folder();
    let index = tempfile::tempdir().unwrap();
    let core = open(notes.path(), index.path());
    bencher.bench_local(|| core.search("work ho", Order::LastChange).unwrap());
}

/// An autosave: the note file and its index row.
#[divan::bench(sample_count = 50)]
fn save(bencher: divan::Bencher<'_, '_>) {
    let notes = notes_folder();
    let index = tempfile::tempdir().unwrap();
    let core = open(notes.path(), index.path());
    let mut body = core.read("folder 1/Note 1.md").unwrap().body;
    bencher.bench_local(|| {
        body.push_str("word ");
        core.save("folder 1/Note 1.md", &body).result.unwrap()
    });
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
        let path = "folder 1/Note 1.md";
        if flip {
            core.add_tag(path, "new").result.unwrap()
        } else {
            core.remove_tag(path, "new").result.unwrap()
        }
    });
}
