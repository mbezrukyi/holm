use std::time::SystemTime;

use holm_core::model::NoteInfo;
use holm_core::testing::note_path;

use super::{MAX_ROWS, PickNotes};

fn rows(query: &str, notes: &[NoteInfo]) -> Vec<usize> {
    super::rows(query, &PickNotes::new(notes.to_vec()))
}

fn notes(titles: &[String]) -> Vec<NoteInfo> {
    titles
        .iter()
        .map(|title| NoteInfo {
            path: note_path(&format!("{title}.md")),
            id: None,
            title: title.clone(),
            preview: String::new(),
            tags: Vec::new(),
            pinned: false,
            modified: SystemTime::UNIX_EPOCH,
        })
        .collect()
}

#[test]
fn titles_match_with_no_regard_to_case() {
    let list = notes(&["Plan".into(), "Goals".into(), "Top".into()]);
    assert_eq!(rows("", &list), [0, 1, 2]);
    assert_eq!(rows(" O ", &list), [1, 2]);
    assert_eq!(rows("milk", &list), Vec::<usize>::new());
}

#[test]
fn only_the_first_titles_show() {
    let many: Vec<String> = (0..MAX_ROWS + 10).map(|n| format!("Note {n}")).collect();
    let list = notes(&many);
    assert_eq!(rows("", &list), (0..MAX_ROWS).collect::<Vec<_>>());
    assert_eq!(rows("note 5", &list), [5, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59]);
}
