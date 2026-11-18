//! Outside changes to the notes folder, pushed by hand through the `Feed`,
//! and a few through real file events.

#![allow(clippy::unwrap_used, reason = "tests")]

mod common;

use common::{folder_path, note_path};

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::pin::Pin;
use std::task::Poll;
use std::time::{Duration, Instant};

use futures_core::Stream;

use holm_core::model::{Change, ChangeList, LinkTarget, NoteId, NoteInfo, NoteSet, Order};
use holm_core::{Changes, Core, Error, Feed, NotePath};
use tempfile::TempDir;

/// A notes folder with a core that sees changes only through its feed.
struct Fed {
    dir: TempDir,
    _cache: TempDir,
    core: Core,
    feed: Feed,
    changes: RefCell<Changes>,
}

impl Fed {
    fn new(files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            write(&dir.path().join(path), text);
        }
        let cache = tempfile::tempdir().unwrap();
        let (core, feed) = Core::open_with_feed(dir.path(), cache.path()).unwrap();
        let changes = core.changes();
        Self {
            dir,
            _cache: cache,
            core,
            feed,
            changes: RefCell::new(changes),
        }
    }

    fn path(&self, path: &str) -> PathBuf {
        self.dir.path().join(path)
    }

    fn write(&self, path: &str, text: &str) {
        write(&self.path(path), text);
    }

    /// Every change that is waiting now, from all change lists.
    fn take(&self) -> Vec<Change> {
        self.lists()
            .into_iter()
            .flat_map(|list| list.changes)
            .collect()
    }

    /// Every change list that is waiting now.
    fn lists(&self) -> Vec<ChangeList> {
        std::iter::from_fn(|| self.changes.borrow_mut().try_next()).collect()
    }

    fn info(&self, path: &str) -> NoteInfo {
        self.core.read(&note_path(path)).unwrap().info
    }
}

