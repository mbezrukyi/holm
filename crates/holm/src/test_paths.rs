//! Typed note and folder paths from text, for tests.

#![allow(clippy::unwrap_used, reason = "tests")]

use holm_core::{FolderPath, NotePath};

pub(crate) fn note_path(path: &str) -> NotePath {
    NotePath::new(path).unwrap()
}

pub(crate) fn folder_path(path: &str) -> FolderPath {
    FolderPath::new(path).unwrap()
}
