//! Property tests for the body parser.

use std::ops::Range;

use holm_markdown::{Block, Body};
use proptest::prelude::*;

/// Pieces of Markdown syntax, to reach the edge cases more often than random text.
fn piece() -> impl Strategy<Value = String> {
    prop_oneof![
        prop::sample::select(vec![
            "# ",
            "## ",
            "===",
            "==",
            "=",
            "\\",
            "*",
            "**",
            "~~",
            "`",
            "``",
            "```",
            "~~~",
            "<u>",
            "</u>",
            "<span style=\"color: #abc\">",
            "</span>",
            "<mark>",
            "</mark>",
            "<mark style='background:#fde68a'>",
            "> ",
            ">",
            "[!note]",
            "- ",
            "1. ",
            "[ ] ",
            "[x] ",
            "|",
            "|---|",
            "---",
            "[",
            "]",
            "(",
            ")",
            "](",
            "![",
            "<",
            ">",
            "holm:",
            "#",
            "\n",
            "\r\n",
            "\r",
            " ",
            "  ",
            "\t",
            "a",
            "é",
            "Ш",
            "😀",
            "&amp;",
            "<div>",
        ])
        .prop_map(str::to_owned),
        any::<String>(),
    ]
}

fn body_text() -> impl Strategy<Value = String> {
    prop::collection::vec(piece(), 0..40).prop_map(|pieces| pieces.concat())
}

fn assert_range(text: &str, range: &Range<usize>) {
    assert!(range.start <= range.end, "{range:?}");
    assert!(range.end <= text.len(), "{range:?}");
    assert!(text.is_char_boundary(range.start), "{range:?}");
    assert!(text.is_char_boundary(range.end), "{range:?}");
}

fn assert_blocks(text: &str, blocks: &[Block], parent: &Range<usize>) {
    for block in blocks {
        assert_range(text, &block.range);
        assert!(
            parent.start <= block.range.start && block.range.end <= parent.end,
            "{:?} is outside {parent:?}",
            block.range
        );
        assert_blocks(text, &block.children, &block.range);
    }
}

fn check(text: &str) {
    let body = Body::parse(text);
    assert_blocks(text, body.blocks(), &(0..text.len()));
    for span in body.spans() {
        assert_range(text, &span.range);
        assert!(!span.range.is_empty());
    }
    for pair in body.spans().windows(2) {
        assert!(pair[0].range.start <= pair[1].range.start);
    }
    for range in body.hidden() {
        assert_range(text, range);
        assert!(!range.is_empty());
    }
    for pair in body.hidden().windows(2) {
        assert!(pair[0].end < pair[1].start, "{pair:?}");
    }
    for link in body.links() {
        assert_range(text, &link.range);
        if let Some(target) = &link.target_range {
            assert_range(text, target);
            assert!(link.range.start <= target.start && target.end <= link.range.end);
        }
    }
    for heading in body.headings() {
        assert_range(text, &heading.range);
        assert!((1..=6).contains(&heading.level));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    #[test]
    fn markdown_pieces_keep_every_range_valid(text in body_text()) {
        check(&text);
    }

    #[test]
    fn any_string_keeps_every_range_valid(text in any::<String>()) {
        check(&text);
    }
}
