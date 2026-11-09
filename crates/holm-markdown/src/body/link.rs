//! Link targets.

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
pub(crate) fn target(dest: &str) -> Option<Target> {
    if dest.is_empty() {
        return None;
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
