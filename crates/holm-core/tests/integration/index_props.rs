//! Random edits, then a fresh reconcile from empty facts must give the same answers. A new index folder makes the start
//! reconcile every file from empty facts.

#![cfg(test)]

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use crate::support::Folder;
use holm_core::model::{LinkTarget, NoteInfo, NoteSet, Order, Tag};
use holm_core::testing::{folder_path, tag_name};
use holm_core::{Core, NotePath, TagName};
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
        (any::<usize>(), words, tags).prop_map(|(note, words, tags)| Edit::Outside { note, words, tags }),
        any::<usize>().prop_map(|note| Edit::Delete { note }),
        any::<usize>().prop_map(|note| Edit::Copy { note }),
    ]
}

#[derive(Debug, PartialEq)]
struct Answers {
    notes: Vec<NoteInfo>,
    folders: Vec<PathBuf>,
    by_title: Vec<NoteInfo>,
    in_folders: Vec<Vec<NoteInfo>>,
    tags: Vec<Tag>,
    with_tag: Vec<Vec<NoteInfo>>,
    search: Vec<Vec<NotePath>>,
    links: Vec<Vec<LinkTarget>>,
    backlinks: Vec<Vec<NoteInfo>>,
}

fn answers(core: &Core) -> Answers {
    let notes = core.snapshot().notes(NoteSet::All, Order::LastChange);
    let paths: Vec<NotePath> = notes.iter().map(|n| n.path.clone()).collect();
    Answers {
        folders: core.snapshot().folders().map(crate::support::path_buf).collect(),
        by_title: core.snapshot().notes(NoteSet::All, Order::Title),
        in_folders: ["", "work"]
            .iter()
            .map(|f| core.snapshot().notes(NoteSet::In(&folder_path(f)), Order::LastChange))
            .collect(),
        tags: core.snapshot().tags(),
        with_tag: ["work", "work/holm", "home", "Work"]
            .iter()
            .map(|t| core.snapshot().notes(NoteSet::WithTag(&tag_name(t)), Order::LastChange))
            .collect(),
        search: WORDS
            .iter()
            .chain(&["berr", "CAFE"])
            .map(|w| core.search(w, Order::LastChange).unwrap().into_parts().1)
            .collect(),
        links: paths.iter().map(|p| core.snapshot().links(p).to_vec()).collect(),
        backlinks: paths.iter().map(|p| core.snapshot().backlinks(p)).collect(),
        notes,
    }
}

fn body(folder: &Folder, words: &[usize], link: usize) -> String {
    let mut body = String::from("# Note\n");
    for &w in words {
        body.push_str(WORDS[w]);
        body.push(' ');
    }
    let notes = folder.core.snapshot().notes(NoteSet::All, Order::Title);
    if !notes.is_empty() {
        let target = &notes[link % notes.len()];
        if let Some(id) = &target.id {
            write!(body, "[a](holm:{id}) ").unwrap();
        }
        write!(body, "[b](</{}>)", folder.path("").join(&target.path).display()).unwrap();
    }
    body.push('\n');
    body
}

fn tags(picks: &[usize]) -> Vec<String> {
    picks.iter().map(|&t| TAGS[t].to_owned()).collect()
}

fn pick(folder: &Folder, note: usize) -> Option<NotePath> {
    let notes = folder.core.snapshot().notes(NoteSet::All, Order::Title);
    (!notes.is_empty()).then(|| notes[note % notes.len()].path.clone())
}

fn apply(folder: &mut Folder, edit: &Edit, copies: &mut usize) {
    match edit {
        Edit::Create { in_work } => {
            let place = if *in_work { "work" } else { "" };
            let free = (1..=folder.core.snapshot().notes(NoteSet::All, Order::Title).len() + 1)
                .map(|n| format!("Note {n}"))
                .find(|title| !folder.path(place).join(format!("{title}.md")).exists())
                .unwrap();
            folder.core.create(&folder_path(place), &free).unwrap();
        }
        Edit::Save {
            note,
            words,
            link,
            tags: picks,
            pinned,
        } => {
            if let Some(path) = pick(folder, *note) {
                let body = body(folder, words, *link);
                folder.core.save(&path, &body, None).unwrap();
                for tag in tags(picks) {
                    let tag = TagName::typed(&tag).unwrap();
                    let has = folder.core.read(&path).unwrap().info.tags.contains(&tag);
                    let info = if has {
                        folder.core.remove_tag(&path, &tag)
                    } else {
                        folder.core.add_tag(&path, &tag)
                    };
                    info.unwrap();
                }
                folder.core.set_pinned(&path, *pinned).unwrap();
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
                fs::write(folder.dir.path().join(&path), text).unwrap();
                folder.feed([&path]);
            }
        }
        Edit::Delete { note } => {
            if let Some(path) = pick(folder, *note) {
                fs::remove_file(folder.dir.path().join(&path)).unwrap();
                folder.feed([&path]);
            }
        }
        Edit::Copy { note } => {
            if let Some(path) = pick(folder, *note) {
                *copies += 1;
                let copy = path.as_ref().with_file_name(format!("copy {copies}.md"));
                fs::copy(folder.dir.path().join(&path), folder.dir.path().join(&copy)).unwrap();
                folder.feed([copy]);
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn a_fresh_reconcile_equals_the_facts_after_edits(edits in prop::collection::vec(edit(), 1..16)) {
        let mut folder = Folder::with_feed(&[("work/start.md", "# Start\napple\n")]);
        let mut copies = 0;
        for edit in &edits {
            apply(&mut folder, edit, &mut copies);
        }

        let cache = tempfile::tempdir().unwrap();
        let fresh = Core::open(folder.dir.path(), cache.path()).unwrap();

        prop_assert_eq!(answers(&folder.core), answers(&fresh));
    }
}
