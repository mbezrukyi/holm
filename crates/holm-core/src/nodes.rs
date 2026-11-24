use crate::paths::{FolderPath, NotePath};

/// A note or a folder that the core brings up to date with the disk. A folder node holds everything at its path and
/// below it.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Node {
    Note(NotePath),
    Folder(FolderPath),
}

/// Nodes in which no node holds another, each once, sorted.
#[derive(Debug)]
pub(crate) struct Nodes(Vec<Node>);

impl Node {
    pub(crate) fn holds(&self, other: &Self) -> bool {
        match (self, other) {
            (_, Self::Note(note)) => self.holds_note(note),
            (Self::Folder(node), Self::Folder(folder)) => node == folder || folder.is_below(node),
            (Self::Note(_), Self::Folder(_)) => false,
        }
    }

    pub(crate) fn holds_note(&self, path: &NotePath) -> bool {
        match self {
            Self::Note(note) => note == path,
            Self::Folder(folder) => path.is_below(folder),
        }
    }
}

impl Nodes {
    pub(crate) fn new(nodes: impl IntoIterator<Item = Node>) -> Self {
        let mut nodes: Vec<Node> = nodes.into_iter().collect();
        nodes.sort();
        nodes.dedup();
        let all = nodes.clone();
        nodes.retain(|node| !all.iter().any(|other| other != node && other.holds(node)));
        Self(nodes)
    }

    pub(crate) fn root() -> Self {
        Self(vec![Node::Folder(FolderPath::root())])
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<'a> IntoIterator for &'a Nodes {
    type Item = &'a Node;
    type IntoIter = std::slice::Iter<'a, Node>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

#[cfg(test)]
mod tests;
