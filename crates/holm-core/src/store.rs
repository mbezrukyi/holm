//! Note files on disk: listing, reading, safe writes and new ids.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, Metadata, Permissions};
use std::io;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use holm_markdown::Note as Text;
use tempfile::NamedTempFile;

use crate::error::{Error, Result};
use crate::model::{Note, NoteId, NoteInfo};
use crate::relink::{Moved, relink};

/// The folder for pasted files at the root. The folder list skips it.
pub(crate) const ATTACHMENTS: &str = "attachments";
const NEW_NOTE: &str = "Untitled";
/// A new file gets these bits before the umask.
const NEW_FILE_MODE: u32 = 0o666;

/// The note files in one notes folder.
#[derive(Debug)]
pub(crate) struct Store {
    root: PathBuf,
}

/// What tells two versions of a file apart without a read: times and size.
///
/// The change time moves on every write, also when Holm keeps the old modification time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    pub(crate) modified: i64,
    pub(crate) changed: i64,
    pub(crate) size: i64,
}

impl Stamp {
    fn of(meta: &Metadata) -> Self {
        Self {
            modified: meta.mtime() * 1_000_000_000 + meta.mtime_nsec(),
            changed: meta.ctime() * 1_000_000_000 + meta.ctime_nsec(),
            size: i64::try_from(meta.size()).unwrap_or(i64::MAX),
        }
    }
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

    /// The notes folder, as an absolute path with no symlinks.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Every note file at or below `scope`, with its stamp. The empty path is the whole notes folder.
    ///
    /// `scope` can be a note, a folder, or a path with nothing there.
    /// A subfolder or a file that can't be read is left out with a log line.
    pub(crate) fn files(&self, scope: &Path) -> Result<Vec<(PathBuf, Stamp)>> {
        if !scope.as_os_str().is_empty() {
            match self.entry(scope) {
                Entry::Note(stamp) => return Ok(vec![(scope.to_owned(), stamp)]),
                Entry::Folder => {}
                Entry::Other | Entry::Missing => return Ok(Vec::new()),
            }
        }
        let mut files = Vec::new();
        self.walk(scope, &mut |rel, kind| {
            if kind == Kind::Note {
                match fs::metadata(self.root.join(rel)) {
                    Ok(meta) => files.push((rel.to_owned(), Stamp::of(&meta))),
                    Err(error) => tracing::warn!(%error, path = %rel.display(), "skipped a note"),
                }
            }
        })?;
        Ok(files)
    }

    /// What is at `rel` now. `rel` must be a plain path, as [`Store::check_folder`] allows.
    ///
    /// Like the folder walk, it does not follow symlinks.
    pub(crate) fn entry(&self, rel: &Path) -> Entry {
        let real_dir =
            |dir: &Path| fs::symlink_metadata(self.root.join(dir)).is_ok_and(|meta| meta.is_dir());
        let mut parents = rel.ancestors().skip(1);
        if !parents.all(real_dir) {
            return Entry::Missing;
        }
        match fs::symlink_metadata(self.root.join(rel)) {
            Ok(meta) if meta.is_dir() => Entry::Folder,
            Ok(meta) if meta.is_file() && is_md(rel) => Entry::Note(Stamp::of(&meta)),
            Ok(_) => Entry::Other,
            Err(_) => Entry::Missing,
        }
    }

    /// Whether a note file is at `rel`.
    pub(crate) fn exists(&self, rel: &Path) -> bool {
        self.note_path(rel).is_ok_and(|path| path.is_file())
    }

    /// Checks that `rel` is a note path inside the notes folder.
    pub(crate) fn check_note(rel: &Path) -> Result<()> {
        if is_md(rel) {
            plain_path(rel).map(drop)
        } else {
            Err(bad_path(rel))
        }
    }

    /// Checks that `rel` is a folder path inside the notes folder.
    pub(crate) fn check_folder(rel: &Path) -> Result<()> {
        plain_path(rel).map(drop)
    }

