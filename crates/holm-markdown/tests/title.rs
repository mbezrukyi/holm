//! The title range and the preview line of a note.

use holm_markdown::{Note, title_range};

fn preview(text: &str) -> String {
    Note::parse(text).preview()
}

#[test]
fn the_preview_is_the_first_text_after_the_title() {
    assert_eq!(
        preview("# Plans\n\nFirst *line*.\nSecond.\n"),
        "First line."
    );
    assert_eq!(
        preview("---\nid: 01A\n---\n# Plans\n> `code` here\n"),
        "code here"
    );
    assert_eq!(preview("No title.\n"), "No title.");
    assert_eq!(preview("## Smaller\ntext\n"), "Smaller");
}

#[test]
fn lines_with_no_text_are_skipped() {
    assert_eq!(
        preview("# Plans\n\n---\n```rust\nlet x = 1;\n"),
        "let x = 1;"
    );
    assert_eq!(preview("# Plans\n"), "");
    assert_eq!(preview(""), "");
}

#[test]
fn a_long_line_is_cut_at_a_char() {
    let line = "é".repeat(300);
    assert_eq!(preview(&line).chars().count(), 200);
}

#[test]
fn the_title_range_holds_only_the_heading_text() {
    let title = |body: &str| title_range(body).map(|range| body[range].to_owned());
    assert_eq!(title("# Plans\ntext\n").as_deref(), Some("Plans"));
    assert_eq!(title("\n\n  #   Plans  ##\n").as_deref(), Some("Plans"));
    assert_eq!(title("# Plans\r\n").as_deref(), Some("Plans"));
    assert_eq!(title("# C#\n").as_deref(), Some("C#"));
    assert_eq!(
        title("# **Bold** title\n").as_deref(),
        Some("**Bold** title")
    );
    assert_eq!(title("#\n").as_deref(), Some(""));
    assert_eq!(title("## Plans\n"), None);
    assert_eq!(title("#Plans\n"), None);
    assert_eq!(title("    # Code\n"), None);
    assert_eq!(title("text\n# Plans\n"), None);
    assert_eq!(title(""), None);
}
