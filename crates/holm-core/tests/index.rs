//! Lists, tags, search and links through `Core` on a real temp folder and a real index.

#![allow(clippy::unwrap_used, reason = "tests")]

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use common::Folder;
use holm_core::Core;
use holm_core::model::{Link, LinkTarget, NoteInfo, NoteSet, Order, Tag};

fn paths(notes: Vec<NoteInfo>) -> Vec<String> {
    common::paths(notes.into_iter().map(|n| n.path))
}

/// The search results in the order of a list.
fn found(folder: &Folder, text: &str) -> Vec<String> {
    let found = folder.core.search(text).unwrap();
    common::paths(folder.core.snapshot().sorted(found, Order::Title))
}

fn note(id: &str, extra: &str, body: &str) -> String {
    format!("---\nid: {id}\n{extra}---\n{body}")
}

fn tag(name: &str, notes: usize, children: Vec<Tag>) -> Tag {
    Tag {
        name: name.to_owned(),
        notes,
        children,
    }
}

#[test]
fn notes_come_last_change_first_with_pinned_notes_on_top() {
    let folder = Folder::new(&[
        ("old.md", "# Old\n"),
        ("new.md", "# New\n"),
        ("pinned.md", "---\npinned: true\n---\n# Pinned\n"),
        ("work/mid.md", "# Mid\n"),
    ]);
    folder.set_time("old.md", 100);
    folder.set_time("new.md", 300);
    folder.set_time("pinned.md", 50);
    folder.set_time("work/mid.md", 200);
    let mut folder = folder;
    folder.reopen();

    assert_eq!(
        paths(
            folder
                .core
                .snapshot()
                .notes(NoteSet::All, Order::LastChange)
        ),
        ["pinned.md", "new.md", "work/mid.md", "old.md"]
    );
    assert_eq!(
        paths(folder.core.snapshot().notes(NoteSet::All, Order::Title)),
        ["pinned.md", "work/mid.md", "new.md", "old.md"]
    );
}

#[test]
fn title_order_ignores_capital_letters() {
    let folder = Folder::new(&[("banana.md", ""), ("Apple.md", ""), ("cherry.md", "")]);

    let titles: Vec<String> = folder
        .core
        .snapshot()
        .notes(NoteSet::All, Order::Title)
        .into_iter()
        .map(|n| n.title)
        .collect();

    assert_eq!(titles, ["Apple", "banana", "cherry"]);
}

#[test]
fn a_folder_lists_only_its_own_notes() {
    let folder = Folder::new(&[
        ("root.md", ""),
        ("work/a.md", ""),
        ("work/b.md", ""),
        ("work/holm/c.md", ""),
    ]);

    let mut work = paths(
        folder
            .core
            .snapshot()
            .notes(NoteSet::In(Path::new("work")), Order::Title),
    );
    work.sort();

    assert_eq!(work, ["work/a.md", "work/b.md"]);
    assert_eq!(
        paths(
            folder
                .core
                .snapshot()
                .notes(NoteSet::In(Path::new("")), Order::Title)
        ),
        ["root.md"]
    );
    assert_eq!(
        folder
            .core
            .snapshot()
            .notes(NoteSet::In(Path::new("missing")), Order::Title),
        Vec::<NoteInfo>::new()
    );
    assert_eq!(
        folder
            .core
            .snapshot()
            .notes(NoteSet::In(Path::new("../x")), Order::Title),
        Vec::<NoteInfo>::new()
    );
}

#[test]
fn notes_keep_their_fields_in_the_index() {
    let text = "---\nid: 01A\ntags: [work/holm, home]\npinned: true\n---\n\n- Buy *milk*\n";
    let folder = Folder::new(&[("a.md", text)]);

    let listed = folder
        .core
        .snapshot()
        .notes(NoteSet::All, Order::LastChange);

    assert_eq!(listed, [folder.core.read("a.md").unwrap().info]);
    assert_eq!(listed[0].tags, ["work/holm", "home"]);
    assert_eq!(listed[0].preview, "Buy milk");
}

