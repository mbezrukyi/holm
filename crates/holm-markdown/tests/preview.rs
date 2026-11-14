//! The preview line of a note.

use holm_markdown::Note;

fn preview(text: &str) -> String {
    Note::parse(text).preview()
}

#[test]
fn the_preview_is_the_first_line_of_text() {
    assert_eq!(preview("\nFirst *line*.\nSecond.\n"), "First line.");
    assert_eq!(preview("---\nid: 01A\n---\n> `code` here\n"), "code here");
    assert_eq!(preview("## Smaller\ntext\n"), "Smaller");
}

#[test]
fn a_heading_is_only_text() {
    assert_eq!(preview("# Spring plans\n\nFirst line.\n"), "Spring plans");
}

#[test]
fn lines_with_no_text_are_skipped() {
    assert_eq!(preview("\n---\n```rust\nlet x = 1;\n"), "let x = 1;");
    assert_eq!(preview("#\n"), "");
    assert_eq!(preview(""), "");
}

#[test]
fn a_long_line_is_cut_at_a_char() {
    let line = "é".repeat(300);
    assert_eq!(preview(&line).chars().count(), 200);
}