    /// Every subfolder at or below `scope`, sorted by path. It skips `attachments/` at the root.
    ///
    /// The empty path is the whole notes folder. `scope` can be a note, a folder,
    /// or a path with nothing there.
    pub(crate) fn folders(&self, scope: &Path) -> Result<Vec<PathBuf>> {
        let listed = |rel: &Path| rel != Path::new(ATTACHMENTS);
        let mut folders = Vec::new();
        if !scope.as_os_str().is_empty() {
            if plain_path(scope).is_err() || self.entry(scope) != Entry::Folder {
                return Ok(folders);
            }
            if listed(scope) {
                folders.push(scope.to_owned());
            }
        }
        self.walk(scope, &mut |rel, kind| {
            if kind == Kind::Folder && listed(rel) {
                folders.push(rel.to_owned());
            }
        })?;
        folders.sort();
        Ok(folders)
    }

    pub(crate) fn read(&self, rel: &Path) -> Result<(Note, Stamp)> {
        let path = self.note_path(rel)?;
        let (text, stamp) = read_text(&path)?;
        Ok((note(rel, &Text::parse(&text), stamp), stamp))
    }

    /// Changes the frontmatter of the note with `edit` and keeps its modification time.
    pub(crate) fn edit_fields(
        &self,
        rel: &Path,
        edit: impl FnOnce(&mut Text),
    ) -> Result<(Note, Stamp)> {
        let path = self.note_path(rel)?;
        let (old, stamp) = read_text(&path)?;
        let mut text = Text::parse(&old);
        edit(&mut text);
        write(&path, &old, &text.to_string(), Some(stamp))?;
        self.read(rel)
    }

    /// Writes `body` over the body of the note at `rel`. The frontmatter stays.
    ///
    /// It adds an id when the file has none. When the body stays the same,
    /// the file keeps its old modification time.
    pub(crate) fn save(&self, rel: &Path, body: &str) -> Result<(Note, Stamp)> {
        let path = self.note_path(rel)?;
        let (old, stamp) = read_text(&path)?;
        let mut text = Text::parse(&old);
        let mut body = body.to_owned();
        if !body.is_empty() && !body.ends_with('\n') {
            body.push_str(text.line_ending().as_str());
        }
        let body_changed = text.body() != body;
        text.set_body(&body);
        if text.id().is_none() {
            text.set_id(Some(NoteId::generate().as_str()));
        }
        write(
            &path,
            &old,
            &text.to_string(),
            (!body_changed).then_some(stamp),
        )?;
        self.read(rel)
    }

    /// Writes `note` again at its path after another tool deleted the file.
    /// It keeps the id, the tags and the pin. When a file is at the path again,
    /// it saves the body over it like [`Self::save`].
    pub(crate) fn restore(&self, note: &Note) -> Result<(Note, Stamp)> {
        let path = self.note_path(&note.info.path)?;
        let dir = path.parent().unwrap_or(&self.root);
        let mut text = Text::parse("");
        let id = note.info.id.clone().unwrap_or_else(NoteId::generate);
        text.set_id(Some(id.as_str()));
        text.set_tags(&note.info.tags);
        text.set_pinned(note.info.pinned);
        let file = temp_file(dir, text.to_string().as_bytes(), None, None)
            .map_err(|source| write_error(&path, source))?;
        match file.persist_noclobber(&path) {
            Ok(_) => sync_dir(dir).map_err(|source| write_error(&path, source))?,
            Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(write_error(&path, e.error)),
        }
        self.save(&note.info.path, &note.body)
    }

    /// Creates an empty note with a new id as `Untitled.md` in `folder`.
    /// On a clash, the name gets a number: `Untitled 2.md`.
    pub(crate) fn create(&self, folder: &Path) -> Result<(Note, Stamp)> {
        let dir = self.root.join(plain_path(folder)?);
        let mut text = Text::parse("");
        text.set_id(Some(NoteId::generate().as_str()));
        let file = temp_file(&dir, text.to_string().as_bytes(), None, None)
            .map_err(|source| write_error(&dir, source))?;
        let name = persist_new(file, &dir, OsStr::new(NEW_NOTE), ".md")?;
        self.read(&folder.join(name))
    }

