//! Random edits, then a full rebuild of the index must give the same answers.

#![allow(clippy::unwrap_used, reason = "tests")]

mod common;

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use common::Folder;
use holm_core::Core;
use holm_core::model::{Link, NoteInfo, Order, Tag};
use proptest::prelude::*;

const WORDS: [&str; 4] = ["apple", "Berry", "café", "dough"];
const TAGS: [&str; 4] = ["work", "work/holm", "home", "Work/x"];

#[derive(Clone, Debug)]
enum Edit {
    Create {
        in_work: bool,
    },
    Save {
        note: usize,
        words: Vec<usize>,
        link: usize,
        tags: Vec<usize>,
        pinned: bool,
    },
    Outside {
        note: usize,
        words: Vec<usize>,
        tags: Vec<usize>,
    },
    Delete {
        note: usize,
    },
    Copy {
        note: usize,
    },
}

fn edit() -> impl Strategy<Value = Edit> {
    let words = prop::collection::vec(0..WORDS.len(), 0..4);
    let tags = prop::collection::vec(0..TAGS.len(), 0..3);
    prop_oneof![
        any::<bool>().prop_map(|in_work| Edit::Create { in_work }),
        (
            any::<usize>(),
            words.clone(),
            any::<usize>(),
            tags.clone(),
            any::<bool>()
        )
            .prop_map(|(note, words, link, tags, pinned)| Edit::Save {
                note,
                words,
                link,
                tags,
                pinned
            }),
        (any::<usize>(), words, tags).prop_map(|(note, words, tags)| Edit::Outside {
            note,
            words,
            tags
        }),
        any::<usize>().prop_map(|note| Edit::Delete { note }),
        any::<usize>().prop_map(|note| Edit::Copy { note }),
    ]
}

/// Everything the index answers about the notes folder.
#[derive(Debug, PartialEq)]
struct Answers {
    notes: Vec<NoteInfo>,
    by_title: Vec<NoteInfo>,
    in_folders: Vec<Vec<NoteInfo>>,
    tags: Vec<Tag>,
    with_tag: Vec<Vec<NoteInfo>>,
    search: Vec<Vec<NoteInfo>>,
    links: Vec<Vec<Link>>,
    backlinks: Vec<Vec<NoteInfo>>,
}

fn answers(core: &Core) -> Answers {
    let notes = core.notes(Order::LastChange).unwrap();
    let paths: Vec<PathBuf> = notes.iter().map(|n| n.path.clone()).collect();
    Answers {
        by_title: core.notes(Order::Title).unwrap(),
        in_folders: ["", "work"]
            .iter()
            .map(|f| core.notes_in(f, Order::LastChange).unwrap())
            .collect(),
        tags: core.tags().unwrap(),
        with_tag: ["work", "work/holm", "home", "Work"]
            .iter()
            .map(|t| core.notes_with_tag(t, Order::LastChange).unwrap())
            .collect(),
        search: WORDS
            .iter()
            .chain(&["berr", "CAFE"])
            .map(|w| core.search(w, Order::LastChange).unwrap())
            .collect(),
        links: paths.iter().map(|p| core.links(p).unwrap()).collect(),
        backlinks: paths
            .iter()
            .map(|p| core.backlinks(p, Order::LastChange).unwrap())
            .collect(),
        notes,
    }
}

fn body(folder: &Folder, words: &[usize], link: usize) -> String {
    let mut body = String::from("# Note\n");
    for &w in words {
        body.push_str(WORDS[w]);
        body.push(' ');
    }
    let notes = folder.core.notes(Order::Title).unwrap();
    if !notes.is_empty() {
        let target = &notes[link % notes.len()];
        if let Some(id) = &target.id {
            write!(body, "[a](holm:{id}) ").unwrap();
        }
        write!(
            body,
            "[b](</{}>)",
            folder.path("").join(&target.path).display()
        )
        .unwrap();
    }
    body.push('\n');
    body
}

fn tags(picks: &[usize]) -> Vec<String> {
    picks.iter().map(|&t| TAGS[t].to_owned()).collect()
}

fn pick(folder: &Folder, note: usize) -> Option<PathBuf> {
    let notes = folder.core.notes(Order::Title).unwrap();
    (!notes.is_empty()).then(|| notes[note % notes.len()].path.clone())
}

fn apply(folder: &mut Folder, edit: &Edit, copies: &mut usize) {
    match edit {
        Edit::Create { in_work } => {
            folder
                .core
                .create(if *in_work { "work" } else { "" })
                .unwrap();
        }
        Edit::Save {
            note,
            words,
            link,
            tags: picks,
            pinned,
        } => {
            if let Some(path) = pick(folder, *note) {
                let mut note = folder.core.read(&path).unwrap();
                note.body = body(folder, words, *link);
                note.info.tags = tags(picks);
                note.info.pinned = *pinned;
                folder.core.save(&note).unwrap();
            }
        }
        Edit::Outside {
            note,
            words,
            tags: picks,
        } => {
            if let Some(path) = pick(folder, *note) {
                let tags = tags(picks).join(", ");
                let words: Vec<&str> = words.iter().map(|&w| WORDS[w]).collect();
                let text = format!("---\ntags: [{tags}]\n---\n{}\n", words.join(" "));
                fs::write(folder.dir.path().join(path), text).unwrap();
                folder.reopen();
            }
        }
        Edit::Delete { note } => {
            if let Some(path) = pick(folder, *note) {
                fs::remove_file(folder.dir.path().join(path)).unwrap();
                folder.reopen();
            }
        }
        Edit::Copy { note } => {
            if let Some(path) = pick(folder, *note) {
                *copies += 1;
                let copy = path.with_file_name(format!("copy {copies}.md"));
                fs::copy(folder.dir.path().join(&path), folder.dir.path().join(copy)).unwrap();
                folder.reopen();
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn a_full_rebuild_equals_the_index_after_edits(edits in prop::collection::vec(edit(), 1..16)) {
        let mut folder = Folder::new(&[("work/start.md", "# Start\napple\n")]);
        let mut copies = 0;
        for edit in &edits {
            apply(&mut folder, edit, &mut copies);
        }

        let cache = tempfile::tempdir().unwrap();
        let rebuilt = Core::open(folder.dir.path(), cache.path()).unwrap();

        prop_assert_eq!(answers(&folder.core), answers(&rebuilt));
    }
}
