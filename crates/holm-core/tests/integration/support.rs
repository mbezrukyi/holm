#![expect(clippy::unwrap_used, reason = "test helpers")]

use std::fmt::Display;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use holm_core::{Core, Feed};
use tempfile::TempDir;

pub(crate) struct Folder {
    pub(crate) dir: TempDir,
    pub(crate) cache: TempDir,
    pub(crate) core: Core,
    /// Only for a folder from [`Folder::with_feed`].
    pub(crate) feed: Option<Feed>,
}

impl Folder {
    pub(crate) fn new(files: &[(&str, &str)]) -> Self {
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
    pub(crate) fn with_feed(files: &[(&str, &str)]) -> Self {
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
    pub(crate) fn feed<P: AsRef<Path>>(&self, paths: impl IntoIterator<Item = P>) {
        self.feed.as_ref().unwrap().changed(paths).unwrap();
    }

    pub(crate) fn path(&self, path: &str) -> PathBuf {
        self.dir.path().join(path)
    }

    pub(crate) fn write(&self, path: &str, text: &str) {
        write(self.dir.path(), path, text);
    }

    pub(crate) fn text(&self, path: &str) -> String {
        fs::read_to_string(self.path(path)).unwrap()
    }

    /// The sorted names in the folder `path`, hidden ones too.
    pub(crate) fn names(&self, path: &str) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.path(path))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// Sets the modification time to `secs` after 1970.
    pub(crate) fn set_time(&self, path: &str, secs: u64) -> SystemTime {
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
    pub(crate) fn reopen(&mut self) {
        self.core = Core::open(self.dir.path(), self.cache.path()).unwrap();
    }

    pub(crate) fn index_file(&self) -> PathBuf {
        self.cache.path().join("index.sqlite")
    }
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

pub(crate) fn paths<P: Display>(items: impl IntoIterator<Item = P>) -> Vec<String> {
    items.into_iter().map(|p| p.to_string()).collect()
}

pub(crate) fn path_buf(path: impl Display) -> PathBuf {
    PathBuf::from(path.to_string())
}
