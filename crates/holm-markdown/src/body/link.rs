//! Link targets.

/// Where a link points.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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

impl Target {
    /// The target as a link destination, the part between `(` and `)`. Parsing it gives back
    /// this target, with a space for each line break.
    ///
    /// A destination with spaces or brackets goes in `< >`. A file path that reads as another
    /// kind of target gets `./` before it, so it parses back with `./`.
    ///
    /// ```
    /// use holm_markdown::Target;
    ///
    /// let target = Target::Note { id: "01JC3X".into(), heading: Some("Bug list".into()) };
    /// assert_eq!(target.to_destination(), "<holm:01JC3X#Bug list>");
    /// assert_eq!(Target::File("notes:2026.md".into()).to_destination(), "./notes:2026.md");
    /// ```
    #[must_use]
    pub fn to_destination(&self) -> String {
        let dest = match self {
            Self::Note { id, heading: None } => format!("holm:{id}"),
            Self::Note {
                id,
                heading: Some(heading),
            } => format!("holm:{id}#{heading}"),
            Self::Heading(heading) => format!("#{heading}"),
            Self::File(path) => {
                if target(path).is_some_and(|parsed| parsed == *self) {
                    path.clone()
                } else {
                    format!("./{path}")
                }
            }
            Self::Web(url) => url.clone(),
        };
        let dest = dest.replace(['\n', '\r'], " ");
        let pointy = dest.is_empty()
            || dest
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || "()<>".contains(c));
        let mut out = String::with_capacity(dest.len() + 2);
        if pointy {
            out.push('<');
        }
        for c in dest.chars() {
            if "\\&<>".contains(c) {
                out.push('\\');
            }
            out.push(c);
        }
        if pointy {
            out.push('>');
        }
        out
    }
}

/// Plain text as Markdown inline text that shows it as written, for the text of a new link.
///
/// ```
/// assert_eq!(holm_markdown::escape_text("[*draft*]"), r"\[\*draft\*\]");
/// ```
#[must_use]
pub fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c == '\n' || c == '\r' {
            out.push(' ');
            continue;
        }
        if "\\`*_~=<>[]&|".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
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
