#![expect(clippy::unwrap_used, reason = "test helpers")]

use std::fmt::Display;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use holm_core::{Core, Feed, FolderPath, NotePath, TagName};
use tempfile::TempDir;

pub struct Folder {
    pub dir: TempDir,
    pub cache: TempDir,
    pub core: Core,
    /// Only for a folder from [`Folder::with_feed`].
    pub feed: Option<Feed>,
}

impl Folder {
    pub fn new(files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            write(dir.path(), path, text);
        }
        let cache = tempfile::tempdir().unwrap();
        let core = Core::open(dir.path(), cache.path()).unwrap();
        Self {
            dir,
            cache,
            core,
            feed: None,
        }
    }

    /// A folder whose core sees outside changes only through [`Folder::feed`].
    pub fn with_feed(files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            write(dir.path(), path, text);
        }
        let cache = tempfile::tempdir().unwrap();
        let (core, feed) = Core::open_with_feed(dir.path(), cache.path()).unwrap();
        Self {
            dir,
            cache,
            core,
            feed: Some(feed),
        }
    }

    /// Tells the core of a [`Folder::with_feed`] that `paths` changed.
    pub fn feed<P: AsRef<Path>>(&self, paths: impl IntoIterator<Item = P>) {
        self.feed.as_ref().unwrap().changed(paths).unwrap();
    }

    pub fn path(&self, path: &str) -> PathBuf {
        self.dir.path().join(path)
    }

    pub fn write(&self, path: &str, text: &str) {
        write(self.dir.path(), path, text);
    }

    pub fn text(&self, path: &str) -> String {
        fs::read_to_string(self.path(path)).unwrap()
    }

    /// The sorted names in the folder `path`, hidden ones too.
    pub fn names(&self, path: &str) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.path(path))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// Sets the modification time to `secs` after 1970.
    pub fn set_time(&self, path: &str, secs: u64) -> SystemTime {
        let time = SystemTime::UNIX_EPOCH + Duration::from_secs(secs);
        fs::File::options()
            .write(true)
            .open(self.path(path))
            .unwrap()
            .set_modified(time)
            .unwrap();
        time
    }

    /// Opens the folder again with the same index, like a new start of Holm.
    pub fn reopen(&mut self) {
        self.core = Core::open(self.dir.path(), self.cache.path()).unwrap();
    }

    pub fn index_file(&self) -> PathBuf {
        self.cache.path().join("index.sqlite")
    }
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

pub fn paths<P: Display>(items: impl IntoIterator<Item = P>) -> Vec<String> {
    items.into_iter().map(|p| p.to_string()).collect()
}

pub fn note_path(path: &str) -> NotePath {
    NotePath::new(path).unwrap()
}

pub fn folder_path(path: &str) -> FolderPath {
    FolderPath::new(path).unwrap()
}

pub fn tag_name(name: &str) -> TagName {
    TagName::new(name).unwrap()
}

pub fn path_buf(path: impl Display) -> PathBuf {
    PathBuf::from(path.to_string())
}