#[test]
fn tags_form_a_tree_and_parents_count_the_notes_of_children() {
    let folder = Folder::new(&[
        ("a.md", "---\ntags: [work/holm, work/holm/ui]\n---\n"),
        ("b.md", "---\ntags: [work]\n---\n"),
        ("c.md", "---\ntags: [Home, work/other]\n---\n"),
        ("d.md", "---\ntags: [art/paint]\n---\n"),
        ("e.md", ""),
    ]);

    assert_eq!(
        folder.core.snapshot().tags(),
        [
            tag("art", 1, vec![tag("art/paint", 1, vec![])]),
            tag("Home", 1, vec![]),
            tag(
                "work",
                3,
                vec![
                    tag("work/holm", 1, vec![tag("work/holm/ui", 1, vec![])]),
                    tag("work/other", 1, vec![]),
                ]
            ),
        ]
    );
}

#[test]
fn a_tag_lists_the_notes_of_its_child_tags_too() {
    let folder = Folder::new(&[
        ("a.md", "---\ntags: [work/holm]\n---\n"),
        ("b.md", "---\ntags: [work]\n---\n"),
        ("c.md", "---\ntags: [workshop]\n---\n"),
        ("d.md", "---\ntags: [Work]\n---\n"),
    ]);

    let mut work = paths(
        folder
            .core
            .snapshot()
            .notes(NoteSet::WithTag("work"), Order::Title),
    );
    work.sort();

    assert_eq!(work, ["a.md", "b.md"]);
    assert_eq!(
        paths(
            folder
                .core
                .snapshot()
                .notes(NoteSet::WithTag("work/holm"), Order::Title)
        ),
        ["a.md"]
    );
    assert_eq!(
        folder
            .core
            .snapshot()
            .notes(NoteSet::WithTag("wor"), Order::Title),
        Vec::<NoteInfo>::new()
    );
}

#[test]
fn search_finds_words_in_titles_text_and_tags() {
    let folder = Folder::new(&[
        ("a.md", "# Garden plans\nSome text.\n"),
        ("b.md", "# Other\nThe café opens at nine.\n"),
        ("c.md", "---\ntags: [recipes/bread]\n---\n# Bake\n"),
        ("d.md", "# Nothing here\n"),
    ]);
    let search = |text| found(&folder, text);

    assert_eq!(search("garden"), ["a.md"]);
    assert_eq!(search("PLA"), ["a.md"], "a word matches as a prefix");
    assert_eq!(search("cafe"), ["b.md"], "accents make no difference");
    assert_eq!(search("bread"), ["c.md"]);
    assert_eq!(search("garden text"), ["a.md"]);
    assert_eq!(
        search("bake bread"),
        ["c.md"],
        "words match the title and a tag"
    );
    assert_eq!(
        search("garden nine"),
        Vec::<String>::new(),
        "every word must match"
    );
    assert_eq!(search(""), Vec::<String>::new());
    assert_eq!(search("  \"* ( "), Vec::<String>::new());
    assert_eq!(search("\"garden"), ["a.md"], "quotes are plain text");
}

#[test]
fn search_follows_a_tag_change_and_keeps_the_text() {
    let folder = Folder::new(&[("a.md", "---\ntags: [work]\n---\n# Garden\n")]);

    folder.core.rename_tag("work", "home").unwrap();

    assert_eq!(found(&folder, "work"), Vec::<String>::new());
    assert_eq!(found(&folder, "home garden"), ["a.md"]);
}

