use holm_markdown::{Body, Target, escape_text};
use proptest::prelude::*;

/// The target of the one link in `[text](dest)`, and the text a reader sees. A heading around the link gives the text,
/// as heading links match it.
fn parse_link(text: &str, dest: &str) -> (Target, String) {
    let source = format!("# [{text}]({dest})\n");
    let body = Body::parse(&source);
    let [link] = body.links() else {
        panic!("one link in {source:?}");
    };
    (link.target.clone(), body.headings()[0].text.clone())
}

#[test]
fn destinations() {
    let cases = [
        (Target::File("a.png".into()), "a.png"),
        (Target::File("my file.pdf".into()), "<my file.pdf>"),
        (Target::File("a(1).png".into()), "<a(1).png>"),
        (Target::File("/home/me/a b.txt".into()), "</home/me/a b.txt>"),
        (Target::Heading("Bug list".into()), "<#Bug list>"),
        (
            Target::Note {
                id: "01JC3X".into(),
                heading: None,
            },
            "holm:01JC3X",
        ),
        (Target::Web("https://a.org/?q=1&b=2".into()), r"https://a.org/?q=1\&b=2"),
    ];
    let dotted = [
        (Target::File("#top.md".into()), "./#top.md"),
        (Target::File("holm:x".into()), "./holm:x"),
        (Target::File("notes:2026.md".into()), "./notes:2026.md"),
    ];
    for (target, want) in dotted {
        assert_eq!(target.to_destination(), want);
        assert_eq!(parse_link("x", want).0, Target::File(want.to_owned()));
    }
    for (target, want) in cases {
        assert_eq!(target.to_destination(), want);
        assert_eq!(parse_link("x", want).0, target, "dest {want:?}");
    }
}

fn piece() -> impl Strategy<Value = String> {
    "[a-z0-9 ()<>\\\\&#:./!\\[\\]*_`~=|é]{1,12}"
}

fn any_target() -> impl Strategy<Value = Target> {
    prop_oneof![
        ("[A-Z0-9]{1,8}", prop::option::of(piece())).prop_map(|(id, heading)| Target::Note { id, heading }),
        piece().prop_map(Target::Heading),
        piece().prop_map(Target::File),
        piece().prop_map(|rest| Target::Web(format!("https://{rest}"))),
    ]
}

proptest! {
    #[test]
    fn a_written_target_parses_back(target in any_target()) {
        let dest = target.to_destination();
        let parsed = match parse_link("x", &dest).0 {
            Target::File(path) if !matches!(&target, Target::File(p) if p.starts_with("./")) => {
                Target::File(path.strip_prefix("./").map_or(path.clone(), str::to_owned))
            }
            parsed => parsed,
        };
        prop_assert_eq!(parsed, target);
    }

    #[test]
    fn escaped_text_shows_as_written(text in "[a-z ()<>\\\\&#:./!\\[\\]*_`~=|é-]{1,16}") {
        let text = text.trim().to_owned();
        prop_assume!(!text.is_empty());
        let shown = parse_link(&escape_text(&text), "a.md").1;
        prop_assert_eq!(shown, text);
    }
}