    /// Copies `content` into a new file in `attachments/` at the root, and returns its path.
    ///
    /// The file is `stem` + `ext`, or `stem 2` + `ext` and so on on a clash.
    /// It makes the `attachments/` folder when it is missing.
    pub(crate) fn add_attachment(
        &self,
        stem: &OsStr,
        ext: &str,
        content: impl io::Read,
    ) -> Result<PathBuf> {
        let dir = self.root.join(ATTACHMENTS);
        match self.entry(Path::new(ATTACHMENTS)) {
            Entry::Folder => {}
            Entry::Missing => {
                match fs::create_dir(&dir) {
                    Err(e) if e.kind() != io::ErrorKind::AlreadyExists => {
                        return Err(write_error(&dir, e));
                    }
                    _ => {}
                }
                sync_dir(&self.root).map_err(|e| write_error(&dir, e))?;
            }
            Entry::Note(_) | Entry::Other => {
                return Err(write_error(&dir, io::ErrorKind::NotADirectory.into()));
            }
        }
        let file =
            temp_file(&dir, content, None, None).map_err(|source| write_error(&dir, source))?;
        let name = persist_new(file, &dir, stem, ext)?;
        Ok(Path::new(ATTACHMENTS).join(name))
    }

    /// Moves the note at `from` to `folder` as `<stem>.md`, or `<stem> 2.md` and so on
    /// on a clash, and returns its new path. It fixes the relative file links in the note.
    ///
    /// The note keeps its modification time and permissions.
    /// When `from` is one of the names to try, the note stays where it is.
    pub(crate) fn move_note(&self, from: &Path, folder: &Path, stem: &OsStr) -> Result<PathBuf> {
        let source = self.note_path(from)?;
        let dir = self.root.join(plain_path(folder)?);
        let (old, stamp) = read_text(&source)?;
        let text = relink(&old, &self.root, folder_of(from), folder, None).unwrap_or(old);
        let permissions = fs::metadata(&source)
            .map_err(|e| Error::Read {
                path: source.clone(),
                source: e,
            })?
            .permissions();
        let mut file = None;
        for name in numbered(stem, ".md") {
            let rel = folder.join(&name);
            if rel == from {
                return Ok(rel);
            }
            let temp = match file.take() {
                Some(temp) => temp,
                None => temp_file(
                    &dir,
                    text.as_bytes(),
                    Some(permissions.clone()),
                    Some(time(stamp.modified)),
                )
                .map_err(|source| write_error(&dir, source))?,
            };
            let path = dir.join(&name);
            match temp.persist_noclobber(&path) {
                Ok(_) => {
                    remove(&source)?;
                    sync_dir(&dir).map_err(|e| write_error(&path, e))?;
                    sync_dir(folder_abs(&source)).map_err(|e| write_error(&source, e))?;
                    return Ok(rel);
                }
                Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => file = Some(e.file),
                Err(e) => return Err(write_error(&path, e.error)),
            }
        }
        Err(write_error(&dir, io::ErrorKind::StorageFull.into()))
    }