fn write(path: &std::path::Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn note(id: &str, extra: &str, body: &str) -> String {
    format!("---\nid: {id}\n{extra}---\n{body}")
}

fn previews(core: &Core) -> Vec<String> {
    let mut previews: Vec<String> = core
        .snapshot()
        .notes(NoteSet::All, Order::Title)
        .into_iter()
        .map(|n| n.preview)
        .collect();
    previews.sort();
    previews
}

#[test]
fn an_outside_edit_updates_the_index_and_sends_a_change() {
    let fed = Fed::new(&[("a.md", &note("A", "", "# Old\n"))]);

    fed.write("a.md", &note("A", "", "# New words\n"));
    fed.feed.changed(["a.md"]).unwrap();

    assert_eq!(fed.take(), [Change::Changed(fed.info("a.md"))]);
    assert_eq!(previews(&fed.core), ["New words"]);
    assert_eq!(fed.core.search("words").unwrap().len(), 1);
}

#[test]
fn a_new_file_is_added() {
    let fed = Fed::new(&[]);

    fed.write("b.md", "# B\n");
    fed.feed.changed(["b.md"]).unwrap();

    assert_eq!(fed.take(), [Change::Added(fed.info("b.md"))]);
    assert_eq!(previews(&fed.core), ["B"]);
}

#[test]
fn a_move_in_two_batches_with_the_old_path_first_is_a_remove_and_an_add() {
    let fed = Fed::new(&[("a.md", &note("A", "", "# A\n"))]);

    fs::rename(fed.path("a.md"), fed.path("z.md")).unwrap();
    fed.feed.changed(["a.md"]).unwrap();
    fed.feed.changed(["z.md"]).unwrap();

    assert_eq!(
        fed.take(),
        [
            Change::Removed(note_path("a.md")),
            Change::Added(fed.info("z.md"))
        ]
    );
}

#[test]
fn a_deleted_file_is_removed() {
    let fed = Fed::new(&[("a.md", "# A\n"), ("b.md", "# B\n")]);

    fs::remove_file(fed.path("a.md")).unwrap();
    fed.feed.changed(["a.md"]).unwrap();

    assert_eq!(fed.take(), [Change::Removed(note_path("a.md"))]);
    assert_eq!(previews(&fed.core), ["B"]);
}

#[test]
fn a_moved_note_keeps_its_id_and_its_backlinks() {
    let fed = Fed::new(&[
        ("a.md", &note("A", "", "# A\n")),
        ("b.md", &note("B", "", "[a](holm:A)\n")),
    ]);

    fs::create_dir(fed.path("work")).unwrap();
    fs::rename(fed.path("a.md"), fed.path("work/a.md")).unwrap();
    fed.feed.changed(["a.md", "work"]).unwrap();

    let changes = fed.take();
    assert_eq!(
        changes[0],
        Change::Moved {
            from: note_path("a.md"),
            note: fed.info("work/a.md"),
        }
    );
    assert_eq!(changes[1..], [Change::Folders]);
    let backlinks = fed.core.snapshot().backlinks(&note_path("work/a.md"));
    assert_eq!(backlinks[0].path, note_path("b.md"));
}

#[test]
fn a_move_with_the_new_path_first_is_one_change() {
    let fed = Fed::new(&[("a.md", &note("A", "", "# A\n"))]);

    fs::rename(fed.path("a.md"), fed.path("z.md")).unwrap();
    // In two batches, the core sees the note at the new path before the old path is gone.
    fed.feed.changed(["z.md"]).unwrap();
    fed.feed.changed(["a.md"]).unwrap();

    assert_eq!(
        fed.take(),
        [Change::Moved {
            from: note_path("a.md"),
            note: fed.info("z.md"),
        }]
    );
}

#[test]
fn holm_s_own_saves_and_new_notes_come_only_as_notes() {
    let fed = Fed::new(&[("a.md", "# A\n")]);

    fed.core.save(&note_path("a.md"), "# A\n\nMore.\n").unwrap();
    fed.core.add_tag(&note_path("a.md"), "work").unwrap();
    let new = fed.core.create(&folder_path(""), "New").unwrap();
    fed.feed
        .changed([PathBuf::from("a.md"), common::path_buf(new.info.path)])
        .unwrap();

    assert_eq!(fed.take(), [Change::Notes, Change::Notes, Change::Notes]);
}

#[test]
fn a_pin_comes_only_as_notes_and_an_attachment_not_at_all() {
    let fed = Fed::new(&[("a.md", "# A\n")]);

    fed.core.set_pinned(&note_path("a.md"), true).unwrap();
    let image = fed
        .core
        .paste_image(&note_path("a.md"), "png", b"png")
        .unwrap();
    fed.feed
        .changed([PathBuf::from("a.md"), PathBuf::from("attachments"), image])
        .unwrap();

    assert_eq!(fed.take(), [Change::Notes]);
}

#[test]
fn each_own_move_comes_as_one_list_with_its_revision() {
    let fed = Fed::new(&[
        ("a.md", "---\ntags: [work/holm]\n---\n# A\n"),
        ("area/b.md", "# B\n"),
    ]);

    let note = fed.core.rename_note(&note_path("a.md"), "C").unwrap();
    let note_revision = fed.core.snapshot().revision();
    fed.core
        .rename_folder(&folder_path("area"), "zone")
        .unwrap();
    let folder_revision = fed.core.snapshot().revision();
    fed.core.rename_tag("work", " Job ").unwrap();
    fed.core.rename_tag("gone", "x").unwrap();
    fed.core
        .rename_folder(&folder_path("zone"), "zone")
        .unwrap();

    let lists: Vec<(u64, Vec<Change>)> = fed
        .lists()
        .into_iter()
        .map(|list| (list.snapshot.revision(), list.changes))
        .collect();
    assert_eq!(
        lists,
        [
            (
                note_revision,
                vec![Change::Moved {
                    from: note_path("a.md"),
                    note,
                }],
            ),
            (
                folder_revision,
                vec![Change::FolderMoved {
                    from: folder_path("area"),
                    to: folder_path("zone"),
                }],
            ),
            (
                fed.core.snapshot().revision(),
                vec![
                    Change::Notes,
                    Change::TagMoved {
                        from: "work".into(),
                        to: "Job".into(),
                    },
                ],
            ),
        ]
    );
    fed.feed.changed(["C.md", "zone"]).unwrap();
    assert_eq!(fed.take(), []);
}

#[test]
fn each_list_carries_the_snapshot_of_its_revision() {
    let fed = Fed::new(&[("a.md", "# A\n")]);

    fed.core.rename_note(&note_path("a.md"), "B").unwrap();
    fed.core.create_folder(&folder_path(""), "work").unwrap();
    fed.write("c.md", "# C\n");
    fed.feed.changed(["c.md"]).unwrap();

    let shown: Vec<(Vec<PathBuf>, Vec<PathBuf>)> = fed
        .lists()
        .iter()
        .map(|list| {
            let notes = list.snapshot.notes(NoteSet::All, Order::Title);
            let folders = list.snapshot.folders().map(common::path_buf).collect();
            let notes = notes.into_iter().map(|info| common::path_buf(info.path));
            (notes.collect(), folders)
        })
        .collect();
    let notes = |paths: &[&str]| paths.iter().map(PathBuf::from).collect::<Vec<_>>();
    assert_eq!(
        shown,
        [
            (notes(&["B.md"]), vec![]),
            (notes(&["B.md"]), notes(&["work"])),
            (notes(&["B.md", "c.md"]), notes(&["work"])),
        ]
    );
}

#[test]
fn a_move_rule_follows_paths_below_a_folder_and_child_tags() {
    let folder = Change::FolderMoved {
        from: folder_path("work"),
        to: folder_path("job"),
    };
    let after = |change: &Change, path: &str| change.folder_after(&folder_path(path));
    assert_eq!(after(&folder, "work"), Some(folder_path("job")));
    assert_eq!(after(&folder, "work/deep"), Some(folder_path("job/deep")));
    assert_eq!(
        folder.note_after(&note_path("work/deep/a.md")),
        Some(note_path("job/deep/a.md"))
    );
    assert_eq!(after(&folder, "workshop"), None);
    assert_eq!(folder.tag_after("work"), None);

    let moved = Change::Moved {
        from: note_path("a.md"),
        note: NoteInfo {
            path: note_path("b.md"),
            id: None,
            title: "b".into(),
            preview: String::new(),
            tags: Vec::new(),
            pinned: false,
            modified: std::time::SystemTime::UNIX_EPOCH,
        },
    };
    assert_eq!(
        moved.note_after(&note_path("a.md")),
        Some(note_path("b.md"))
    );
    assert_eq!(moved.note_after(&note_path("c.md")), None);
    assert_eq!(moved.folder_after(&folder_path("a")), None);

    let tag = Change::TagMoved {
        from: "work".into(),
        to: "home/job".into(),
    };
    assert_eq!(tag.tag_after("work"), Some("home/job".into()));
    assert_eq!(tag.tag_after("work/holm"), Some("home/job/holm".into()));
    assert_eq!(tag.tag_after("workshop"), None);
    assert_eq!(after(&tag, "work"), None);
}

#[test]
fn a_new_folder_adds_its_notes_and_changes_the_folder_list() {
    let fed = Fed::new(&[]);

    fed.write("work/a.md", "# A\n");
    fed.write("work/deep/b.md", "# B\n");
    fed.feed.changed(["work"]).unwrap();

    assert_eq!(
        fed.take(),
        [
            Change::Added(fed.info("work/a.md")),
            Change::Added(fed.info("work/deep/b.md")),
            Change::Folders,
        ]
    );
    assert_eq!(
        fed.core
            .snapshot()
            .folders()
            .map(common::path_buf)
            .collect::<Vec<_>>(),
        [PathBuf::from("work"), PathBuf::from("work/deep")]
    );
}

#[test]
fn a_deleted_folder_removes_its_notes() {
    let fed = Fed::new(&[
        ("work/a.md", "# A\n"),
        ("work/deep/b.md", "# B\n"),
        ("c.md", "# C\n"),
    ]);

    fs::remove_dir_all(fed.path("work")).unwrap();
    // Each path inside it may come too. The folder covers them.
    fed.feed.changed(["work/deep/b.md"]).unwrap();
    let mut changes = fed.take();
    fed.feed.changed(["work"]).unwrap();
    changes.extend(fed.take());

    assert_eq!(
        changes,
        [
            Change::Removed(note_path("work/deep/b.md")),
            Change::Removed(note_path("work/a.md")),
            Change::Folders,
        ]
    );
    assert_eq!(previews(&fed.core), ["C"]);
}

#[test]
fn a_renamed_folder_keeps_search_links_and_file_links_of_its_notes() {
    let fed = Fed::new(&[
        (
            "work/a.md",
            &note("A", "tags: [food]\n", "# A\nPotatoes [c](holm:C)\n"),
        ),
        ("work/pic.md", &note("P", "", "# Pic\n![p](pic.png)\n")),
        ("work/plain.md", "# Plain\nCarrots\n"),
        ("b.md", &note("B", "", "[a](holm:A)\n")),
        ("c.md", &note("C", "", "# C\n")),
    ]);
    let links = fed.core.snapshot().links(&note_path("work/a.md")).to_vec();
    fed.take();

    fs::rename(fed.path("work"), fed.path("job")).unwrap();
    fed.feed.changed(["work", "job"]).unwrap();

    assert_eq!(
        fed.take(),
        [
            Change::Moved {
                from: note_path("work/a.md"),
                note: fed.info("job/a.md"),
            },
            Change::Moved {
                from: note_path("work/pic.md"),
                note: fed.info("job/pic.md"),
            },
            Change::Added(fed.info("job/plain.md")),
            Change::Removed(note_path("work/plain.md")),
            Change::Folders,
        ]
    );
    let snapshot = fed.core.snapshot();
    let found = |text| fed.core.search(text).unwrap();
    assert_eq!(found("potatoes"), [note_path("job/a.md")]);
    assert_eq!(found("carrots"), [note_path("job/plain.md")]);
    assert_eq!(found("food"), [note_path("job/a.md")]);
    assert_eq!(snapshot.links(&note_path("job/a.md")), links);
    let back = |path: &NotePath| -> Vec<NotePath> {
        let notes = snapshot.backlinks(path);
        notes.into_iter().map(|n| n.path).collect()
    };
    assert_eq!(back(&note_path("job/a.md")), [note_path("b.md")]);
    assert_eq!(back(&note_path("c.md")), [note_path("job/a.md")]);
    assert_eq!(
        snapshot.links(&note_path("job/pic.md"))[0].target,
        LinkTarget::File("job/pic.png".into())
    );
    assert_eq!(
        snapshot.notes(NoteSet::WithTag("food"), Order::Title)[0].path,
        note_path("job/a.md")
    );
}

#[test]
fn a_tag_change_sends_a_tags_change() {
    let fed = Fed::new(&[("a.md", &note("A", "", "# A\n"))]);

    fed.write("a.md", &note("A", "tags: [work]\n", "# A\n"));
    fed.feed.changed(["a.md"]).unwrap();
    assert_eq!(
        fed.take(),
        [Change::Changed(fed.info("a.md")), Change::Tags]
    );

    fed.write("a.md", &note("A", "tags: [work]\n", "# A\n\nText.\n"));
    fed.feed.changed(["a.md"]).unwrap();
    assert_eq!(fed.take(), [Change::Changed(fed.info("a.md"))]);

    fs::remove_file(fed.path("a.md")).unwrap();
    fed.feed.changed(["a.md"]).unwrap();
    assert_eq!(
        fed.take(),
        [Change::Removed(note_path("a.md")), Change::Tags]
    );
    assert_eq!(fed.core.snapshot().tags(), []);
}

#[test]
fn hidden_paths_and_other_files_send_nothing() {
    let fed = Fed::new(&[]);

    fed.write(".git/HEAD", "ref\n");
    fed.write(".hidden.md", "# Hidden\n");
    fed.write("picture.png", "png");
    fed.feed.changed([".git/HEAD"]).unwrap();
    fed.feed.changed([".hidden.md"]).unwrap();
    fed.feed.changed(["picture.png"]).unwrap();

    assert_eq!(fed.take(), []);
    assert_eq!(previews(&fed.core), Vec::<String>::new());
}

#[test]
fn the_empty_path_checks_the_whole_notes_folder() {
    let fed = Fed::new(&[("a.md", "# A\n"), ("b.md", "# B\n")]);

    fed.write("a.md", "# A2\n");
    fs::remove_file(fed.path("b.md")).unwrap();
    fed.write("work/c.md", "# C\n");
    fed.feed.changed([""]).unwrap();

    assert_eq!(
        fed.take(),
        [
            Change::Changed(fed.info("a.md")),
            Change::Removed(note_path("b.md")),
            Change::Added(fed.info("work/c.md")),
            Change::Folders,
        ]
    );
}

#[test]
fn a_copied_note_gets_a_new_id() {
    let fed = Fed::new(&[("a.md", &note("A", "", "# A\n"))]);

    fs::copy(fed.path("a.md"), fed.path("copy.md")).unwrap();
    fed.feed.changed(["copy.md"]).unwrap();

    let copy = fed.info("copy.md");
    assert_ne!(copy.id.as_ref().map(NoteId::as_str), Some("A"));
    assert_eq!(fed.take(), [Change::Added(copy)]);
    // The new id went into the file. Its event finds nothing new.
    fed.feed.changed(["copy.md"]).unwrap();
    assert_eq!(fed.take(), []);
}

#[test]
fn every_stream_gets_each_change() {
    let fed = Fed::new(&[]);
    let mut second = fed.core.changes();
    drop(fed.core.changes());

    fed.write("a.md", "# A\n");
    fed.feed.changed(["a.md"]).unwrap();

    let added = Change::Added(fed.info("a.md"));
    assert_eq!(fed.take(), std::slice::from_ref(&added));
    let list = second.try_next().unwrap();
    assert_eq!(list.changes, [added]);
}

#[test]
fn the_feed_refuses_paths_outside_the_notes_folder() {
    let fed = Fed::new(&[]);

    for path in ["/etc/passwd", "../a.md", "work/../a.md"] {
        assert!(matches!(
            fed.feed.changed(["a.md", path]),
            Err(Error::BadPath { .. })
        ));
    }
}

#[test]
fn a_stream_ends_when_the_core_is_gone() {
    let fed = Fed::new(&[]);
    let mut changes = fed.core.changes();

    drop(fed);

    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    assert_eq!(
        Pin::new(&mut changes)
            .poll_next(&mut cx)
            .map(|list| list.is_none()),
        Poll::Ready(true)
    );
}

mod real_events {
    //! These tests use real `notify` events on a temp folder.

    use super::*;

    /// Long enough for a slow CI machine. A passing test does not wait this long.
    const WAIT: Duration = Duration::from_secs(10);

    fn open(files: &[(&str, &str)]) -> (TempDir, TempDir, Core, Changes) {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            write(&dir.path().join(path), text);
        }
        let cache = tempfile::tempdir().unwrap();
        let core = Core::open(dir.path(), cache.path()).unwrap();
        let changes = core.changes();
        (dir, cache, core, changes)
    }

    /// Waits up to [`WAIT`] for the next change list, and gives its changes.
    fn next(changes: &mut Changes) -> Vec<Change> {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(list) = changes.try_next() {
                return list.changes;
            }
            assert!(Instant::now() < deadline, "no change list came");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn an_outside_edit_comes_as_a_change() {
        let (dir, _cache, core, mut changes) = open(&[("a.md", "# Old\n")]);

        fs::write(dir.path().join("a.md"), "# New\n").unwrap();

        assert_eq!(
            next(&mut changes),
            [Change::Changed(core.read(&note_path("a.md")).unwrap().info)]
        );
    }

    #[test]
    fn a_save_comes_only_as_notes_but_the_next_outside_change_comes() {
        let (dir, _cache, core, mut changes) = open(&[("a.md", "# A\n")]);

        core.save(&note_path("a.md"), "# A\n\nMore.\n").unwrap();
        fs::write(dir.path().join("b.md"), "# B\n").unwrap();

        assert_eq!(next(&mut changes), [Change::Notes]);
        assert_eq!(
            next(&mut changes),
            [Change::Added(core.read(&note_path("b.md")).unwrap().info)]
        );
    }

    #[test]
    fn a_rename_comes_as_a_move() {
        let (dir, _cache, core, mut changes) = open(&[("a.md", &note("A", "", "# A\n"))]);

        fs::rename(dir.path().join("a.md"), dir.path().join("b.md")).unwrap();

        assert_eq!(
            next(&mut changes),
            [Change::Moved {
                from: note_path("a.md"),
                note: core.read(&note_path("b.md")).unwrap().info,
            }]
        );
    }

    #[test]
    fn walks_and_reads_do_not_raise_the_revision() {
        let (dir, cache, core, mut changes) = open(&[("a.md", &note("A", "", "# A\n"))]);
        // A folder that moves in comes as one event, so the core walks it in one batch.
        write(&cache.path().join("work/b.md"), &note("B", "", "# B\n"));
        fs::rename(cache.path().join("work"), dir.path().join("work")).unwrap();
        while !next(&mut changes).contains(&Change::Folders) {}
        std::thread::sleep(Duration::from_millis(300));
        let start = core.snapshot().revision();

        core.read(&note_path("work/b.md")).unwrap();
        core.snapshot().notes(NoteSet::All, Order::Title);
        fs::read(dir.path().join("a.md")).unwrap();
        std::thread::sleep(Duration::from_secs(1));

        assert_eq!(core.snapshot().revision(), start);
    }
}

#[test]
fn the_revision_grows_with_own_writes_and_outside_changes() {
    let fed = Fed::new(&[("a.md", "# A\n")]);
    let start = fed.core.snapshot().revision();

    fed.core.read(&note_path("a.md")).unwrap();
    fed.core.snapshot().notes(NoteSet::All, Order::Title);
    assert_eq!(
        fed.core.snapshot().revision(),
        start,
        "reads change nothing"
    );

    fed.core.create(&folder_path(""), "New").unwrap();
    let created = fed.core.snapshot().revision();
    assert!(created > start);

    fed.write("a.md", "# A again\n");
    fed.feed.changed(["a.md"]).unwrap();
    assert!(fed.core.snapshot().revision() > created);
}

fn paths(core: &Core) -> Vec<String> {
    let mut paths = common::paths(core.snapshot().paths(NoteSet::All, Order::Title));
    paths.sort();
    paths
}

#[test]
fn a_missing_notes_folder_keeps_the_notes() {
    let fed = Fed::new(&[("a.md", "# A\n"), ("w/b.md", "# B\n")]);
    let start = fed.core.snapshot().revision();
    let away = fed.dir.path().with_extension("away");

    fs::rename(fed.dir.path(), &away).unwrap();
    let gone = fed.feed.changed([""]);

    assert!(matches!(gone, Err(Error::Read { .. })), "{gone:?}");
    assert_eq!(paths(&fed.core), ["a.md", "w/b.md"]);
    assert_eq!(fed.core.snapshot().revision(), start);
    assert!(fed.lists().is_empty());

    fs::rename(&away, fed.dir.path()).unwrap();
    fed.feed.changed([""]).unwrap();

    assert_eq!(paths(&fed.core), ["a.md", "w/b.md"]);
    assert_eq!(fed.core.snapshot().revision(), start);
}

#[test]
fn a_gone_folder_that_the_snapshot_does_not_list_loses_its_notes() {
    let fed = Fed::new(&[("a.md", "# A\n"), ("attachments/n.md", "# N\n")]);

    fs::remove_dir_all(fed.path("attachments")).unwrap();
    fed.feed.changed(["attachments"]).unwrap();

    assert_eq!(paths(&fed.core), ["a.md"]);
}

#[test]
fn a_folder_can_become_a_note() {
    let fed = Fed::new(&[("x.md/a.md", "# A\n")]);

    fs::remove_dir_all(fed.path("x.md")).unwrap();
    fed.write("x.md", "# X\n");
    fed.feed.changed(["x.md"]).unwrap();

    assert_eq!(paths(&fed.core), ["x.md"]);
    assert_eq!(fed.core.snapshot().folders().count(), 0);
}

#[test]
fn a_note_can_become_a_folder() {
    let fed = Fed::new(&[("x.md", "# X\n")]);

    fs::remove_file(fed.path("x.md")).unwrap();
    fed.write("x.md/a.md", "# A\n");
    fed.feed.changed(["x.md"]).unwrap();

    assert_eq!(paths(&fed.core), ["x.md/a.md"]);
    assert_eq!(common::paths(fed.core.snapshot().folders()), ["x.md"]);
}
