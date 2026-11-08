//! Link targets and where they stand in the source.

use std::ops::Range;

use pulldown_cmark::LinkType;

/// Where a link points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// `holm:<id>`, or `holm:<id>#<heading>` for a heading in that note.
    Note {
        /// The note id.
        id: String,
        /// The heading text after `#`.
        heading: Option<String>,
    },
    /// `#<heading>`: a heading in the same note.
    Heading(String),
    /// A file path, relative to the note or absolute.
    File(String),
    /// Any other address with a scheme, like `https:` or `mailto:`.
    Web(String),
}

/// The target of a link destination, or `None` when it is empty.
pub(super) fn target(dest: &str, link_type: LinkType) -> Option<Target> {
    if dest.is_empty() {
        return None;
    }
    if link_type == LinkType::Email {
        return Some(Target::Web(format!("mailto:{dest}")));
    }
    if let Some(rest) = dest.strip_prefix("holm:") {
        let (id, heading) = match rest.split_once('#') {
            Some((id, heading)) => (id, Some(heading.to_owned())),
            None => (rest, None),
        };
        return Some(Target::Note {
            id: id.to_owned(),
            heading,
        });
    }
    if let Some(heading) = dest.strip_prefix('#') {
        return Some(Target::Heading(heading.to_owned()));
    }
    if has_scheme(dest) {
        return Some(Target::Web(dest.to_owned()));
    }
    Some(Target::File(dest.to_owned()))
}

/// A URL scheme: a letter, then letters, digits, `+`, `-` or `.`, then `:`.
/// One letter is not a scheme, so `C:` stays a path.
fn has_scheme(dest: &str) -> bool {
    let Some((scheme, _)) = dest.split_once(':') else {
        return false;
    };
    scheme.len() >= 2
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// The destination of an inline link `[text](dest "title")`, with its `< >`.
///
/// `text_end` is where the link text ends, at the `]`.
pub(super) fn target_range(text: &str, text_end: usize, end: usize) -> Option<Range<usize>> {
    let bytes = &text.as_bytes()[..end];
    if bytes.get(text_end) != Some(&b']') || bytes.get(text_end + 1) != Some(&b'(') {
        return None;
    }
    let mut start = text_end + 2;
    while bytes.get(start).is_some_and(u8::is_ascii_whitespace) {
        start += 1;
    }
    let mut i = start;
    if bytes.get(i) == Some(&b'<') {
        while i < end {
            match bytes[i] {
                b'\\' => i += 1,
                b'>' => return Some(start..i + 1),
                _ => {}
            }
            i += 1;
        }
        return None;
    }
    let mut depth = 0_usize;
    while i < end {
        match bytes[i] {
            b'\\' => i += 1,
            b'(' => depth += 1,
            b')' if depth == 0 => break,
            b')' => depth -= 1,
            b if b.is_ascii_whitespace() => break,
            _ => {}
        }
        i += 1;
    }
    let i = i.min(end);
    (i > start).then_some(start..i)
}