    /// Creates the folder `name` in `parent`, or `name 2` and so on on a clash,
    /// and returns its path.
    pub(crate) fn create_folder(&self, parent: &Path, name: &str) -> Result<PathBuf> {
        let parent = plain_path(parent)?;
        for name in numbered(&folder_name(name)?, "") {
            let rel = parent.join(name);
            if rel == Path::new(ATTACHMENTS) {
                continue;
            }
            let path = self.root.join(&rel);
            match fs::create_dir(&path) {
                Ok(()) => {
                    sync_dir(folder_abs(&path)).map_err(|e| write_error(&path, e))?;
                    return Ok(rel);
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(write_error(&path, e)),
            }
        }
        Err(write_error(&self.root, io::ErrorKind::StorageFull.into()))
    }

    /// Moves the folder `from` to `parent` as `name`, or `name 2` and so on on a clash,
    /// and returns its new path.
    ///
    /// It fixes the relative file links in the notes inside. A note that can't be fixed
    /// keeps its links, with a log line.
    pub(crate) fn move_folder(&self, from: &Path, parent: &Path, name: &str) -> Result<PathBuf> {
        self.check_user_folder(from)?;
        let parent = plain_path(parent)?;
        if parent.starts_with(from) {
            return Err(bad_path(parent));
        }
        let source = self.root.join(from);
        for name in numbered(&folder_name(name)?, "") {
            let rel = parent.join(name);
            if rel == from {
                return Ok(rel);
            }
            if rel == Path::new(ATTACHMENTS) {
                continue;
            }
            let path = self.root.join(&rel);
            // An empty folder takes the name first. The rename then replaces it,
            // so it never replaces a folder that someone else made.
            match fs::create_dir(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(write_error(&path, e)),
            }
            if let Err(e) = fs::rename(&source, &path) {
                if let Err(error) = fs::remove_dir(&path) {
                    tracing::warn!(%error, path = %path.display(), "kept an empty folder");
                }
                return Err(write_error(&source, e));
            }
            sync_dir(folder_abs(&path)).map_err(|e| write_error(&path, e))?;
            sync_dir(folder_abs(&source)).map_err(|e| write_error(&source, e))?;
            self.relink_folder(Moved { from, to: &rel })?;
            return Ok(rel);
        }
        Err(write_error(&self.root, io::ErrorKind::StorageFull.into()))
    }

    /// Deletes the note file at `rel`. A note that is already gone is no error.
    pub(crate) fn delete_note(&self, rel: &Path) -> Result<()> {
        let path = self.note_path(rel)?;
        remove(&path)?;
        sync_dir(folder_abs(&path)).map_err(|e| write_error(&path, e))
    }

    /// Deletes the folder `rel` with everything in it. A folder that is already gone is no error.
    pub(crate) fn delete_folder(&self, rel: &Path) -> Result<()> {
        if self.entry(rel) == Entry::Missing {
            return Ok(());
        }
        self.check_user_folder(rel)?;
        let path = self.root.join(rel);
        fs::remove_dir_all(&path).map_err(|e| write_error(&path, e))?;
        sync_dir(folder_abs(&path)).map_err(|e| write_error(&path, e))
    }

    /// Deletes the file at `rel` when [`Store::is_attachment`] allows it.
    pub(crate) fn delete_attachment(&self, rel: &Path) -> Result<()> {
        if self.is_attachment(rel) {
            remove(&self.root.join(rel))?;
        }
        Ok(())
    }

    /// Whether `rel` is a file in `attachments/` that is not a note.
    /// A symlink counts as a file. No folder on the way may be a symlink.
    pub(crate) fn is_attachment(&self, rel: &Path) -> bool {
        rel.starts_with(ATTACHMENTS)
            && plain_path(rel).is_ok()
            && self.entry(rel) == Entry::Other
            && fs::symlink_metadata(self.root.join(rel)).is_ok_and(|meta| !meta.is_dir())
    }

    /// Checks that `rel` is a folder that the user may move or delete:
    /// a real folder that is not the root and not `attachments/`.
    fn check_user_folder(&self, rel: &Path) -> Result<()> {
        plain_path(rel)?;
        if rel.as_os_str().is_empty() || rel == Path::new(ATTACHMENTS) {
            return Err(bad_path(rel));
        }
        match self.entry(rel) {
            Entry::Folder => Ok(()),
            Entry::Missing => Err(Error::Read {
                path: self.root.join(rel),
                source: io::ErrorKind::NotFound.into(),
            }),
            Entry::Note(_) | Entry::Other => Err(Error::Read {
                path: self.root.join(rel),
                source: io::ErrorKind::NotADirectory.into(),
            }),
        }
    }

    /// Fixes the relative file links of each note inside a folder that moved.
    fn relink_folder(&self, moved: Moved<'_>) -> Result<()> {
        for (rel, _) in self.files(moved.to)? {
            let Ok(inside) = rel.strip_prefix(moved.to) else {
                continue;
            };
            let old_folder = moved.from.join(folder_of(inside));
            let path = self.root.join(&rel);
            let fixed = read_text(&path).and_then(|(old, stamp)| {
                match relink(&old, &self.root, &old_folder, folder_of(&rel), Some(moved)) {
                    Some(new) => write(&path, &old, &new, Some(stamp)),
                    None => Ok(()),
                }
            });
            if let Err(error) = fixed {
                tracing::warn!(%error, "kept the file links of a moved note");
            }
        }
        Ok(())
    }

    /// Calls `visit` with the relative path of each note and subfolder below the folder `start`.
    /// It skips hidden entries and symlinks, so it can't loop.
    fn walk(&self, start: &Path, visit: &mut dyn FnMut(&Path, Kind)) -> Result<()> {
        let mut dirs = vec![start.to_owned()];
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
        Self::check_note(rel).map(|()| self.root.join(rel))
    }
}

/// What is at a path in the notes folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    Note(Stamp),
    Folder,
    /// A file that is not a note, or a symlink.
    Other,
    Missing,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Note,
    Folder,
}

fn note(rel: &Path, text: &Text, stamp: Stamp) -> Note {
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
            preview: text.preview(),
            tags: text.tags(),
            pinned: text.pinned(),
            modified: time(stamp.modified),
        },
        body: text.body().to_owned(),
    }
}

