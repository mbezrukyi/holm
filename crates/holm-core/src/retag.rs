//! Tag changes in many notes: a rename or a delete of a tag with its child tags.

use holm_markdown::normalize_tag;

use crate::error::{Error, Result};

/// A change of one tag and its child tags in every note that has them.
pub(crate) enum TagEdit<'a> {
    /// `from` becomes `to`, and `from/x` becomes `to/x`.
    Rename { from: &'a str, to: String },
    /// `tag` and `tag/x` go away.
    Delete(&'a str),
}

impl<'a> TagEdit<'a> {
    /// A rename of `from` to `to`. It cleans `to` like a typed tag.
    pub(crate) fn rename(from: &'a str, to: &str) -> Result<Self> {
        let to = normalize_tag(to).ok_or_else(|| bad_tag(to))?;
        Ok(Self::Rename {
            from: check(from)?,
            to,
        })
    }

    pub(crate) fn delete(tag: &'a str) -> Result<Self> {
        Ok(Self::Delete(check(tag)?))
    }

    /// The tag that the edit changes.
    pub(crate) fn tag(&self) -> &str {
        match self {
            Self::Rename { from, .. } => from,
            Self::Delete(tag) => tag,
        }
    }

    /// The tags of a note after the edit. A rename to a tag that the note has merges the two.
    pub(crate) fn apply(&self, tags: &[String]) -> Vec<String> {
        unique(tags.iter().filter_map(|tag| {
            let Some(rest) = child_part(tag, self.tag()) else {
                return Some(tag.clone());
            };
            match self {
                Self::Rename { to, .. } => Some(format!("{to}{rest}")),
                Self::Delete(_) => None,
            }
        }))
    }
}

/// The tags in their order, with no repeats.
pub(crate) fn unique(tags: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags {
        if !out.contains(&tag) {
            out.push(tag);
        }
    }
    out
}

/// What follows `parent` in `tag`: `""` for `parent` itself, `"/holm"` for `parent/holm`.
/// `None` when `tag` is not `parent` or one of its child tags.
fn child_part<'t>(tag: &'t str, parent: &str) -> Option<&'t str> {
    tag.strip_prefix(parent)
        .filter(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Checks that `tag` is a tag name as Holm writes it.
fn check(tag: &str) -> Result<&str> {
    if normalize_tag(tag).as_deref() == Some(tag) {
        Ok(tag)
    } else {
        Err(bad_tag(tag))
    }
}

fn bad_tag(tag: &str) -> Error {
    Error::BadTag {
        tag: tag.to_owned(),
    }
}
