use std::fmt::Write as _;
use std::fs;
use std::ops::Range;

use holm_markdown::{Block, Body};

fn show(text: &str, range: &Range<usize>) -> String {
    format!("{range:?} {:?}", &text[range.clone()])
}

fn blocks(out: &mut String, text: &str, blocks: &[Block], depth: usize) {
    for block in blocks {
        let indent = "  ".repeat(depth);
        let _ = writeln!(out, "{indent}{:?} {}", block.kind, show(text, &block.range));
        self::blocks(out, text, &block.children, depth + 1);
    }
}

fn report(text: &str) -> String {
    let body = Body::parse(text);
    let mut out = String::from("--- blocks ---\n");
    blocks(&mut out, text, body.blocks(), 0);
    out.push_str("--- spans ---\n");
    for span in body.spans() {
        let _ = writeln!(out, "{:?} {}", span.style, show(text, &span.range));
    }
    out.push_str("--- hidden ---\n");
    for range in body.hidden() {
        let _ = writeln!(out, "{}", show(text, range));
    }
    out.push_str("--- links ---\n");
    for link in body.links() {
        let _ = writeln!(out, "{}", show(text, &link.range));
        let _ = writeln!(out, "  target: {:?}", link.target);
        let target = link.target_range.as_ref().map(|r| show(text, r));
        let _ = writeln!(out, "  target range: {target:?}");
        let _ = writeln!(out, "  embedded: {}", link.embedded);
    }
    out.push_str("--- headings ---\n");
    for heading in body.headings() {
        let _ = writeln!(
            out,
            "{} {:?} {}",
            heading.level,
            heading.text,
            show(text, &heading.range)
        );
    }
    out
}

#[test]
fn bodies() {
    insta::glob!("bodies/*.md", |path| {
        let text = fs::read_to_string(path).unwrap();
        insta::assert_snapshot!(report(&text));
    });
}
