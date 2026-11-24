use super::*;
use crate::model::display::plain_chip;
use crate::model::{LineIndex, LinkTarget};

fn model(text: &str, selection: Range<usize>) -> Model {
    let mut model = Model::new(text);
    model.select(SourceOffset::range(selection));
    model
}

fn note(id: &str, heading: Option<&str>) -> Target {
    Target::Note {
        id: id.to_owned(),
        heading: heading.map(str::to_owned),
    }
}

#[test]
fn the_link_at_an_offset_is_the_innermost_one() {
    let text = "a [![img](i.png) b](holm:X) [c](d.md)";
    let model = Model::new(text);
    assert_eq!(model.link_at(SourceOffset::new(0)), None);
    assert_eq!(
        model.link_at(SourceOffset::new(5)).map(|l| &l.target),
        Some(&Target::File("i.png".into()))
    );
    assert_eq!(
        model.link_at(SourceOffset::new(18)).map(|l| &l.target),
        Some(&note("X", None))
    );
    // The end of a link counts when no other link starts there.
    assert_eq!(
        model.link_at(SourceOffset::new(27)).map(|l| &l.target),
        Some(&note("X", None))
    );
    assert_eq!(
        model.link_at(SourceOffset::new(28)).map(|l| &l.target),
        Some(&Target::File("d.md".into()))
    );
}

#[test]
fn a_new_link_takes_the_selection_as_its_text() {
    let mut m = model("see plans here", 4..9);
    assert!(m.insert_link("Plans", &note("X", Some("Bug list")), LinkForm::Text));
    assert_eq!(m.text(), "see [plans](<holm:X#Bug list>) here");
    assert_eq!(m.selection(), SourceOffset::range(30..30));
}

#[test]
fn a_new_link_with_no_selection_escapes_its_text() {
    let mut m = model("x ", 2..2);
    assert!(m.insert_link("A *bold* [idea]", &note("X", None), LinkForm::Text));
    assert_eq!(m.text(), r"x [A \*bold\* \[idea\]](holm:X)");
    let mut m = model("", 0..0);
    let photo = Target::File("attachments/a b.png".into());
    assert!(m.insert_link("photo", &photo, LinkForm::Block));
    assert_eq!(m.text(), "![photo](<attachments/a b.png>)\n");
    assert_eq!(m.selection(), SourceOffset::range(32..32));
    assert!(m.undo());
    assert_eq!(m.text(), "");
}

#[test]
fn an_inline_file_link_gets_spaces_where_the_text_needs_them() {
    let file = |path: &str| Target::File(path.into());
    let mut m = model("see now", 3..3);
    assert!(m.insert_link("a", &file("a.pdf"), LinkForm::Inline));
    assert!(m.insert_link("b", &file("b.pdf"), LinkForm::Inline));
    assert_eq!(m.text(), "see ![a](a.pdf) ![b](b.pdf) now");
    assert_eq!(m.cursor(), SourceOffset::new(27));
    let mut m = model("one two.", 7..7);
    assert!(m.insert_link("a", &file("a.pdf"), LinkForm::Inline));
    assert_eq!(m.text(), "one two ![a](a.pdf).");
    let mut m = model("one\n", 4..4);
    assert!(m.insert_link("a", &file("a.pdf"), LinkForm::Inline));
    assert_eq!(m.text(), "one\n![a](a.pdf)");
}

#[test]
fn a_new_target_keeps_the_link_text_and_the_cursor() {
    let mut m = model("go [there](https://a.org) now", 6..6);
    assert!(m.set_link_target(&note("X", None)));
    assert_eq!(m.text(), "go [there](holm:X) now");
    assert_eq!(m.cursor(), SourceOffset::new(6));
    let mut m = model("[a][ref]\n\n[ref]: b.md\n", 1..1);
    assert!(!m.set_link_target(&note("X", None)));
    let mut m = model("no link", 1..1);
    assert!(!m.set_link_target(&note("X", None)));
}

#[test]
fn same_note_heading_links_are_broken_without_their_heading() {
    let mut m = Model::new("# Plans\n\n## Bug list\n");
    assert!(!m.is_broken(&Target::Heading("Bug list".into())));
    assert!(m.is_broken(&Target::Heading("bug list".into())));
    assert!(!m.is_broken(&note("X", Some("a"))));
    m.set_broken(HashSet::from([note("X", Some("a"))]));
    assert!(m.is_broken(&note("X", Some("a"))));
    assert_eq!(m.heading_start("Bug list"), Some(SourceOffset::new(9)));
}

#[test]
fn broken_links_get_their_own_look() {
    let mut m = model("[a](holm:X#b) [c](#d) [e](holm:Y)\n\n# d\n", 40..40);
    m.set_broken(HashSet::from([note("X", Some("b"))]));
    let looks: Vec<_> = m
        .display_line(LineIndex(0), &mut plain_chip)
        .shown
        .runs
        .iter()
        .filter_map(|run| run.look.link)
        .map(|link| link == LinkTarget::Missing)
        .collect();
    assert_eq!(looks, [true, false, false]);
}

#[test]
fn find_ignores_case_and_gives_separate_matches() {
    let m = Model::new("Aaa ÄB äb aaa");
    assert_eq!(
        m.find_all("aa"),
        vec![SourceOffset::range(0..2), SourceOffset::range(12..14)]
    );
    assert_eq!(
        m.find_all("äb"),
        vec![SourceOffset::range(4..7), SourceOffset::range(8..11)]
    );
    assert_eq!(m.find_all(""), Vec::<Range<SourceOffset>>::new());
    assert_eq!(m.find_all("zz"), Vec::<Range<SourceOffset>>::new());
}
