//! Note files on disk: listing, reading, safe writes and new ids.

use std::ffi::OsStr;
use std::fs::{self, File, Permissions};
use std::io::{self, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use holm_markdown::Note as Text;
use tempfile::NamedTempFile;

use crate::error::{Error, Result};
use crate::model::{Note, NoteId, NoteInfo};

/// The folder for pasted files. The folder list skips it.
const ATTACHMENTS: &str = "attachments";
const NEW_NOTE: &str = "Untitled";
/// A new file gets these bits before the umask.
const NEW_FILE_MODE: u32 = 0o666;

/// The note files in one notes folder.
#[derive(Debug)]
pub(crate) struct Store {
    root: PathBuf,
}

impl Store {
    pub(crate) fn open(root: &Path) -> Result<Self> {
        let fail = |source| Error::NotesFolder {
            path: root.to_owned(),
            source,
        };
        let root = fs::canonicalize(root).map_err(fail)?;
        if !root.is_dir() {
            return Err(fail(io::ErrorKind::NotADirectory.into()));
        }
        Ok(Self { root })
    }

    /// Every note in the notes folder and in its subfolders, sorted by path.
    ///
    /// A note or subfolder that can't be read is left out with a log line.
    pub(crate) fn notes(&self) -> Result<Vec<NoteInfo>> {
        let mut notes = Vec::new();
        self.walk(&mut |rel, kind| {
            if kind == Kind::Note {
                match self.read(rel) {
                    Ok(note) => notes.push(note.info),
                    Err(error) => tracing::warn!(%error, "skipped a note"),
                }
            }
        })?;
        notes.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(notes)
    }

    /// Every subfolder, sorted by path. It skips `attachments/` at the root.
    pub(crate) fn folders(&self) -> Result<Vec<PathBuf>> {
        let mut folders = Vec::new();
        self.walk(&mut |rel, kind| {
            if kind == Kind::Folder && rel != Path::new(ATTACHMENTS) {
                folders.push(rel.to_owned());
            }
        })?;
        folders.sort();
        Ok(folders)
    }

    pub(crate) fn read(&self, rel: &Path) -> Result<Note> {
        let path = self.note_path(rel)?;
        let (text, modified) = read_text(&path)?;
        Ok(note(rel, &Text::parse(&text), modified))
    }

    /// Writes the body, tags and pin of `note` over its file.
    ///
    /// It adds an id when the file has none. When the body stays the same,
    /// the file keeps its old modification time.
    pub(crate) fn save(&self, note: &Note) -> Result<Note> {
        let rel = &note.info.path;
        let path = self.note_path(rel)?;
        let (old, modified) = read_text(&path)?;
        let mut text = Text::parse(&old);
        let mut body = note.body.clone();
        if !body.is_empty() && !body.ends_with('\n') {
            body.push_str(text.line_ending().as_str());
        }
        let body_changed = text.body() != body;
        text.set_body(&body);
        if text.tags() != note.info.tags {
            text.set_tags(&note.info.tags);
        }
        if text.pinned() != note.info.pinned {
            text.set_pinned(note.info.pinned);
        }
        if text.id().is_none() {
            text.set_id(Some(NoteId::generate().as_str()));
        }
        let new = text.to_string();
        if new != old {
            let permissions = fs::metadata(&path)
                .map_err(|source| write_error(&path, source))?
                .permissions();
            let keep = (!body_changed).then_some(modified);
            replace(&path, &new, permissions, keep).map_err(|source| write_error(&path, source))?;
        }
        self.read(rel)
    }

    /// Creates an empty note with a new id as `Untitled.md` in `folder`.
    /// On a clash, the name gets a number: `Untitled 2.md`.
    pub(crate) fn create(&self, folder: &Path) -> Result<Note> {
        let dir = self.root.join(plain_path(folder)?);
        let mut text = Text::parse("");
        text.set_id(Some(NoteId::generate().as_str()));
        let mut file = temp_file(&dir, &text.to_string(), None, None)
            .map_err(|source| write_error(&dir, source))?;
        for n in 1_u32.. {
            let name = if n == 1 {
                format!("{NEW_NOTE}.md")
            } else {
                format!("{NEW_NOTE} {n}.md")
            };
            let path = dir.join(&name);
            match file.persist_noclobber(&path) {
                Ok(_) => {
                    sync_dir(&dir).map_err(|source| write_error(&path, source))?;
                    return self.read(&folder.join(name));
                }
                Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => file = e.file,
                Err(e) => return Err(write_error(&path, e.error)),
            }
        }
        Err(write_error(&dir, io::ErrorKind::StorageFull.into()))
    }

    /// Calls `visit` with the relative path of each note and subfolder.
    /// It skips hidden entries and symlinks, so it can't loop.
    fn walk(&self, visit: &mut dyn FnMut(&Path, Kind)) -> Result<()> {
        let mut dirs = vec![PathBuf::new()];
        while let Some(dir) = dirs.pop() {
            let path = self.root.join(&dir);
            let entries = match fs::read_dir(&path) {
                Ok(entries) => entries,
                Err(source) if dir.as_os_str().is_empty() => {
                    return Err(Error::Read { path, source });
                }
                Err(error) => {
                    tracing::warn!(%error, path = %path.display(), "skipped a folder");
                    continue;
                }
            };
            for entry in entries {
                let Ok(entry) = entry else { continue };
                let name = entry.file_name();
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if is_hidden(&name) {
                    continue;
                }
                let rel = dir.join(&name);
                if kind.is_dir() {
                    visit(&rel, Kind::Folder);
                    dirs.push(rel);
                } else if kind.is_file() && is_md(&rel) {
                    visit(&rel, Kind::Note);
                }
            }
        }
        Ok(())
    }

    /// The absolute path of a note, after a check of the relative path.
    fn note_path(&self, rel: &Path) -> Result<PathBuf> {
        if is_md(rel) {
            plain_path(rel).map(|rel| self.root.join(rel))
        } else {
            Err(bad_path(rel))
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Note,
    Folder,
}

fn note(rel: &Path, text: &Text, modified: SystemTime) -> Note {
    let title = text.title().unwrap_or_else(|| {
        rel.file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    Note {
        info: NoteInfo {
            path: rel.to_owned(),
            id: text.id().map(NoteId::from_field),
            title,
            tags: text.tags(),
            pinned: text.pinned(),
            modified,
        },
        body: text.body().to_owned(),
    }
}

fn read_text(path: &Path) -> Result<(String, SystemTime)> {
    let fail = |source| Error::Read {
        path: path.to_owned(),
        source,
    };
    let mut file = File::open(path).map_err(fail)?;
    let modified = file.metadata().and_then(|m| m.modified()).map_err(fail)?;
    let mut bytes = Vec::new();
    io::Read::read_to_end(&mut file, &mut bytes).map_err(fail)?;
    let text = String::from_utf8(bytes).map_err(|e| Error::NotUtf8 {
        path: path.to_owned(),
        source: e.utf8_error(),
    })?;
    Ok((text, modified))
}

/// Replaces the file at `path` with `text` in one rename, so a crash never leaves half a note.
fn replace(
    path: &Path,
    text: &str,
    permissions: Permissions,
    modified: Option<SystemTime>,
) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    temp_file(dir, text, Some(permissions), modified)?
        .persist(path)
        .map_err(|e| e.error)?;
    sync_dir(dir)
}

/// Writes `text` to a hidden temp file in `dir` and flushes it to the disk.
fn temp_file(
    dir: &Path,
    text: &str,
    permissions: Option<Permissions>,
    modified: Option<SystemTime>,
) -> io::Result<NamedTempFile> {
    let mut file = tempfile::Builder::new()
        .prefix(".holm-")
        .suffix(".tmp")
        .permissions(Permissions::from_mode(NEW_FILE_MODE))
        .tempfile_in(dir)?;
    file.write_all(text.as_bytes())?;
    if let Some(permissions) = permissions {
        file.as_file().set_permissions(permissions)?;
    }
    if let Some(modified) = modified {
        file.as_file().set_modified(modified)?;
    }
    file.as_file().sync_all()?;
    Ok(file)
}

/// Makes a rename in `dir` last after a crash.
fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Checks that `rel` is a plain relative path with no hidden part.
fn plain_path(rel: &Path) -> Result<&Path> {
    let plain = rel.components().all(|part| match part {
        Component::Normal(name) => !is_hidden(name),
        _ => false,
    });
    if plain { Ok(rel) } else { Err(bad_path(rel)) }
}

fn is_hidden(name: &OsStr) -> bool {
    name.as_encoded_bytes().starts_with(b".")
}

fn is_md(path: &Path) -> bool {
    path.extension() == Some(OsStr::new("md"))
}

fn bad_path(rel: &Path) -> Error {
    Error::BadPath {
        path: rel.to_owned(),
    }
}

fn write_error(path: &Path, source: io::Error) -> Error {
    Error::Write {
        path: path.to_owned(),
        source,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use std::time::Duration;

    use super::*;

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn replace_writes_the_new_text_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.md");
        fs::write(&path, "old").unwrap();

        replace(&path, "new", Permissions::from_mode(0o644), None).unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(names(dir.path()), ["a.md"]);
    }

    #[test]
    fn replace_sets_the_permissions_and_the_modification_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.md");
        fs::write(&path, "old").unwrap();
        let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);

        replace(&path, "new", Permissions::from_mode(0o640), Some(time)).unwrap();

        let meta = fs::metadata(&path).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o640);
        assert_eq!(meta.modified().unwrap(), time);
    }

    #[test]
    fn a_failed_replace_keeps_the_old_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.md");
        fs::write(&path, "old").unwrap();
        // A folder can't take the place of a file, so the rename fails.
        let target = dir.path().join("folder.md");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("x"), "").unwrap();

        assert!(replace(&target, "new", Permissions::from_mode(0o644), None).is_err());

        assert_eq!(fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(names(dir.path()), ["a.md", "folder.md"]);
    }
}