/// The folder of a note or folder path. The root folder is the empty path.
pub(crate) fn folder_of(rel: &Path) -> &Path {
    rel.parent().unwrap_or(Path::new(""))
}

fn folder_abs(path: &Path) -> &Path {
    path.parent().unwrap_or(Path::new("/"))
}

/// `stem` + `ext`, then `stem 2` + `ext`, `stem 3` + `ext` and so on.
fn numbered(stem: &OsStr, ext: &str) -> impl Iterator<Item = OsString> {
    (1_u32..).map(move |n| {
        let mut name = stem.to_owned();
        if n > 1 {
            name.push(format!(" {n}"));
        }
        name.push(ext);
        name
    })
}

/// Gives `file` the first free name in `dir`: `stem` + `ext`, then `stem 2` + `ext` and so on.
/// It never replaces a file, and it returns the name.
fn persist_new(mut file: NamedTempFile, dir: &Path, stem: &OsStr, ext: &str) -> Result<OsString> {
    for name in numbered(stem, ext) {
        let path = dir.join(&name);
        match file.persist_noclobber(&path) {
            Ok(_) => {
                sync_dir(dir).map_err(|source| write_error(&path, source))?;
                return Ok(name);
            }
            Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => file = e.file,
            Err(e) => return Err(write_error(&path, e.error)),
        }
    }
    Err(write_error(dir, io::ErrorKind::StorageFull.into()))
}

/// A folder name from what the user typed, cleaned like a note file name.
fn folder_name(name: &str) -> Result<OsString> {
    let name = holm_markdown::file_stem(name.trim());
    if name.is_empty() {
        return Err(bad_path(Path::new(&name)));
    }
    Ok(name.into())
}

/// Deletes the file at `path`. A file that is already gone is no error.
fn remove(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(write_error(path, e)),
        _ => Ok(()),
    }
}

fn read_text(path: &Path) -> Result<(String, Stamp)> {
    let fail = |source| Error::Read {
        path: path.to_owned(),
        source,
    };
    let mut file = File::open(path).map_err(fail)?;
    let stamp = Stamp::of(&file.metadata().map_err(fail)?);
    let mut bytes = Vec::new();
    io::Read::read_to_end(&mut file, &mut bytes).map_err(fail)?;
    let text = String::from_utf8(bytes).map_err(|e| Error::NotUtf8 {
        path: path.to_owned(),
        source: e.utf8_error(),
    })?;
    Ok((text, stamp))
}

/// A time in nanoseconds since 1970 as a [`SystemTime`].
pub(crate) fn time(nanos: i64) -> SystemTime {
    let since = std::time::Duration::from_nanos(nanos.unsigned_abs());
    if nanos < 0 {
        SystemTime::UNIX_EPOCH - since
    } else {
        SystemTime::UNIX_EPOCH + since
    }
}

/// Replaces `old` with `new` at `path`, when they differ.
/// With `keep`, the file keeps that stamp's modification time.
fn write(path: &Path, old: &str, new: &str, keep: Option<Stamp>) -> Result<()> {
    if new == old {
        return Ok(());
    }
    let permissions = fs::metadata(path)
        .map_err(|source| write_error(path, source))?
        .permissions();
    let keep = keep.map(|stamp| time(stamp.modified));
    replace(path, new, permissions, keep).map_err(|source| write_error(path, source))
}

/// Replaces the file at `path` with `text` in one rename, so a crash never leaves half a note.
fn replace(
    path: &Path,
    text: &str,
    permissions: Permissions,
    modified: Option<SystemTime>,
) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    temp_file(dir, text.as_bytes(), Some(permissions), modified)?
        .persist(path)
        .map_err(|e| e.error)?;
    sync_dir(dir)
}

/// Writes `content` to a hidden temp file in `dir` and flushes it to the disk.
fn temp_file(
    dir: &Path,
    mut content: impl io::Read,
    permissions: Option<Permissions>,
    modified: Option<SystemTime>,
) -> io::Result<NamedTempFile> {
    let mut file = tempfile::Builder::new()
        .prefix(".holm-")
        .suffix(".tmp")
        .permissions(Permissions::from_mode(NEW_FILE_MODE))
        .tempfile_in(dir)?;
    io::copy(&mut content, &mut file)?;
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
