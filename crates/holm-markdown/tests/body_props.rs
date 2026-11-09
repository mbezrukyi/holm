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
            "\n\n",
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
            "<!--",
            "-->",
            "    ",
            "]: ",
            "[a]: /x\n",
            "[a]",
        ])
        .prop_map(str::to_owned),
        any::<String>(),
    ]
}

fn body_text() -> impl Strategy<Value = String> {
    prop::collection::vec(piece(), 0..40).prop_map(|pieces| pieces.concat())
}

/// Lines of block syntax, joined by one or two line breaks, for texts with many blocks.
fn block_text() -> impl Strategy<Value = String> {
    let line = prop::sample::select(vec![
        "Some *text*",
        "more text",
        "# A heading",
        "Setext",
        "===",
        "---",
        "> a quote",
        "> [!tip]",
        "lazy line",
        "- item",
        "  - nested",
        "1. one",
        "2) two",
        "- [ ] task",
        "```",
        "~~~",
        "    code",
        "| a | b |",
        "|---|:-:|",
        "<!--",
        "-->",
        "<div>",
        "</div>",
        "[a]: /x",
        "A [link][a] and ==mark==",
        "",
        "  ",
    ]);
    let gap = prop::sample::select(vec!["\n", "\n\n", "\n \n"]);
    prop::collection::vec((line, gap), 0..40).prop_map(|lines| {
        lines
            .into_iter()
            .flat_map(|(line, gap)| [line, gap])
            .collect()
    })
}

/// A text and one edit in it: a range on char boundaries and the new text for it.
fn text_and_edit(
    text: impl Strategy<Value = String>,
) -> impl Strategy<Value = (String, Range<usize>, String)> {
    (
        text,
        any::<prop::sample::Index>(),
        any::<prop::sample::Index>(),
        prop::collection::vec(piece(), 0..3).prop_map(|pieces| pieces.concat()),
    )
        .prop_map(|(text, a, b, new)| {
            let bounds: Vec<usize> = (0..=text.len())
                .filter(|&i| text.is_char_boundary(i))
                .collect();
            let (a, b) = (*a.get(&bounds), *b.get(&bounds));
            (text, a.min(b)..a.max(b), new)
        })
}

/// Asserts that `reparsed` has the same result as a full parse of `text`.
fn assert_same(text: &str, reparsed: &Body) {
    let full = Body::parse(text);
    assert_eq!(reparsed.blocks(), full.blocks());
    assert_eq!(reparsed.spans(), full.spans());
    assert_eq!(reparsed.hidden(), full.hidden());
    assert_eq!(reparsed.links(), full.links());
    assert_eq!(reparsed.headings(), full.headings());
    let ranges = |body: &Body| {
        body.leaves()
            .iter()
            .map(holm_markdown::inline::Leaf::range)
            .collect::<Vec<_>>()
    };
    assert_eq!(ranges(reparsed), ranges(&full));
    for (leaf, full_leaf) in reparsed.leaves().iter().zip(full.leaves()) {
        assert_eq!(reparsed.inline(text, leaf), full.inline(text, full_leaf));
    }
}

fn check_reparse(old: &str, range: Range<usize>, new: &str) {
    let mut body = Body::parse(old);
    let mut text = old.to_owned();
    text.replace_range(range.clone(), new);
    body.reparse(&text, range, new.len());
    assert_same(&text, &body);
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
    fn reparse_after_an_edit_equals_a_full_parse(
        (old, range, new) in text_and_edit(prop::collection::vec(piece(), 0..80).prop_map(|pieces| pieces.concat()))
    ) {
        check_reparse(&old, range, &new);
    }

    #[test]
    fn reparse_of_many_blocks_equals_a_full_parse((old, range, new) in text_and_edit(block_text())) {
        check_reparse(&old, range, &new);
    }

    #[test]
    fn any_string_keeps_every_range_valid(text in any::<String>()) {
        check(&text);
    }
}
