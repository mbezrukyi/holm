//! Helpers for the tests and benches of this crate and of the crates that use it.

#![expect(clippy::unwrap_used, reason = "test helpers")]

pub mod sample;

use crate::{FolderPath, NotePath, TagName};

pub fn note_path(path: &str) -> NotePath {
    NotePath::new(path).unwrap()
}

pub fn folder_path(path: &str) -> FolderPath {
    FolderPath::new(path).unwrap()
}

pub fn tag_name(name: &str) -> TagName {
    TagName::new(name).unwrap()
}
