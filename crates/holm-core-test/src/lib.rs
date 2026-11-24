#![expect(clippy::unwrap_used, reason = "test helpers")]

use holm_core::{FolderPath, NotePath, TagName};

pub fn note_path(path: &str) -> NotePath {
    NotePath::new(path).unwrap()
}

pub fn folder_path(path: &str) -> FolderPath {
    FolderPath::new(path).unwrap()
}

pub fn tag_name(name: &str) -> TagName {
    TagName::new(name).unwrap()
}
