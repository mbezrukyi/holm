//! The files that the links of a note point to.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui_kit::Size;

/// What a linked path is on the disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileKind {
    /// An image, with its size in pixels.
    Image(Size<usize>),
    Folder,
    /// Any other file, with its size in bytes.
    File(u64),
    Missing,
}

impl FileKind {
    /// Reads the kind of `path` from the disk. An image file gives its size from its header.
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

/// The linked files of one note. A relative link target starts at the folder of the note.
/// It reads the kind of each path from the disk once, until the folder changes.
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

    /// The path of a link target.
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
mod tests {
    use std::fs;

    use super::*;

    fn data(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(name)
    }

    #[test]
    fn the_kind_of_a_path_comes_from_the_disk() {
        assert_eq!(FileKind::of(&data("data")), FileKind::Folder);
        let path = data("data/chunk.md");
        let size = path.metadata().map(|meta| meta.len()).unwrap_or_default();
        assert_eq!(FileKind::of(&path), FileKind::File(size));
        assert_eq!(FileKind::of(&data("data/gone.pdf")), FileKind::Missing);
        assert!(matches!(FileKind::of(&data("data/cat.png")), FileKind::Image(_)));
    }

    #[test]
    fn a_relative_target_starts_at_the_folder() {
        let mut files = LinkedFiles::default();
        assert_eq!(files.path("docs/a.pdf"), Path::new("docs/a.pdf"));
        files.set_folder(Some(data("data")));
        assert_eq!(files.path("cat.png"), data("data/cat.png"));
        assert!(matches!(files.kind("cat.png"), FileKind::Image(_)));
        assert_eq!(files.path("/abs/a.pdf"), Path::new("/abs/a.pdf"));
    }

    #[test]
    fn the_disk_is_read_once_for_each_path() -> std::io::Result<()> {
        let folder = std::env::temp_dir().join(format!("holm-linked-{}", std::process::id()));
        fs::create_dir_all(&folder)?;
        fs::write(folder.join("a.txt"), "abc")?;
        let mut files = LinkedFiles::default();
        files.set_folder(Some(folder.clone()));
        assert_eq!(files.kind("a.txt"), FileKind::File(3));
        fs::remove_dir_all(&folder)?;
        assert_eq!(files.kind("a.txt"), FileKind::File(3));
        files.set_folder(Some(folder));
        assert_eq!(files.kind("a.txt"), FileKind::Missing);
        Ok(())
    }
}
