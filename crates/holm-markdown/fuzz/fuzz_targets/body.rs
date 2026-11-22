//! Parses any text as a note body and checks that every range is valid.
#![no_main]

use std::ops::Range;

use holm_markdown::{Block, Body, OffsetRange, SourceOffset};
use libfuzzer_sys::fuzz_target;

fn check(text: &str, range: &Range<SourceOffset>) {
    let range = range.bytes();
    assert!(range.start <= range.end && range.end <= text.len());
    assert!(text.is_char_boundary(range.start) && text.is_char_boundary(range.end));
}

fn check_blocks(text: &str, blocks: &[Block]) {
    for block in blocks {
        check(text, &block.range);
        check_blocks(text, &block.children);
    }
}

fuzz_target!(|text: &str| {
    let body = Body::parse(text);
    check_blocks(text, body.blocks());
    for span in body.spans() {
        check(text, &span.range);
    }
    for range in body.hidden() {
        check(text, range);
    }
    for link in body.links() {
        check(text, &link.range);
        if let Some(target) = &link.target_range {
            check(text, target);
        }
    }
    for heading in body.headings() {
        check(text, &heading.range);
    }
});
