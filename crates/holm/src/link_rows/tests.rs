use std::time::SystemTime;

use holm_core::model::NoteInfo;
use holm_core::testing::note_path;

use super::*;

fn notes(titles: &[&str]) -> Vec<NoteInfo> {
    titles
        .iter()
        .map(|title| NoteInfo {
            path: note_path(&format!("{title}.md")),
            id: None,
            title: (*title).to_owned(),
            preview: String::new(),
            tags: Vec::new(),
            pinned: false,
            modified: SystemTime::UNIX_EPOCH,
        })
        .collect()
}

fn titles(text: &str, notes: &[NoteInfo]) -> Vec<String> {
    rows(Query::Titles(text), &PickNotes::new(notes.to_vec()), &[])
        .into_iter()
        .map(|row| match row {
            LinkRow::Note(at) => notes[at].title.clone(),
            LinkRow::Create(title) => format!("+{title}"),
            LinkRow::Heading(text) => format!("#{text}"),
        })
        .collect()
}

#[test]
fn titles_match_in_order_and_fuzzy() {
    let list = notes(&["Plans", "Holm ideas", "Shopping list", "Pasta"]);
    assert_eq!(titles("", &list), ["Plans", "Holm ideas", "Shopping list", "Pasta"]);
    assert_eq!(titles("pla", &list), ["Plans", "+pla"]);
    assert_eq!(titles("hi", &list), ["Holm ideas", "Shopping list", "+hi"]);
    assert_eq!(titles("PLANS", &list), ["Plans"]);
    assert_eq!(titles("zz", &list), ["+zz"]);
    assert_eq!(titles("pa", &list)[0], "Pasta");
}

#[test]
fn only_the_best_titles_show_and_the_create_row_stays() {
    let many: Vec<String> = (0..MAX_NOTES + 10).map(|n| format!("Note {n}")).collect();
    let list = notes(&many.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(titles("", &list).len(), MAX_NOTES);
    let shown = titles("note", &list);
    assert_eq!(shown.len(), MAX_NOTES + 1);
    assert_eq!(shown[0], "Note 0");
    assert_eq!(shown[MAX_NOTES], "+note");
}

#[test]
fn a_hash_after_a_whole_title_switches_to_headings() {
    let parse = |text, list: &[NoteInfo]| Query::parse(text, &PickNotes::new(list.to_vec()));
    let list = notes(&["Plans", "C", "C# notes"]);
    assert_eq!(
        parse("plans#bug", &list),
        Query::Headings {
            note: Some(0),
            filter: "bug"
        }
    );
    assert_eq!(parse("Pla#", &list), Query::Titles("Pla#"));
    assert_eq!(
        parse("#top", &list),
        Query::Headings {
            note: None,
            filter: "top"
        }
    );
    // "C" is a title, so its `#` wins over "C# notes".
    assert_eq!(
        parse("C# notes", &list),
        Query::Headings {
            note: Some(1),
            filter: " notes"
        }
    );
    let list = notes(&["C# notes"]);
    assert_eq!(parse("C# no", &list), Query::Titles("C# no"));
}

#[test]
fn parse_reads_what_text_writes() {
    let list = notes(&["Plans", "Ideas"]);
    let notes = PickNotes::new(list);
    let parse = |title, heading| match Query::parse(&Query::text(title, heading), &notes) {
        Query::Titles(text) => (None, text.to_owned()),
        Query::Headings { note, filter } => (Some(note), filter.to_owned()),
    };
    assert_eq!(parse(Some("Ideas"), Some("Bug")), (Some(Some(1)), "Bug".to_owned()));
    assert_eq!(parse(Some("Ideas"), Some("")), (Some(Some(1)), String::new()));
    assert_eq!(parse(None, Some("Top")), (Some(None), "Top".to_owned()));
    assert_eq!(parse(Some("Plans"), None), (None, "Plans".to_owned()));
    assert_eq!(parse(None, None), (None, String::new()));
}

#[test]
fn headings_filter_with_fuzzy_matching() {
    let headings = ["Bug list".to_owned(), "Ideas".to_owned(), "Big bets".to_owned()];
    let query = |filter| Query::Headings { note: None, filter };
    let shown = |filter| -> Vec<LinkRow> { rows(query(filter), &PickNotes::new(Vec::new()), &headings) };
    assert_eq!(shown("").len(), 3);
    assert_eq!(
        shown("bi"),
        [
            LinkRow::Heading("Big bets".to_owned()),
            LinkRow::Heading("Bug list".to_owned())
        ]
    );
    assert_eq!(shown("xyz"), []);
}
