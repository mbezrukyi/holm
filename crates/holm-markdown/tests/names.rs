//! Tests for tag names and file names.

use holm_markdown::{file_stem, normalize_tag};

#[test]
fn tag_names() {
    let cases = [
        ("work", Some("work")),
        ("two words", Some("two-words")),
        ("  Work  ", Some("Work")),
        ("work/holm", Some("work/holm")),
        ("/work/holm/", Some("work/holm")),
        ("a//b", Some("a/b")),
        ("a / b c", Some("a/b-c")),
        ("", None),
        (" / / ", None),
    ];
    for (input, want) in cases {
        assert_eq!(normalize_tag(input).as_deref(), want, "input {input:?}");
    }
}

#[test]
fn file_stems() {
    assert_eq!(file_stem("Plans for 2026"), "Plans for 2026");
    assert_eq!(file_stem("a/b/c"), "a-b-c");
    assert_eq!(file_stem("..dots"), "-.dots");
    assert_eq!(file_stem("Ünïcode: ok?"), "Ünïcode: ok?");
}

#[test]
fn long_file_stem_stops_at_a_char() {
    let text = "é".repeat(200);
    let stem = file_stem(&text);
    assert_eq!(stem.len(), 240);
    assert!(text.starts_with(&stem));
}
