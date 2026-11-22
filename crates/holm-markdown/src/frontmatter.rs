//! The frontmatter as a list of entries. Each entry keeps its exact text.

use crate::tag::clean_tags;
use crate::yaml::{read_flow_list, read_scalar, write_scalar};

/// One of Holm's own fields. The order of the cases is the order in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Key {
    Id,
    Tags,
    Pinned,
}

impl Key {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "id" => Some(Self::Id),
            "tags" => Some(Self::Tags),
            "pinned" => Some(Self::Pinned),
            _ => None,
        }
    }
}

/// A top level field with its lines below it, or a comment or a blank line.
#[derive(Clone, Debug)]
struct Entry {
    raw: String,
    key: Option<Key>,
}

#[derive(Clone, Debug)]
pub(crate) struct Frontmatter {
    open: String,
    entries: Vec<Entry>,
    close: String,
}

impl Frontmatter {
    /// Splits `text` into frontmatter and body. Without a closed frontmatter, all text is body.
    pub(crate) fn split(text: &str) -> (Option<Self>, &str) {
        let mut lines = text.split_inclusive('\n');
        let Some(open) = lines.next() else {
            return (None, text);
        };
        if !open.ends_with('\n') || !is_fence(open, "---") {
            return (None, text);
        }
        let mut inner = Vec::new();
        let mut end = open.len();
        for line in lines {
            end += line.len();
            if is_fence(line, "---") || is_fence(line, "...") {
                let frontmatter = Self {
                    open: open.to_owned(),
                    entries: entries(&inner),
                    close: line.to_owned(),
                };
                return (Some(frontmatter), &text[end..]);
            }
            inner.push(line);
        }
        (None, text)
    }

    /// An empty frontmatter, for a note that has none.
    pub(crate) fn new(line_ending: &str) -> Self {
        Self {
            open: format!("---{line_ending}"),
            entries: Vec::new(),
            close: format!("---{line_ending}"),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn raw(&self, key: Key) -> Option<&str> {
        self.entries.iter().find(|e| e.key == Some(key)).map(|e| e.raw.as_str())
    }

    pub(crate) fn id(&self) -> Option<String> {
        let value = value(self.raw(Key::Id)?);
        let (id, _) = read_scalar(value, false);
        (!id.is_empty()).then_some(id)
    }

    pub(crate) fn pinned(&self) -> bool {
        // A quoted "true" is a string in YAML, not a bool.
        self.raw(Key::Pinned).is_some_and(|raw| {
            let value = value(raw).trim_start();
            !value.starts_with(['"', '\'']) && matches!(read_scalar(value, false).0.as_str(), "true" | "True" | "TRUE")
        })
    }

    pub(crate) fn tags(&self) -> Vec<String> {
        self.raw(Key::Tags).map(read_tags).unwrap_or_default()
    }

    /// Replaces the field `key` with `raw`, or removes it when `raw` is `None`.
    ///
    /// The new field takes the place of the old one. A field that was not there goes
    /// after the last Holm field that comes before it, or at the top.
    pub(crate) fn set(&mut self, key: Key, raw: Option<String>) {
        let old = self.entries.iter().position(|e| e.key == Some(key));
        // Removing a later copy of the key never moves the first one.
        self.entries.retain(|e| e.key != Some(key));
        let Some(raw) = raw else {
            return;
        };
        let at = old.unwrap_or_else(|| {
            self.entries
                .iter()
                .rposition(|e| e.key.is_some_and(|k| k < key))
                .map_or(0, |i| i + 1)
        });
        self.entries.insert(at, Entry { raw, key: Some(key) });
    }

    pub(crate) fn write_id(id: &str, line_ending: &str) -> String {
        format!("id: {}{line_ending}", write_scalar(id))
    }

    pub(crate) fn write_tags(tags: &[String], line_ending: &str) -> String {
        let items = tags.iter().map(|tag| format!("  - {}{line_ending}", write_scalar(tag)));
        std::iter::once(format!("tags:{line_ending}")).chain(items).collect()
    }

    pub(crate) fn write_pinned(line_ending: &str) -> String {
        format!("pinned: true{line_ending}")
    }
}

impl std::fmt::Display for Frontmatter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.open)?;
        for entry in &self.entries {
            f.write_str(&entry.raw)?;
        }
        f.write_str(&self.close)
    }
}

/// The line without its line ending.
fn content(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

fn is_fence(line: &str, fence: &str) -> bool {
    content(line).trim_end_matches([' ', '\t']) == fence
}

/// Splits a top level line into its key and the text after the `:`.
fn split_key(line: &str) -> Option<(&str, &str)> {
    if line.starts_with(['#', '-', ' ', '\t']) {
        return None;
    }
    line.match_indices(':').find_map(|(i, _)| {
        let rest = &line[i + 1..];
        (rest.is_empty() || rest.starts_with([' ', '\t'])).then(|| (line[..i].trim_end(), rest))
    })
}

/// The text after the `:` on the first line of an entry.
fn value(raw: &str) -> &str {
    let first = content(raw.split_inclusive('\n').next().unwrap_or(""));
    split_key(first).map_or("", |(_, rest)| rest)
}

fn is_continuation(line: &str) -> bool {
    line.starts_with([' ', '\t']) || line == "-" || line.starts_with("- ")
}

fn entries(lines: &[&str]) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    // Blank lines join the entry above only when more of that entry follows.
    let mut blanks: Vec<&str> = Vec::new();
    for &line in lines {
        let text = content(line);
        if text.trim().is_empty() {
            blanks.push(line);
            continue;
        }
        if is_continuation(text)
            && let Some(last) = entries.last_mut()
        {
            for blank in blanks.drain(..) {
                last.raw.push_str(blank);
            }
            last.raw.push_str(line);
            continue;
        }
        entries.extend(blanks.drain(..).map(other));
        let key = split_key(text).and_then(|(key, _)| Key::from_name(key));
        entries.push(Entry {
            raw: line.to_owned(),
            key,
        });
    }
    entries.extend(blanks.into_iter().map(other));
    entries
}

fn other(line: &str) -> Entry {
    Entry {
        raw: line.to_owned(),
        key: None,
    }
}

/// Reads a flow list, a block list or one scalar.
fn read_tags(raw: &str) -> Vec<String> {
    let value = value(raw).trim_start();
    let below = raw.split_inclusive('\n').skip(1).map(content);
    if let Some(flow) = value.strip_prefix('[') {
        let joined: Vec<&str> = std::iter::once(flow).chain(below).collect();
        return clean_tags(read_flow_list(&joined.join(" ")));
    }
    if !value.is_empty() && !value.starts_with('#') {
        return clean_tags([read_scalar(value, false).0]);
    }
    let items = below.filter_map(|line| {
        let item = line.trim_start().strip_prefix('-')?;
        (item.is_empty() || item.starts_with([' ', '\t'])).then(|| read_scalar(item, false).0)
    });
    clean_tags(items)
}
