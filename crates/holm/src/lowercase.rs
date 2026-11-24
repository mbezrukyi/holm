//! Text in lowercase, so two texts compare with no regard to capital letters.

use std::borrow::Cow;

pub(crate) struct Lowercase<'a>(Cow<'a, str>);

impl Lowercase<'static> {
    pub(crate) fn new(text: &str) -> Self {
        let mut lower = String::with_capacity(text.len());
        push_lowercase(&mut lower, text);
        Self(Cow::Owned(lower))
    }
}

impl Lowercase<'_> {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// The match rule of each list that a typed text filters, except the fuzzy match of the link picker.
    pub(crate) fn holds(&self, part: &Lowercase<'_>) -> bool {
        self.0.contains(part.as_str())
    }
}

/// All in one string, so a list makes them with few allocations.
pub(crate) struct LowercaseList {
    text: String,
    ends: Vec<usize>,
}

impl LowercaseList {
    pub(crate) fn new<'a>(texts: impl ExactSizeIterator<Item = &'a str>) -> Self {
        let mut text = String::new();
        let mut ends = Vec::with_capacity(texts.len());
        for one in texts {
            push_lowercase(&mut text, one);
            ends.push(text.len());
        }
        Self { text, ends }
    }

    /// In the order of `new`.
    pub(crate) fn iter(&self) -> impl Iterator<Item = Lowercase<'_>> {
        let starts = std::iter::once(0).chain(self.ends.iter().copied());
        starts
            .zip(&self.ends)
            .map(|(start, end)| Lowercase(Cow::Borrowed(&self.text[start..*end])))
    }
}

fn push_lowercase(to: &mut String, text: &str) {
    if text.is_ascii() {
        let start = to.len();
        to.push_str(text);
        to[start..].make_ascii_lowercase();
    } else {
        to.extend(text.chars().flat_map(char::to_lowercase));
    }
}

#[cfg(test)]
mod tests;
