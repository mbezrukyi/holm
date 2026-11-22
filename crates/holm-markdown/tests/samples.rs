use std::fmt::Write as _;
use std::fs;

use holm_markdown::NoteFile;

/// So the snapshot shows the line endings.
fn visible(text: &str) -> String {
    text.replace('\r', "␍")
}

fn report(text: &str) -> String {
    let note = NoteFile::parse(text);
    let frontmatter = &text[..text.len() - note.body().len()];

    let mut edited = note.clone();
    edited.set_id(Some("01JC3X0000NEW"));
    edited.set_tags(&["work/holm", "new tag"]);
    edited.set_pinned(true);

    let mut cleared = note.clone();
    cleared.set_id(None);
    cleared.set_tags::<&str>(&[]);
    cleared.set_pinned(false);

    let mut out = String::new();
    let _ = writeln!(out, "line ending: {:?}", note.line_ending());
    let _ = writeln!(out, "id: {:?}", note.id());
    let _ = writeln!(out, "tags: {:?}", note.tags());
    let _ = writeln!(out, "pinned: {}", note.pinned());
    let _ = writeln!(out, "preview: {:?}", note.preview());
    let _ = writeln!(out, "\n--- frontmatter ---\n{}", visible(frontmatter));
    let _ = writeln!(out, "--- body ---\n{}", visible(note.body()));
    let _ = writeln!(out, "--- set all ---\n{}", visible(&edited.to_string()));
    let _ = writeln!(out, "--- remove all ---\n{}", visible(&cleared.to_string()));
    out
}

#[test]
fn samples() {
    insta::glob!("samples/*.md", |path| {
        let text = fs::read_to_string(path).unwrap();
        assert_eq!(NoteFile::parse(&text).to_string(), text);
        insta::assert_snapshot!(report(&text));
    });
}
