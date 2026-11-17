//! Link targets through `Core` on a real temp folder: notes by id, headings, broken links and
//! file link paths.

#![allow(clippy::unwrap_used, reason = "tests")]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::Folder;
use holm_core::Error;
use holm_core::model::{Link, LinkTarget};

fn note(id: &str, body: &str) -> String {
    format!("---\nid: {id}\n---\n{body}")
}

/// The links of `path` in the snapshot. It checks that the index rows and a new index give
/// the same links.
fn links_everywhere(folder: &mut Folder, path: &str) -> Vec<Link> {
    let links = folder.core.snapshot().links(path).to_vec();
    folder.reopen();
    assert_eq!(folder.core.snapshot().links(path), links, "index rows");
    fs::remove_file(folder.index_file()).unwrap();
    folder.reopen();
    assert_eq!(folder.core.snapshot().links(path), links, "new index");
    links
}

fn file(path: &str) -> Link {
    Link {
        target: LinkTarget::File(PathBuf::from(path)),
        embedded: false,
    }
}

#[test]
fn a_tag_change_keeps_the_links() {
    let mut folder = Folder::new(&[("work/a.md", &note("01A", "[f](x.pdf) [n](holm:01B)\n"))]);
    let before = folder.core.snapshot().links("work/a.md").to_vec();

    folder.core.add_tag("work/a.md", "new").unwrap();

    assert_eq!(links_everywhere(&mut folder, "work/a.md"), before);
    assert_eq!(before[0], file("work/x.pdf"));
}

#[test]
fn a_save_follows_the_links_of_the_text() {
    let mut folder = Folder::new(&[("work/a.md", &note("01A", "[f](x.pdf)\n"))]);

    folder
        .core
        .save("work/a.md", "More words. [f](x.pdf)\n")
        .unwrap();
    assert_eq!(
        links_everywhere(&mut folder, "work/a.md"),
        [file("work/x.pdf")]
    );

    folder
        .core
        .save("work/a.md", "More words. [f](y.pdf)\n")
        .unwrap();
    assert_eq!(
        links_everywhere(&mut folder, "work/a.md"),
        [file("work/y.pdf")]
    );
}

#[test]
fn a_folder_move_points_relative_file_links_to_the_new_place() {
    let mut folder = Folder::new(&[
        ("work/a.md", &note("01A", "[b](sub/b.md)\n")),
        ("work/sub/b.md", ""),
        ("archive/x.md", ""),
    ]);

    folder.core.move_folder("work", "archive").unwrap();

    assert_eq!(
        folder.text("archive/work/a.md"),
        note("01A", "[b](sub/b.md)\n")
    );
    assert_eq!(
        links_everywhere(&mut folder, "archive/work/a.md"),
        [file("archive/work/sub/b.md")]
    );
}

#[test]
fn note_with_id_finds_the_note_or_none() {
    let folder = Folder::new(&[("work/a.md", &note("01A", "# A\n")), ("b.md", "# B\n")]);

    let found = folder.core.snapshot().note_with_id("01A").cloned().unwrap();

    assert_eq!(found.path, Path::new("work/a.md"));
    assert_eq!(folder.core.snapshot().note_with_id("01X").cloned(), None);
    assert_eq!(folder.core.snapshot().note_with_id("").cloned(), None);
}

#[test]
fn headings_list_every_heading_also_the_first() {
    let body = "# Plans\n\n## Bug list\n\ntext\n\n### Later *ideas*\n\n# Second\n";
    let folder = Folder::new(&[("a.md", body)]);

    assert_eq!(
        folder.core.headings("a.md").unwrap(),
        ["Plans", "Bug list", "Later ideas", "Second"]
    );
    assert!(matches!(
        folder.core.headings("gone.md"),
        Err(Error::Read { .. })
    ));
}

#[test]
fn broken_links_name_missing_notes_and_headings_once() {
    let body = "\
[a](holm:01T) [b](<holm:01T#Bug list>) [c](<holm:01T#bug list>) [d](<holm:01T#Plans>)
[e](holm:01X) [f](holm:01X) [g](<#Nowhere>) [h](gone.pdf)
";
    let folder = Folder::new(&[
        ("a.md", body),
        ("t.md", &note("01T", "# Plans\n\n## Bug list\n")),
    ]);
    let broken = folder.core.broken_links("a.md").unwrap();

    let shown: Vec<(String, Option<String>)> = broken
        .iter()
        .map(|target| match target {
            LinkTarget::Note { id, heading } => (id.to_string(), heading.clone()),
            other => panic!("not a note link: {other:?}"),
        })
        .collect();
    assert_eq!(
        shown,
        [
            ("01T".to_owned(), Some("bug list".to_owned())),
            ("01X".to_owned(), None),
        ]
    );
}

#[test]
fn file_link_is_relative_inside_and_absolute_outside() {
    let folder = Folder::new(&[("work/a.md", "")]);
    folder.write("docs/plan.pdf", "pdf");
    let outside = tempfile::tempdir().unwrap();
    let away = outside.path().join("x.pdf");
    std::fs::write(&away, "pdf").unwrap();

    let inside = folder
        .core
        .file_link("work/a.md", folder.path("docs/plan.pdf"))
        .unwrap();
    let out = folder.core.file_link("a.md", &away).unwrap();

    assert_eq!(inside, PathBuf::from("../docs/plan.pdf"));
    assert_eq!(out, std::fs::canonicalize(&away).unwrap());
    assert!(matches!(
        folder.core.file_link("a.md", "x.pdf"),
        Err(Error::BadPath { .. })
    ));
}

#[test]
fn paste_image_keeps_the_format() {
    let folder = Folder::new(&[("a.md", "")]);

    let path = folder.core.paste_image("a.md", "jpg", b"jpg").unwrap();

    assert!(path.starts_with("attachments"));
    assert_eq!(path.extension().unwrap(), "jpg");
}
