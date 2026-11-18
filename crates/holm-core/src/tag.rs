//! The name of a tag and the rules of child tags.

use std::fmt;

use holm_markdown::normalize_tag;

use crate::error::{Error, Result};

/// The full name of a tag, like `work/holm`, as Holm writes it in the frontmatter.
///
/// Each part between two `/` is not empty and has no spaces. The tag `work` holds itself and
/// its child tags, like `work/holm`, but not `workshop`. Names match exactly, also in capital
/// letters. Its order is the order of the text.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TagName(String);

impl TagName {
    /// The name, or `None` when it is not written as Holm writes tags.
    pub fn new(name: &str) -> Option<Self> {
        (normalize_tag(name).as_deref() == Some(name)).then(|| Self(name.into()))
    }

    /// The tag that a person typed, cleaned: no spaces around a part, `-` for a space inside
    /// one, and no empty parts.
    ///
    /// # Errors
    ///
    /// [`Error::BadTag`] when nothing is left.
    pub fn typed(text: &str) -> Result<Self> {
        normalize_tag(text).map(Self).ok_or_else(|| Error::BadTag {
            tag: text.to_owned(),
        })
    }

    /// A name that is already written as Holm writes tags, like a tag of a parsed note.
    pub(crate) fn from_written(name: String) -> Self {
        debug_assert!(Self::new(&name).is_some(), "{name:?} is not a tag name");
        Self(name)
    }

    /// The full name.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The tag one level up: `work` for `work/holm`. A top tag has none.
    pub fn parent(&self) -> Option<Self> {
        parent(&self.0).map(|parent| Self::from_written(parent.to_owned()))
    }

    /// The part after the last `/`: `holm` for `work/holm`.
    pub fn last_part(&self) -> &str {
        self.0.rsplit_once('/').map_or(&self.0, |(_, last)| last)
    }

    /// Whether `tag` is this tag or one of its child tags, at any depth.
    pub fn holds(&self, tag: &Self) -> bool {
        child_part(&tag.0, &self.0).is_some()
    }

    /// The name after the tag `from` became `to`, or `None` when `from` does not hold this
    /// tag. Child tags follow: `work/holm` becomes `home/holm`.
    #[must_use]
    pub fn moved(&self, from: &Self, to: &Self) -> Option<Self> {
        let rest = child_part(&self.0, &from.0)?;
        Some(if rest.is_empty() {
            to.clone()
        } else {
            Self(format!("{to}{rest}"))
        })
    }

    /// This name and the name of each tag that holds it, as text: `work/holm`, then `work`.
    pub(crate) fn names(&self) -> impl Iterator<Item = &str> {
        std::iter::successors(Some(self.as_str()), |name| parent(name))
    }

    /// The tags of a parsed note.
    pub(crate) fn of_note(text: &holm_markdown::NoteFile) -> Vec<Self> {
        text.tags().into_iter().map(Self::from_written).collect()
    }
}

/// The name of the tag one level up from the tag `name`.
pub(crate) fn parent(name: &str) -> Option<&str> {
    name.rsplit_once('/').map(|(parent, _)| parent)
}

/// What follows `parent` in `tag`: `""` for `parent` itself, `"/holm"` for `parent/holm`.
/// `None` when `tag` is not `parent` or one of its child tags.
fn child_part<'t>(tag: &'t str, parent: &str) -> Option<&'t str> {
    tag.strip_prefix(parent)
        .filter(|rest| rest.is_empty() || rest.starts_with('/'))
}

impl PartialEq<str> for TagName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for TagName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl AsRef<str> for TagName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TagName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for TagName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.0, f)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    fn tag(name: &str) -> TagName {
        TagName::new(name).unwrap()
    }

    #[test]
    fn new_takes_only_written_names() {
        assert_eq!(tag("work/holm").as_str(), "work/holm");
        for name in [
            "",
            "/",
            "work/",
            "/work",
            "work//holm",
            " work",
            "my work",
            "work/ holm",
        ] {
            assert_eq!(TagName::new(name), None, "{name:?}");
        }
    }

    #[test]
    fn typed_cleans_the_text() {
        assert_eq!(
            TagName::typed(" my work / holm/ ").unwrap(),
            tag("my-work/holm")
        );
        assert!(matches!(TagName::typed(" / "), Err(Error::BadTag { tag }) if tag == " / "));
    }

    #[test]
    fn parent_and_last_part() {
        assert_eq!(tag("work/holm/ui").parent(), Some(tag("work/holm")));
        assert_eq!(tag("work").parent(), None);
        assert_eq!(tag("work/holm").last_part(), "holm");
        assert_eq!(tag("work").last_part(), "work");
    }

    #[test]
    fn a_tag_holds_itself_and_its_children() {
        let work = tag("work");
        assert!(work.holds(&work));
        assert!(work.holds(&tag("work/holm/ui")));
        assert!(!work.holds(&tag("workshop")));
        assert!(!work.holds(&tag("home/work")));
        assert!(!tag("work/holm").holds(&work));
        assert!(!tag("Work").holds(&work));
    }

    #[test]
    fn moved_takes_the_children_along() {
        let (from, to) = (tag("work"), tag("life/home"));
        assert_eq!(tag("work").moved(&from, &to), Some(tag("life/home")));
        assert_eq!(
            tag("work/holm").moved(&from, &to),
            Some(tag("life/home/holm"))
        );
        assert_eq!(tag("workshop").moved(&from, &to), None);
    }

    #[test]
    fn names_go_up_to_the_top() {
        let tag = tag("a/b/c");
        assert_eq!(tag.names().collect::<Vec<_>>(), ["a/b/c", "a/b", "a"]);
    }
}
