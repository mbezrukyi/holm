//! The attachment folder `attachments/` at the root, where Holm puts pasted files.
//!
//! Only the folder with exactly this name is the attachments folder. A folder like
//! `Attachments/` is a user folder. A new folder name takes the name in any case, like a name
//! clash.

use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

use super::{AtPath, Store, remove, sync_dir, temp_file, write_error};
use crate::error::{Error, Result};
use crate::paths::{FolderPath, Name, NotePath};

const ATTACHMENTS: &str = "attachments";
/// The stem of a pasted image before its hash.
const PASTED_IMAGE: &str = "Pasted image";

/// Longest file stem in bytes. It leaves room for the hash and the extension of an
/// attachment under [`MAX_NAME_BYTES`](crate::paths::MAX_NAME_BYTES).
const MAX_STEM_BYTES: usize = 240;
/// The bytes of the content hash in an attachment name: 8 hex digits.
const HASH_BYTES: usize = 4;

/// The attachments folder. It is always `Some`.
fn folder() -> Option<FolderPath> {
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
fn holds(folder: &FolderPath) -> bool {
    folder.as_ref().starts_with(ATTACHMENTS)
}

impl Store {
    /// Copies the file at `file` into the attachment folder, and returns its path from the
    /// folder of `note`, for a link.
    pub(crate) fn attach_file(&self, note: &NotePath, file: &Path) -> Result<PathBuf> {
        let read_error = |source| Error::Read {
            path: file.to_owned(),
            source,
        };
        let source = File::open(file).map_err(read_error)?;
        if !source.metadata().map_err(read_error)?.is_file() {
            return Err(read_error(io::ErrorKind::IsADirectory.into()));
        }
        let stem = file.file_stem().unwrap_or_default().to_string_lossy();
        let ext = file.extension().map(OsStr::to_string_lossy);
        self.attach(note, &stem, ext.as_deref(), source)
    }

    /// Saves a pasted image in the format `extension` into the attachment folder, and returns
    /// its path from the folder of `note`, for a link.
    pub(crate) fn paste_image(&self, note: &NotePath, extension: &str, bytes: &[u8]) -> Result<PathBuf> {
        self.attach(note, PASTED_IMAGE, Some(extension), bytes)
    }

    fn attach(&self, note: &NotePath, stem: &str, ext: Option<&str>, content: impl io::Read) -> Result<PathBuf> {
        let file = self.add_attachment(stem, ext, content)?;
        Ok(crate::links::link(&self.root, note, &file))
    }

    /// Copies `content` into the attachment folder as `<stem>-<hash>.<ext>`, and returns its
    /// absolute path. The hash is a short hash of the content, so a name never clashes.
    /// With no `ext`, the name has no `.` either.
    ///
    /// The stem and the extension keep their text, but `/` and a leading `.` become `-`.
    /// A very long stem or extension stops at [`MAX_STEM_BYTES`].
    ///
    /// When a file with that name is there, it keeps that file and drops the new copy.
    /// It makes the folder when it is missing.
    fn add_attachment(&self, stem: &str, ext: Option<&str>, content: impl io::Read) -> Result<PathBuf> {
        let Some(folder) = folder() else {
            return Err(write_error(&self.root, io::ErrorKind::InvalidInput.into()));
        };
        let dir = self.root.join(&folder);
        match self.folder_at(&folder) {
            AtPath::Folder(_) => {}
            AtPath::Missing => {
                match fs::create_dir(&dir) {
                    Err(e) if e.kind() != io::ErrorKind::AlreadyExists => {
                        return Err(write_error(&dir, e));
                    }
                    _ => {}
                }
                sync_dir(&self.root).map_err(|e| write_error(&dir, e))?;
            }
            _ => {
                return Err(write_error(&dir, io::ErrorKind::NotADirectory.into()));
            }
        }
        let mut hashed = Hashed {
            inner: content,
            hash: Sha256::new(),
        };
        let file = temp_file(&dir, &mut hashed, None, None).map_err(|source| write_error(&dir, source))?;
        let hash = hashed.hash.finalize();
        let mut name = file_stem(stem);
        name.push('-');
        for byte in &hash[..HASH_BYTES] {
            let _ = write!(name, "{byte:02x}");
        }
        if let Some(ext) = ext {
            name.push('.');
            name.push_str(&file_stem(ext));
        }
        let path = dir.join(&name);
        match file.persist_noclobber(&path) {
            Ok(_) => sync_dir(&dir).map_err(|e| write_error(&path, e))?,
            Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(write_error(&path, e.error)),
        }
        Ok(path)
    }

    /// The paths in `targets` that are attachments, in their order.
    /// Each path is relative to the notes folder.
    pub(crate) fn attachments(&self, targets: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
        targets
            .into_iter()
            .filter(|target| self.is_attachment(target))
            .collect()
    }

    /// Deletes the file at `rel` when it is an attachment.
    pub(crate) fn delete_attachment(&self, rel: &Path) -> Result<()> {
        if self.is_attachment(rel) {
            remove(&self.root.join(rel))?;
        }
        Ok(())
    }

    /// Whether `rel`, a path relative to the notes folder, is a file in the attachment folder
    /// that is not a note. A symlink counts as a file. No folder on the way may be a symlink.
    fn is_attachment(&self, rel: &Path) -> bool {
        let Some((folder, name)) = FolderPath::split_raw(rel) else {
            return false;
        };
        holds(&folder)
            && self.at(&folder, &name) == AtPath::Other
            && fs::symlink_metadata(self.root.join(rel)).is_ok_and(|meta| !meta.is_dir())
    }
}

/// A file stem from any text, like the name of a pasted file.
///
/// It keeps the text as it is, but replaces `/` and a leading `.` with `-`.
/// It also cuts a very long text to [`MAX_STEM_BYTES`].
fn file_stem(text: &str) -> String {
    let mut stem = text.replace('/', "-");
    if stem.starts_with('.') {
        stem.replace_range(..1, "-");
    }
    if stem.len() > MAX_STEM_BYTES {
        let mut end = MAX_STEM_BYTES;
        while !stem.is_char_boundary(end) {
            end -= 1;
        }
        stem.truncate(end);
    }
    stem
}

/// A reader that hashes what it reads.
struct Hashed<R> {
    inner: R,
    hash: Sha256,
}

impl<R: io::Read> io::Read for Hashed<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.hash.update(&buf[..read]);
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder_path(path: &str) -> FolderPath {
        FolderPath::new(path).unwrap()
    }

    fn name(name: &str) -> Name {
        Name::typed(name).unwrap()
    }

    #[test]
    fn file_stems() {
        assert_eq!(file_stem("Plans for 2026"), "Plans for 2026");
        assert_eq!(file_stem("a/b/c"), "a-b-c");
        assert_eq!(file_stem("..dots"), "-.dots");
        assert_eq!(file_stem("Ünïcode: ok?"), "Ünïcode: ok?");
    }

    #[test]
    fn a_long_file_stem_stops_at_a_char() {
        let text = "é".repeat(200);
        let stem = file_stem(&text);
        assert_eq!(stem.len(), MAX_STEM_BYTES);
        assert!(text.starts_with(&stem));
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
        assert!(holds(&folder_path("attachments")));
        assert!(holds(&folder_path("attachments/2026")));
        assert!(!holds(&folder_path("Attachments")));
        assert!(!holds(&folder_path("attachments2")));
        assert!(!holds(&folder_path("work/attachments")));
        assert!(!holds(&FolderPath::root()));
    }

    #[test]
    fn only_the_exact_folder_is_the_attachments_folder() {
        assert!(is(&folder_path("attachments")));
        assert!(!is(&folder_path("Attachments")));
        assert!(!is(&folder_path("attachments/2026")));
        assert!(!is(&FolderPath::root()));
    }
}
