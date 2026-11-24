//! The rows of the link picker for the text in its field.
//!
//! The field matches note titles with fuzzy matching. A `#` after an exact title switches to the headings of that note.
//! A field that starts with `#` lists the headings of the open note.

use crate::lowercase::Lowercase;
use crate::pick_notes::PickNotes;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Query<'a> {
    Titles(&'a str),
    /// The headings of the note at this place in the picker notes, or of the open note with `None`.
    Headings {
        note: Option<usize>,
        filter: &'a str,
    },
}

impl<'a> Query<'a> {
    /// The first `#` whose text before it is a whole title switches to headings. So "C# notes" stays a title, unless a
    /// note has the title "C".
    pub(crate) fn parse(text: &'a str, notes: &PickNotes) -> Self {
        if let Some(filter) = text.strip_prefix('#') {
            return Self::Headings { note: None, filter };
        }
        for (at, _) in text.match_indices('#') {
            if let Some(note) = notes.position(&Lowercase::new(&text[..at])) {
                return Self::Headings {
                    note: Some(note),
                    filter: &text[at + 1..],
                };
            }
        }
        Self::Titles(text)
    }

    /// The field text that [`Self::parse`] reads back: `title#heading`, `title`, or `#heading` for the open note. An
    /// empty heading gives `title#`, the headings of the note with no filter.
    pub(crate) fn text(title: Option<&str>, heading: Option<&str>) -> String {
        match (title, heading) {
            (Some(title), Some(heading)) => format!("{title}#{heading}"),
            (Some(title), None) => title.to_owned(),
            (None, Some(heading)) => format!("#{heading}"),
            (None, None) => String::new(),
        }
    }
}

/// The list lays out each row it has, so more rows cost time with each key.
const MAX_NOTES: usize = 50;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LinkRow {
    /// The note at this place in the picker notes.
    Note(usize),
    Heading(String),
    /// A new note with this title.
    Create(String),
}

/// `headings` are those of the note that the query names.
///
/// Titles keep the order of `notes` with an empty query. Else the best match comes first. Only the first [`MAX_NOTES`]
/// titles show. When no title is the query itself, the last row creates a note.
pub(crate) fn rows(query: Query<'_>, notes: &PickNotes, headings: &[String]) -> Vec<LinkRow> {
    match query {
        Query::Titles(text) => {
            let text = text.trim();
            let pattern = Lowercase::new(text);
            let mut rows: Vec<LinkRow> = ranked(&pattern, notes.titles())
                .into_iter()
                .take(MAX_NOTES)
                .map(LinkRow::Note)
                .collect();
            if !text.is_empty() && notes.position(&pattern).is_none() {
                rows.push(LinkRow::Create(text.to_owned()));
            }
            rows
        }
        Query::Headings { filter, .. } => {
            let lowered = headings.iter().map(|text| Lowercase::new(text));
            ranked(&Lowercase::new(filter.trim()), lowered)
                .into_iter()
                .map(|at| LinkRow::Heading(headings[at].clone()))
                .collect()
        }
    }
}

/// The places of the texts that match `pattern`, the best match first. With an empty pattern, all of them in their
/// order.
fn ranked<'a>(pattern: &Lowercase<'_>, texts: impl Iterator<Item = Lowercase<'a>>) -> Vec<usize> {
    let mut hits: Vec<(i64, usize)> = texts
        .enumerate()
        .filter_map(|(at, text)| score(pattern, &text).map(|score| (score, at)))
        .collect();
    // A stable sort keeps the order of equal scores.
    hits.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    hits.into_iter().map(|(_, at)| at).collect()
}

/// How well `text` matches `pattern`, or `None` when the chars of `pattern` are not all in `text` in order. Chars in a
/// row and chars at the start of a word score more. Text with the pattern as one piece scores most.
fn score(pattern: &Lowercase<'_>, text: &Lowercase<'_>) -> Option<i64> {
    let (pattern, text) = (pattern.as_str(), text.as_str());
    if pattern.is_empty() {
        return Some(0);
    }
    let mut wanted = pattern.chars().peekable();
    let mut score = 0;
    let mut last: Option<usize> = None;
    let mut before: Option<char> = None;
    for (at, c) in text.chars().enumerate() {
        let Some(&want) = wanted.peek() else {
            break;
        };
        if c == want {
            score += 1;
            if last.is_some_and(|last| last + 1 == at) {
                score += 4;
            }
            if before.is_none_or(|before| !before.is_alphanumeric()) {
                score += 3;
            }
            last = Some(at);
            wanted.next();
        }
        before = Some(c);
    }
    if wanted.peek().is_some() {
        return None;
    }
    if text.starts_with(pattern) {
        score += 20;
    } else if text.contains(pattern) {
        score += 10;
    }
    Some(score)
}

#[cfg(test)]
mod tests;
