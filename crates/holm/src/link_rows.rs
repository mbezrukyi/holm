//! The rows of the link picker for the text in its field.
//!
//! The field matches note titles with fuzzy matching. A `#` after an exact title switches to the
//! headings of that note. A field that starts with `#` lists the headings of the open note.

use holm_core::model::NoteInfo;

/// What the field asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Query<'a> {
    Titles(&'a str),
    /// The headings of the note at this place in the note list, or of the open note with `None`.
    Headings {
        note: Option<usize>,
        filter: &'a str,
    },
}

impl<'a> Query<'a> {
    /// Reads the field. The first `#` whose text before it is a whole title switches to
    /// headings. So "C# notes" stays a title, unless a note has the title "C".
    pub(crate) fn parse(text: &'a str, notes: &[NoteInfo]) -> Self {
        if let Some(filter) = text.strip_prefix('#') {
            return Self::Headings { note: None, filter };
        }
        for (at, _) in text.match_indices('#') {
            let title = &text[..at];
            if let Some(note) = notes.iter().position(|note| same(&note.title, title)) {
                return Self::Headings {
                    note: Some(note),
                    filter: &text[at + 1..],
                };
            }
        }
        Self::Titles(text)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Row {
    /// The note at this place in the note list.
    Note(usize),
    Heading(String),
    /// A new note with this title.
    Create(String),
}

/// The rows for `query`. `headings` are those of the note that the query names.
///
/// Titles keep the order of `notes` with an empty query. Else the best match comes first.
/// When no title is the query itself, the last row creates a note.
pub(crate) fn rows(query: Query<'_>, notes: &[NoteInfo], headings: &[String]) -> Vec<Row> {
    match query {
        Query::Titles(text) => {
            let text = text.trim();
            let mut rows: Vec<Row> = ranked(text, notes.iter().map(|note| note.title.as_str()))
                .into_iter()
                .map(Row::Note)
                .collect();
            if !text.is_empty() && !notes.iter().any(|note| same(&note.title, text)) {
                rows.push(Row::Create(text.to_owned()));
            }
            rows
        }
        Query::Headings { filter, .. } => {
            ranked(filter.trim(), headings.iter().map(String::as_str))
                .into_iter()
                .map(|at| Row::Heading(headings[at].clone()))
                .collect()
        }
    }
}

/// Whether a title is the typed text. Capital letters make no difference.
fn same(title: &str, text: &str) -> bool {
    title.to_lowercase() == text.to_lowercase()
}

/// The places of the texts that match `pattern`, the best match first. With an empty pattern,
/// all of them in their order.
fn ranked<'a>(pattern: &str, texts: impl Iterator<Item = &'a str>) -> Vec<usize> {
    let mut hits: Vec<(i64, usize)> = texts
        .enumerate()
        .filter_map(|(at, text)| score(pattern, text).map(|score| (score, at)))
        .collect();
    // A stable sort keeps the order of equal scores.
    hits.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    hits.into_iter().map(|(_, at)| at).collect()
}

/// How well `text` matches `pattern`, or `None` when the chars of `pattern` are not all in
/// `text` in order. Capital letters make no difference. Chars in a row and chars at the start
/// of a word score more. Text with the pattern as one piece scores most.
fn score(pattern: &str, text: &str) -> Option<i64> {
    let pattern: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    if pattern.is_empty() {
        return Some(0);
    }
    let text: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let mut score = 0;
    let mut next = 0;
    let mut last: Option<usize> = None;
    for (at, c) in text.iter().enumerate() {
        if next == pattern.len() {
            break;
        }
        if *c != pattern[next] {
            continue;
        }
        score += 1;
        if last.is_some_and(|last| last + 1 == at) {
            score += 4;
        }
        if at == 0 || !text[at - 1].is_alphanumeric() {
            score += 3;
        }
        last = Some(at);
        next += 1;
    }
    if next < pattern.len() {
        return None;
    }
    if text.starts_with(&pattern) {
        score += 20;
    } else if text.windows(pattern.len()).any(|part| part == pattern) {
        score += 10;
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::SystemTime;

    use super::*;

    fn notes(titles: &[&str]) -> Vec<NoteInfo> {
        titles
            .iter()
            .map(|title| NoteInfo {
                path: PathBuf::from(format!("{title}.md")),
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
        rows(Query::Titles(text), notes, &[])
            .into_iter()
            .map(|row| match row {
                Row::Note(at) => notes[at].title.clone(),
                Row::Create(title) => format!("+{title}"),
                Row::Heading(text) => format!("#{text}"),
            })
            .collect()
    }

    #[test]
    fn titles_match_in_order_and_fuzzy() {
        let list = notes(&["Plans", "Holm ideas", "Shopping list", "Pasta"]);
        assert_eq!(
            titles("", &list),
            ["Plans", "Holm ideas", "Shopping list", "Pasta"]
        );
        assert_eq!(titles("pla", &list), ["Plans", "+pla"]);
        assert_eq!(titles("hi", &list), ["Holm ideas", "Shopping list", "+hi"]);
        assert_eq!(titles("PLANS", &list), ["Plans"]);
        assert_eq!(titles("zz", &list), ["+zz"]);
        assert_eq!(titles("pa", &list)[0], "Pasta");
    }

    #[test]
    fn a_hash_after_a_whole_title_switches_to_headings() {
        let list = notes(&["Plans", "C", "C# notes"]);
        assert_eq!(
            Query::parse("plans#bug", &list),
            Query::Headings {
                note: Some(0),
                filter: "bug"
            }
        );
        assert_eq!(Query::parse("Pla#", &list), Query::Titles("Pla#"));
        assert_eq!(
            Query::parse("#top", &list),
            Query::Headings {
                note: None,
                filter: "top"
            }
        );
        // "C" is a title, so its `#` wins over "C# notes".
        assert_eq!(
            Query::parse("C# notes", &list),
            Query::Headings {
                note: Some(1),
                filter: " notes"
            }
        );
        let list = notes(&["C# notes"]);
        assert_eq!(Query::parse("C# no", &list), Query::Titles("C# no"));
    }

    #[test]
    fn headings_filter_with_fuzzy_matching() {
        let headings = [
            "Bug list".to_owned(),
            "Ideas".to_owned(),
            "Big bets".to_owned(),
        ];
        let query = |filter| Query::Headings { note: None, filter };
        let shown = |filter| -> Vec<Row> { rows(query(filter), &[], &headings) };
        assert_eq!(shown("").len(), 3);
        assert_eq!(
            shown("bi"),
            [
                Row::Heading("Big bets".to_owned()),
                Row::Heading("Bug list".to_owned())
            ]
        );
        assert_eq!(shown("xyz"), []);
    }
}
