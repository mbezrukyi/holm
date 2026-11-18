//! The attachment folder `attachments/` at the root, where Holm puts pasted files.
//!
//! Only the folder with exactly this name is the attachments folder. A folder like
//! `Attachments/` is a user folder. A new folder name takes the name in any case, like a name
//! clash.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;

use crate::paths::{FolderPath, Name};

const ATTACHMENTS: &str = "attachments";

/// The attachments folder. It is always `Some`.
pub(crate) fn folder() -> Option<FolderPath> {
    FolderPath::new(ATTACHMENTS)
}

/// Whether `folder` is the attachments folder itself.
pub(crate) fn is(folder: &FolderPath) -> bool {
    folder.as_ref() == Path::new(ATTACHMENTS)
}

/// Whether a new folder `name` in `parent` takes the name of the attachments folder.
pub(crate) fn reserves(parent: &FolderPath, name: &Name) -> bool {
    parent.is_root() && name.as_os_str().eq_ignore_ascii_case(ATTACHMENTS)
}

/// Whether `folder` is the attachments folder or a folder inside it.
pub(crate) fn holds(folder: &FolderPath) -> bool {
    folder.as_ref().starts_with(ATTACHMENTS)
}

/// The bytes that each path inside the attachments folder starts with: `attachments/`.
pub(crate) fn prefix() -> Vec<u8> {
    let mut prefix = OsStr::new(ATTACHMENTS).as_bytes().to_vec();
    prefix.push(b'/');
    prefix
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    fn folder_path(path: &str) -> FolderPath {
        FolderPath::new(path).unwrap()
    }

    fn name(name: &str) -> Name {
        Name::typed(name).unwrap()
    }

    #[test]
    fn a_new_name_takes_the_folder_in_any_case_only_at_the_root() {
        let root = FolderPath::root();
        assert!(reserves(&root, &name("attachments")));
        assert!(reserves(&root, &name("Attachments")));
        assert!(reserves(&root, &name("ATTACHMENTS")));
        assert!(!reserves(&root, &name("attachments2")));
        assert!(!reserves(&folder_path("work"), &name("attachments")));
    }

    #[test]
    fn only_the_exact_folder_holds_attachments() {
        assert_eq!(folder(), Some(folder_path("attachments")));
        assert!(is(&folder_path("attachments")));
        assert!(!is(&folder_path("Attachments")));
        assert!(!is(&folder_path("attachments/2026")));
        assert!(!is(&FolderPath::root()));
        assert!(holds(&folder_path("attachments")));
        assert!(holds(&folder_path("attachments/2026")));
        assert!(!holds(&folder_path("Attachments")));
        assert!(!holds(&folder_path("attachments2")));
        assert!(!holds(&folder_path("work/attachments")));
        assert!(!holds(&FolderPath::root()));
        assert_eq!(prefix(), b"attachments/");
    }
}
