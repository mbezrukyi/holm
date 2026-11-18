use std::fmt;

use crate::frontmatter::{Frontmatter, Key};
use crate::preview::preview;
use crate::tag::clean_tags;

/// The line ending of a note.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineEnding {
    /// `\n`. A new note uses it.
    #[default]
    Lf,
    /// `\r\n`.
    CrLf,
}

impl LineEnding {
    /// Finds the line ending of `text` from its first line break. With no break, it is [`LineEnding::Lf`].
    pub fn detect(text: &str) -> Self {
        match text.find('\n') {
            Some(i) if text[..i].ends_with('\r') => Self::CrLf,
            _ => Self::Lf,
        }
    }

    /// The line ending as text.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
        }
    }
}

/// The text of one note, split into frontmatter and body.
///
/// [`NoteFile::parse`] never fails, and its [`Display`](fmt::Display) gives back the same bytes.
/// The setters change only Holm's own fields: `id`, `tags` and `pinned`.
/// Every other line stays in its place with its exact text.
///
/// ```
/// use holm_markdown::NoteFile;
///
/// let text = "---\nauthor: me\n---\n# Plans\n";
/// let mut note = NoteFile::parse(text);
/// assert_eq!(note.preview(), "Plans");
///
/// note.set_tags(&["work/holm"]);
/// assert_eq!(note.to_string(), "---\ntags:\n  - work/holm\nauthor: me\n---\n# Plans\n");
/// ```
#[derive(Clone, Debug)]
pub struct NoteFile {
    frontmatter: Option<Frontmatter>,
    body: String,
    line_ending: LineEnding,
}

impl NoteFile {
    /// Splits the text of a note.
    ///
    /// A frontmatter starts with a `---` line at the very top and ends with a `---` or `...` line.
    /// Without the end line, the whole text is body.
    pub fn parse(text: &str) -> Self {
        let (frontmatter, body) = Frontmatter::split(text);
        Self {
            frontmatter,
            body: body.to_owned(),
            line_ending: LineEnding::detect(text),
        }
    }

    /// The text after the frontmatter.
    pub fn body(&self) -> &str {
        &self.body
    }

    /// The line ending that new lines use. It comes from the first line break of the text.
    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    /// The `id` field, or `None` when it is missing or empty.
    pub fn id(&self) -> Option<String> {
        self.frontmatter.as_ref().and_then(Frontmatter::id)
    }

    /// The `tags` field, normalized like [`normalize_tag`](crate::normalize_tag), with no repeats.
    ///
    /// It reads a block list, a flow list like `[a, b]`, and a single tag.
    pub fn tags(&self) -> Vec<String> {
        self.frontmatter
            .as_ref()
            .map(Frontmatter::tags)
            .unwrap_or_default()
    }

    /// Whether the `pinned` field is `true`.
    pub fn pinned(&self) -> bool {
        self.frontmatter.as_ref().is_some_and(Frontmatter::pinned)
    }

    /// The first line of text of the body, with no Markdown syntax, for a note list.
    ///
    /// It is empty when the body has no text. A long line is cut to 200 chars.
    ///
    /// ```
    /// use holm_markdown::NoteFile;
    ///
    /// let note = NoteFile::parse("\n- Buy **milk**\n");
    /// assert_eq!(note.preview(), "Buy milk");
    /// ```
    pub fn preview(&self) -> String {
        preview(&self.body)
    }

    /// Replaces the text after the frontmatter. The frontmatter stays as it is.
    pub fn set_body(&mut self, body: &str) {
        body.clone_into(&mut self.body);
    }

    /// Sets the `id` field, or removes it with `None`.
    pub fn set_id(&mut self, id: Option<&str>) {
        let raw = id.map(|id| Frontmatter::write_id(id, self.line_ending.as_str()));
        self.set(Key::Id, raw);
    }

    /// Sets the `tags` field as a block list. With no tags left, it removes the field.
    ///
    /// It normalizes each tag like [`normalize_tag`](crate::normalize_tag) and drops repeats.
    pub fn set_tags<S: AsRef<str>>(&mut self, tags: &[S]) {
        let tags = clean_tags(tags);
        let raw =
            (!tags.is_empty()).then(|| Frontmatter::write_tags(&tags, self.line_ending.as_str()));
        self.set(Key::Tags, raw);
    }

    /// Writes `pinned: true`, or removes the field with `false`.
    pub fn set_pinned(&mut self, pinned: bool) {
        let raw = pinned.then(|| Frontmatter::write_pinned(self.line_ending.as_str()));
        self.set(Key::Pinned, raw);
    }

    /// Makes a frontmatter when a field needs one. Removes it when the last field goes.
    fn set(&mut self, key: Key, raw: Option<String>) {
        let line_ending = self.line_ending.as_str();
        match (&mut self.frontmatter, raw) {
            (None, None) => {}
            (None, Some(raw)) => {
                let mut frontmatter = Frontmatter::new(line_ending);
                frontmatter.set(key, Some(raw));
                self.frontmatter = Some(frontmatter);
            }
            (Some(frontmatter), raw) => {
                let was_empty = frontmatter.is_empty();
                frontmatter.set(key, raw);
                if frontmatter.is_empty() && !was_empty {
                    self.frontmatter = None;
                }
            }
        }
    }
}

impl fmt::Display for NoteFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(frontmatter) = &self.frontmatter {
            write!(f, "{frontmatter}")?;
        }
        f.write_str(&self.body)
    }
}
