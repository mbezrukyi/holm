use crate::Snapshot;
use crate::model::NoteSet;
use crate::tag::TagName;

/// A change of one tag and its child tags in every note that has them.
pub(crate) enum TagEdit<'a> {
    /// `from` becomes `to`, and `from/x` becomes `to/x`.
    Rename { from: &'a TagName, to: &'a TagName },
    /// `tag` and `tag/x` go away.
    Delete(&'a TagName),
}

impl TagEdit<'_> {
    pub(crate) fn tag(&self) -> &TagName {
        match self {
            Self::Rename { from, .. } => from,
            Self::Delete(tag) => tag,
        }
    }

    /// How many notes in `snapshot` the edit changes.
    pub(crate) fn count(&self, snapshot: &Snapshot) -> usize {
        snapshot
            .matching(NoteSet::WithTag(self.tag()))
            .filter(|note| self.retag(&note.tags) != note.tags)
            .count()
    }

    /// A rename to a tag that the note has merges the two.
    pub(crate) fn retag(&self, tags: &[TagName]) -> Vec<TagName> {
        unique(tags.iter().filter_map(|tag| match self {
            Self::Rename { from, to } => Some(tag.moved(from, to).unwrap_or_else(|| tag.clone())),
            Self::Delete(gone) => (!gone.holds(tag)).then(|| tag.clone()),
        }))
    }
}

/// The tags in their order, with no repeats.
fn unique(tags: impl IntoIterator<Item = TagName>) -> Vec<TagName> {
    let mut out: Vec<TagName> = Vec::new();
    for tag in tags {
        if !out.contains(&tag) {
            out.push(tag);
        }
    }
    out
}
