//! Typed note and folder paths and tag names from text, for tests.

#![allow(clippy::unwrap_used, reason = "tests")]

use holm_core::{FolderPath, NotePath, TagName};

pub(crate) fn note_path(path: &str) -> NotePath {
    NotePath::new(path).unwrap()
}

pub(crate) fn folder_path(path: &str) -> FolderPath {
    FolderPath::new(path).unwrap()
}

pub(crate) fn tag_name(name: &str) -> TagName {
    TagName::new(name).unwrap()
}
