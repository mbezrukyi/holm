use std::fmt;

use crate::frontmatter::{Frontmatter, Key};
use crate::preview::preview;
use crate::tag::clean_tags;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
}

impl LineEnding {
    /// From the first line break of `text`. With no break, [`LineEnding::Lf`].
    pub fn detect(text: &str) -> Self {
        match text.find('\n') {
            Some(i) if text[..i].ends_with('\r') => Self::CrLf,
            _ => Self::Lf,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
        }
    }
}

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

    pub fn body(&self) -> &str {
        &self.body
    }

    /// New lines use it.
    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    /// `None` when the field is missing or empty.
    pub fn id(&self) -> Option<String> {
        self.frontmatter.as_ref().and_then(Frontmatter::id)
    }

    /// Normalized like [`normalize_tag`](crate::normalize_tag), with no repeats.
    ///
    /// It reads a block list, a flow list like `[a, b]`, and a single tag.
    pub fn tags(&self) -> Vec<String> {
        self.frontmatter.as_ref().map(Frontmatter::tags).unwrap_or_default()
    }

    pub fn pinned(&self) -> bool {
        self.frontmatter.as_ref().is_some_and(Frontmatter::pinned)
    }

    /// The first line that has text, without its Markdown syntax. A long line is cut to 200 chars.
    /// Empty when the body has no text.
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

    /// The frontmatter stays as it is.
    pub fn set_body(&mut self, body: &str) {
        body.clone_into(&mut self.body);
    }

    /// `None` removes the field.
    pub fn set_id(&mut self, id: Option<&str>) {
        let raw = id.map(|id| Frontmatter::write_id(id, self.line_ending.as_str()));
        self.set(Key::Id, raw);
    }

    /// Writes a block list, or removes the field when no tag is left.
    ///
    /// It normalizes each tag like [`normalize_tag`](crate::normalize_tag) and drops repeats.
    pub fn set_tags<S: AsRef<str>>(&mut self, tags: &[S]) {
        let tags = clean_tags(tags);
        let raw = (!tags.is_empty()).then(|| Frontmatter::write_tags(&tags, self.line_ending.as_str()));
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