#[test]
fn links_give_every_link_of_a_note_in_order() {
    let body = "\
[n](holm:01B) [h](<holm:01B#Bug list>) [s](<#Top>)
![img](attachments/a.png) [up](../x.pdf) [out](/etc/hosts) [web](https://example.com)
";
    let folder = Folder::new(&[("work/a.md", body)]);
    let link = |target, embedded| Link { target, embedded };
    let file = |path: &str| LinkTarget::File(PathBuf::from(path));

    let links = folder.core.snapshot().links("work/a.md").to_vec();

    let notes: Vec<(&str, Option<&str>)> = links[..2]
        .iter()
        .filter_map(|l| match &l.target {
            LinkTarget::Note { id, heading } => Some((id.as_str(), heading.as_deref())),
            _ => None,
        })
        .collect();
    assert_eq!(notes, [("01B", None), ("01B", Some("Bug list"))]);
    assert_eq!(
        links[2..],
        [
            link(LinkTarget::Heading("Top".to_owned()), false),
            link(file("work/attachments/a.png"), true),
            link(file("x.pdf"), false),
            link(file("/etc/hosts"), false),
            link(LinkTarget::Web("https://example.com".to_owned()), false),
        ]
    );
    assert_eq!(
        folder.core.snapshot().links("missing.md").to_vec(),
        Vec::<Link>::new()
    );
    assert_eq!(folder.core.snapshot().links("a.txt"), []);
}

#[test]
fn backlinks_come_from_id_links_and_file_links() {
    let folder = Folder::new(&[
        ("target.md", &note("01T", "", "[me](holm:01T)\n")),
        ("by-id.md", "[t](holm:01T)\n"),
        ("by-heading.md", "[t](<holm:01T#Part>)\n"),
        ("work/by-path.md", "[t](../target.md)\n"),
        ("other.md", "[x](holm:01X) [y](target.pdf)\n"),
    ]);

    let mut back = paths(folder.core.snapshot().backlinks("target.md", Order::Title));
    back.sort();

    assert_eq!(back, ["by-heading.md", "by-id.md", "work/by-path.md"]);
    assert_eq!(
        folder.core.snapshot().backlinks("other.md", Order::Title),
        Vec::<NoteInfo>::new()
    );
}

#[test]
fn save_and_create_update_the_index() {
    let folder = Folder::new(&[("a.md", "# A\n")]);
    folder.core.add_tag("a.md", "food").unwrap();

    let saved = folder.core.save("a.md", "# A\nPotatoes\n").unwrap();
    let created = folder.core.create("", "New").unwrap();

    assert_eq!(found(&folder, "potato"), ["a.md"]);
    assert_eq!(folder.core.snapshot().tags(), [tag("food", 1, vec![])]);
    let listed = folder.core.snapshot().notes(NoteSet::All, Order::Title);
    assert_eq!(listed, [saved.info, created.info]);
}

#[test]
fn open_picks_up_changes_made_while_holm_was_closed() {
    let mut folder = Folder::new(&[
        ("edited.md", "# Before\n"),
        ("deleted.md", ""),
        ("kept.md", "# Kept\n"),
    ]);
    folder.write("edited.md", "# After\nNew words\n");
    fs::remove_file(folder.path("deleted.md")).unwrap();
    folder.write("work/added.md", "---\ntags: [new]\n---\n");

    folder.reopen();

    let mut all = paths(folder.core.snapshot().notes(NoteSet::All, Order::Title));
    all.sort();
    assert_eq!(all, ["edited.md", "kept.md", "work/added.md"]);
    assert_eq!(found(&folder, "words"), ["edited.md"]);
    assert_eq!(folder.core.read("edited.md").unwrap().info.preview, "After");
    assert_eq!(folder.core.snapshot().tags(), [tag("new", 1, vec![])]);
}

#[test]
fn open_sees_a_change_that_keeps_the_size_and_the_time() {
    let mut folder = Folder::new(&[("a.md", "# Aaaa\n")]);
    let time = folder.set_time("a.md", 1_000);
    folder.reopen();
    folder.write("a.md", "# Bbbb\n");
    folder.set_time("a.md", 1_000);

    folder.reopen();

    let listed = folder.core.snapshot().notes(NoteSet::All, Order::Title);
    assert_eq!(listed[0].preview, "Bbbb");
    assert_eq!(listed[0].modified, time);
}

#[test]
fn a_copied_note_gets_a_new_id_and_the_known_note_keeps_its_id() {
    let mut folder = Folder::new(&[("b.md", &note("01SAME", "", "# B\n"))]);
    folder.write("a.md", &note("01SAME", "author: me\n", "# Copy\n"));
    let time = folder.set_time("a.md", 1_000);

    folder.reopen();

    let a = folder.core.read("a.md").unwrap();
    let b = folder.core.read("b.md").unwrap();
    assert_eq!(b.info.id.unwrap().as_str(), "01SAME");
    let new = a.info.id.unwrap();
    assert_ne!(new.as_str(), "01SAME");
    assert_eq!(
        folder.text("a.md"),
        note(new.as_str(), "author: me\n", "# Copy\n")
    );
    assert_eq!(a.info.modified, time, "the copy keeps its time");
    assert_eq!(
        folder
            .core
            .snapshot()
            .notes(NoteSet::All, Order::Title)
            .len(),
        2
    );
}

#[test]
fn the_first_of_two_new_notes_with_the_same_id_keeps_it() {
    let folder = Folder::new(&[
        ("a.md", &note("01SAME", "", "")),
        ("b.md", &note("01SAME", "", "")),
    ]);

    let a = folder.core.read("a.md").unwrap();
    let b = folder.core.read("b.md").unwrap();

    assert_eq!(a.info.id.unwrap().as_str(), "01SAME");
    assert_ne!(b.info.id.unwrap().as_str(), "01SAME");
}

#[test]
fn a_note_moved_while_holm_was_closed_keeps_its_id() {
    let mut folder = Folder::new(&[
        ("a.md", &note("01MOVE", "", "# A\n")),
        ("link.md", "[a](holm:01MOVE)\n"),
    ]);
    fs::create_dir(folder.path("work")).unwrap();
    fs::rename(folder.path("a.md"), folder.path("work/a.md")).unwrap();

    folder.reopen();

    assert_eq!(folder.text("work/a.md"), note("01MOVE", "", "# A\n"));
    assert_eq!(
        paths(folder.core.snapshot().backlinks("work/a.md", Order::Title)),
        ["link.md"]
    );
}

#[test]
fn saving_a_note_with_the_id_of_a_deleted_note_keeps_the_id() {
    let folder = Folder::new(&[("a.md", &note("01GONE", "", ""))]);
    fs::remove_file(folder.path("a.md")).unwrap();
    folder.write("b.md", &note("01GONE", "", ""));
    let saved = folder.core.save("b.md", "Text\n").unwrap();

    assert_eq!(saved.info.id.unwrap().as_str(), "01GONE");
    assert_eq!(
        paths(folder.core.snapshot().notes(NoteSet::All, Order::Title)),
        ["b.md"]
    );
}

#[test]
fn a_broken_index_is_rebuilt() {
    let folder = Folder::new(&[("a.md", "# A\n")]);
    let cache = tempfile::tempdir().unwrap();
    fs::write(cache.path().join("index.sqlite"), vec![b'x'; 8192]).unwrap();

    let core = Core::open(folder.dir.path(), cache.path()).unwrap();

    assert_eq!(
        paths(core.snapshot().notes(NoteSet::All, Order::Title)),
        ["a.md"]
    );
}

#[test]
fn an_index_with_another_shape_is_rebuilt() {
    let folder = Folder::new(&[("a.md", "# A\n")]);
    let cache = tempfile::tempdir().unwrap();
    drop(Core::open(folder.dir.path(), cache.path()).unwrap());
    let index = cache.path().join("index.sqlite");
    rusqlite::Connection::open(&index)
        .unwrap()
        .execute_batch("DELETE FROM notes; PRAGMA user_version = 999;")
        .unwrap();

    let core = Core::open(folder.dir.path(), cache.path()).unwrap();

    assert_eq!(
        paths(core.snapshot().notes(NoteSet::All, Order::Title)),
        ["a.md"]
    );
}

#[test]
fn the_index_lives_in_memory_when_its_folder_cannot_be_written() {
    let folder = Folder::new(&[("a.md", "old\n")]);
    let cache = tempfile::tempdir().unwrap();
    fs::set_permissions(cache.path(), fs::Permissions::from_mode(0o555)).unwrap();

    let core = Core::open(folder.dir.path(), cache.path()).unwrap();
    let saved = core.save("a.md", "new\n");

    fs::set_permissions(cache.path(), fs::Permissions::from_mode(0o755)).unwrap();
    saved.unwrap();
    let preview = core.snapshot().note("a.md").unwrap().preview.clone();
    assert_eq!(preview, "new");
    assert!(!cache.path().join("index.sqlite").exists());
}

#[test]
fn notes_in_attachments_and_odd_names_round_trip() {
    let folder = Folder::new(&[("attachments/a b.md", ""), ("ünï cödé.md", "")]);

    let mut all: Vec<PathBuf> = folder
        .core
        .snapshot()
        .notes(NoteSet::All, Order::Title)
        .into_iter()
        .map(|n| n.path)
        .collect();
    all.sort();

    assert_eq!(
        all,
        [
            PathBuf::from("attachments/a b.md"),
            PathBuf::from("ünï cödé.md")
        ]
    );
}
