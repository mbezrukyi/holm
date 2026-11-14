//! Link targets through `Core` on a real temp folder: notes by id, headings, broken links and
//! file link paths.

#![allow(clippy::unwrap_used, reason = "tests")]

mod common;

use std::path::{Path, PathBuf};

use common::Folder;
use holm_core::Error;
use holm_core::model::LinkTarget;

fn note(id: &str, body: &str) -> String {
    format!("---\nid: {id}\n---\n{body}")
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
