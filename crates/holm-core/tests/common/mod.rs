//! A notes folder and an index folder in temp folders, with a `Core` on them.

#![allow(dead_code, reason = "each test file uses only some helpers")]
#![allow(clippy::unwrap_used, reason = "tests")]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use holm_core::Core;
use tempfile::TempDir;

pub(crate) struct Folder {
    pub(crate) dir: TempDir,
    pub(crate) cache: TempDir,
    pub(crate) core: Core,
}

impl Folder {
    pub(crate) fn new(files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            write(dir.path(), path, text);
        }
        let cache = tempfile::tempdir().unwrap();
        let core = Core::open(dir.path(), cache.path()).unwrap();
        Self { dir, cache, core }
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

    /// The names in the notes folder itself.
    pub(crate) fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// Opens the folder again with the same index, like a new start of Holm.
    pub(crate) fn reopen(&mut self) {
        self.core = Core::open(self.dir.path(), self.cache.path()).unwrap();
    }

    /// The index file.
    pub(crate) fn index_file(&self) -> PathBuf {
        self.cache.path().join("index.sqlite")
    }
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

pub(crate) fn paths(items: impl IntoIterator<Item = PathBuf>) -> Vec<String> {
    items
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}
