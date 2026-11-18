//! Property tests for split, join and the field setters.

use holm_markdown::{NoteFile, normalize_tag};
use proptest::prelude::*;

/// Lines that look like frontmatter, Markdown and YAML, to reach the edge cases.
fn line() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("---".to_owned()),
        Just("...".to_owned()),
        Just("tags:".to_owned()),
        Just("pinned: true".to_owned()),
        Just(String::new()),
        "id: [a-z0-9\"' #]{0,6}",
        "tags: \\[[a-z,\"' #\\]]{0,10}",
        " {0,3}- [a-z \"'#]{0,6}",
        "#{1,2}[ a-z*`]{0,8}",
        "[a-z:# \\t-]{0,10}",
        any::<String>(),
    ]
}

fn note_text() -> impl Strategy<Value = String> {
    let parts = prop::collection::vec((line(), prop_oneof!["\n", "\r\n", "\r", ""]), 0..16);
    parts.prop_map(|parts| parts.into_iter().map(|(l, e)| l + &e).collect())
}

/// A field Holm does not know, a comment, or a field with lines below it.
/// Each line holds `Q<n>`, and Holm's own lines never hold a `Q`.
fn unknown(n: usize) -> impl Strategy<Value = Vec<String>> {
    prop_oneof![
        Just(vec![format!("Q{n}: value")]),
        Just(vec![format!("# Q{n} comment")]),
        Just(vec![format!("Q{n}:"), format!("  sub_Q{n}: x")]),
        Just(vec![format!("Q{n}:"), format!("- item_Q{n}")]),
    ]
}

#[derive(Clone, Debug)]
enum Op {
    Id(Option<String>),
    Tags(Vec<String>),
    Pinned(bool),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        proptest::option::of("[0-9A-H]{1,8}").prop_map(Op::Id),
        prop::collection::vec("[a-c/ ]{0,6}", 0..4).prop_map(Op::Tags),
        any::<bool>().prop_map(Op::Pinned),
    ]
}

/// Frontmatter lines: unknown entries with Holm fields between them, in any order.
fn frontmatter() -> impl Strategy<Value = (Vec<String>, Vec<String>)> {
    let holm = prop_oneof![
        Just(vec!["id: 01AB".to_owned()]),
        Just(vec!["tags: [a, b]".to_owned()]),
        Just(vec!["tags:".to_owned(), "  - c".to_owned()]),
        Just(vec!["pinned: true".to_owned()]),
    ];
    let unknowns = (0..6usize).prop_flat_map(|count| (0..count).map(unknown).collect::<Vec<_>>());
    (
        unknowns,
        prop::collection::vec(holm, 0..3),
        any::<prop::sample::Index>(),
    )
        .prop_map(|(unknowns, holm, index)| {
            let unknown_lines: Vec<String> = unknowns.concat();
            let mut entries = unknowns;
            for field in holm {
                let at = index.index(entries.len() + 1);
                entries.insert(at, field);
            }
            (entries.concat(), unknown_lines)
        })
}

proptest! {
    #[test]
    fn split_then_join_gives_the_same_bytes(text in note_text()) {
        let note = NoteFile::parse(&text);
        prop_assert_eq!(note.to_string(), text);
        // The readers never panic.
        let _ = (note.id(), note.tags(), note.pinned(), note.preview());
    }

    #[test]
    fn any_string_joins_back(text in any::<String>()) {
        prop_assert_eq!(NoteFile::parse(&text).to_string(), text);
    }

    #[test]
    fn setting_a_field_never_changes_an_unknown_line(
        (lines, unknown_lines) in frontmatter(),
        crlf in any::<bool>(),
        ops in prop::collection::vec(op(), 1..6),
    ) {
        let le = if crlf { "\r\n" } else { "\n" };
        let body = format!("# Title{le}{le}Body u text{le}");
        let mut text = format!("---{le}");
        for line in &lines {
            text.push_str(line);
            text.push_str(le);
        }
        text = format!("{text}---{le}{body}");

        let mut note = NoteFile::parse(&text);
        for op in &ops {
            match op {
                Op::Id(id) => note.set_id(id.as_deref()),
                Op::Tags(tags) => note.set_tags(tags),
                Op::Pinned(pinned) => note.set_pinned(*pinned),
            }
        }
        let out = note.to_string();

        let reparsed = NoteFile::parse(&out);
        prop_assert_eq!(reparsed.body(), body.as_str());
        let frontmatter = &out[..out.len() - body.len()];
        let kept: Vec<&str> = frontmatter.split(le).filter(|line| line.contains('Q')).collect();
        prop_assert_eq!(kept, unknown_lines.iter().map(String::as_str).collect::<Vec<_>>());
        let other_ending = if crlf { "\n\n" } else { "\r" };
        prop_assert!(!out.contains(other_ending));

        if let Some(Op::Id(id)) = ops.iter().rev().find(|op| matches!(op, Op::Id(_))) {
            prop_assert_eq!(reparsed.id(), id.clone());
        }
        if let Some(Op::Tags(tags)) = ops.iter().rev().find(|op| matches!(op, Op::Tags(_))) {
            let mut want: Vec<String> = Vec::new();
            for tag in tags.iter().filter_map(|tag| normalize_tag(tag)) {
                if !want.contains(&tag) {
                    want.push(tag);
                }
            }
            prop_assert_eq!(reparsed.tags(), want);
        }
        if let Some(Op::Pinned(pinned)) = ops.iter().rev().find(|op| matches!(op, Op::Pinned(_))) {
            prop_assert_eq!(reparsed.pinned(), *pinned);
        }
    }
}
