use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui_kit::Size;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileKind {
    /// The size in pixels.
    Image(Size<usize>),
    Folder,
    /// The size in bytes.
    File(u64),
    Missing,
}

impl FileKind {
    /// Reads the disk. An image file gives its size from its header.
    fn of(path: &Path) -> Self {
        let Ok(meta) = path.metadata() else {
            return Self::Missing;
        };
        if meta.is_dir() {
            return Self::Folder;
        }
        imagesize::size(path).map_or(Self::File(meta.len()), |found| {
            Self::Image(Size {
                width: found.width,
                height: found.height,
            })
        })
    }
}

/// A relative link target starts at the folder of the note. It reads the kind of each path from the disk once, until
/// the folder changes.
#[derive(Default)]
pub(crate) struct LinkedFiles {
    folder: Option<PathBuf>,
    kinds: HashMap<PathBuf, FileKind>,
}

impl LinkedFiles {
    pub(crate) fn set_folder(&mut self, folder: Option<PathBuf>) {
        self.folder = folder;
        self.kinds.clear();
    }

    pub(crate) fn path(&self, target: &str) -> PathBuf {
        match &self.folder {
            Some(folder) => folder.join(target),
            None => PathBuf::from(target),
        }
    }

    pub(crate) fn kind(&mut self, target: &str) -> FileKind {
        *self
            .kinds
            .entry(self.path(target))
            .or_insert_with_key(|path| FileKind::of(path))
    }
}

#[cfg(test)]
mod tests;
