//! Note files on disk: listing, reading, safe writes and new ids.

use std::ffi::OsStr;
use std::fs::{self, File, Metadata, Permissions};
use std::io;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use holm_markdown::Note as Text;
use rustix::fs::{Mode, OFlags};
use tempfile::NamedTempFile;

use sha2::{Digest as _, Sha256};

use crate::error::{Error, Result};
use crate::model::{Note, NoteId, NoteInfo};
use crate::paths::{ATTACHMENTS, FolderPath, NotePath, Scope};
use crate::relink::{Moved, relink};

/// Longest file stem in bytes. It leaves room for the hash and the extension of an
/// attachment under [`MAX_NAME_BYTES`](crate::paths::MAX_NAME_BYTES).
const MAX_STEM_BYTES: usize = 240;
/// The bytes of the content hash in an attachment name: 8 hex digits.
const HASH_BYTES: usize = 4;
/// A new file gets these bits before the umask.
const NEW_FILE_MODE: u32 = 0o666;
const TEMP_PREFIX: &str = ".holm-";
const TEMP_SUFFIX: &str = ".tmp";

/// The note files in one notes folder.
#[derive(Debug)]
pub(crate) struct Store {
    root: PathBuf,
}

/// What tells two versions of a file apart without a read: times and size.
///
/// The change time moves on every write, also when Holm keeps the old modification time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    /// Opens the notes folder and deletes the temp files that a crash left in it.
    pub(crate) fn open(root: &Path) -> Result<Self> {
        let fail = |source| Error::NotesFolder {
            path: root.to_owned(),
            source,
        };
        let root = fs::canonicalize(root).map_err(fail)?;
        if !root.is_dir() {
            return Err(fail(io::ErrorKind::NotADirectory.into()));
        }
        let store = Self { root };
        store.walk(&FolderPath::root(), &mut |found| {
            if let Found::Temp(path) = found
                && let Err(error) = fs::remove_file(&path)
            {
                tracing::warn!(%error, path = %path.display(), "kept a temp file");
            }
        })?;
        Ok(store)
    }

    /// The notes folder, as an absolute path with no symlinks.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Every note file at or below `scope`, with its stamp.
    ///
    /// `scope` can be a note, a folder, or a path with nothing there.
    /// A subfolder or a file that can't be read is left out with a log line.
    pub(crate) fn files(&self, scope: &Scope) -> Result<Vec<(NotePath, Stamp)>> {
        match self.entry(scope) {
            Entry::Note(note, stamp) => return Ok(vec![(note, stamp)]),
            Entry::Folder => {}
            Entry::Other | Entry::Missing => return Ok(Vec::new()),
        }
        let mut files = Vec::new();
        self.walk(&scope.folder(), &mut |found| {
            if let Found::Note(rel) = found {
                match fs::metadata(self.root.join(&rel)) {
                    Ok(meta) => files.push((rel, Stamp::of(&meta))),
                    Err(error) => tracing::warn!(%error, path = %rel, "skipped a note"),
                }
            }
        })?;
        Ok(files)
    }

    /// What is at `scope` now. The root is a folder.
    ///
    /// Like the folder walk, it does not follow symlinks.
    pub(crate) fn entry(&self, scope: &Scope) -> Entry {
        let real_dir = |dir: FolderPath| {
            fs::symlink_metadata(self.root.join(dir)).is_ok_and(|meta| meta.is_dir())
        };
        if !scope.folder().parents().all(real_dir) {
            return Entry::Missing;
        }
        match (fs::symlink_metadata(self.root.join(scope)), scope.note()) {
            (Ok(meta), _) if meta.is_dir() => Entry::Folder,
            (Ok(meta), Some(note)) if meta.is_file() => Entry::Note(note, Stamp::of(&meta)),
            (Ok(_), _) => Entry::Other,
            (Err(_), _) => Entry::Missing,
        }
    }

    /// Whether a note file is at `rel`.
    pub(crate) fn exists(&self, rel: &NotePath) -> bool {
        self.root.join(rel).is_file()
    }

    /// Every subfolder at or below `scope`, sorted by path. It skips `attachments/` at the root.
    ///
    /// `scope` can be a note, a folder, or a path with nothing there.
    pub(crate) fn folders(&self, scope: &Scope) -> Result<Vec<FolderPath>> {
        let listed = |rel: &FolderPath| rel.as_ref() != Path::new(ATTACHMENTS);
        let mut folders = Vec::new();
        let start = scope.folder();
        if !start.is_root() {
            if self.entry(scope) != Entry::Folder {
                return Ok(folders);
            }
            if listed(&start) {
                folders.push(start.clone());
            }
        }
        self.walk(&start, &mut |found| {
            if let Found::Folder(rel) = found
                && listed(&rel)
            {
                folders.push(rel);
            }
        })?;
        folders.sort();
        Ok(folders)
    }

    pub(crate) fn read(&self, rel: &NotePath) -> Result<(Note, Stamp)> {
        let (text, stamp) = read_text(&self.root.join(rel))?;
        Ok((note(rel, &Text::parse(&text), stamp), stamp))
    }

    /// Changes the frontmatter of the note with `edit` and keeps its modification time.
    pub(crate) fn edit_fields(
        &self,
        rel: &NotePath,
        edit: impl FnOnce(&mut Text),
    ) -> Result<(Note, Stamp)> {
        let path = self.root.join(rel);
        let (old, stamp) = read_text(&path)?;
        let mut text = Text::parse(&old);
        edit(&mut text);
        write(&path, &old, &text.to_string(), Some(stamp))?;
        self.read(rel)
    }

    /// Writes the new text of many notes. `edit` gets the path and the text of each note,
    /// and returns its new text, or `None` to keep it. Each note keeps its modification
    /// time and permissions.
    ///
    /// It syncs the whole file system twice, not each note. So each note changes in one
    /// step, but after a crash some notes can be new and some old.
    /// It returns each note that changed or failed. A note that fails keeps its old text.
    pub(crate) fn rewrite(
        &self,
        paths: &[NotePath],
        mut edit: impl FnMut(&NotePath, &str) -> Option<String>,
    ) -> Vec<(NotePath, Result<()>)> {
        let mut done = Vec::new();
        let mut temps = Vec::new();
        for rel in paths {
            let path = self.root.join(rel);
            let temp = read_text(&path).and_then(|(old, stamp)| {
                let Some(new) = edit(rel, &old).filter(|new| *new != old) else {
                    return Ok(None);
                };
                let fail = |source| write_error(&path, source);
                let permissions = fs::metadata(&path).map_err(fail)?.permissions();
                let modified = Some(time(stamp.modified));
                let file = unsynced_temp_file(
                    folder_abs(&path),
                    new.as_bytes(),
                    Some(permissions),
                    modified,
                )
                .map_err(fail)?;
                // A closed file, so a big job does not run out of file handles.
                Ok(Some(file.into_temp_path()))
            });
            match temp {
                Ok(Some(temp)) => temps.push((rel.clone(), (path, temp))),
                Ok(None) => {}
                Err(error) => done.push((rel.clone(), Err(error))),
            }
        }
        if temps.is_empty() {
            return done;
        }
        if let Err(errno) = self.sync_all_files() {
            done.extend(
                temps
                    .into_iter()
                    .map(|(rel, (path, _))| (rel, Err(write_error(&path, errno.into())))),
            );
            return done;
        }
        let mut renamed = Vec::new();
        for (rel, (path, temp)) in temps {
            match temp.persist(&path) {
                Ok(()) => renamed.push((rel, path)),
                Err(e) => done.push((rel, Err(write_error(&path, e.error)))),
            }
        }
        let synced = self.sync_all_files();
        done.extend(renamed.into_iter().map(|(rel, path)| {
            let result = synced.map_err(|errno| write_error(&path, errno.into()));
            (rel, result)
        }));
        done
    }

    /// Flushes every write on the file system of the notes folder to the disk.
    fn sync_all_files(&self) -> rustix::io::Result<()> {
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
        rustix::fs::syncfs(rustix::fs::open(&self.root, flags, Mode::empty())?)
    }

    /// Writes `body` over the body of the note at `rel`. The frontmatter stays.
    ///
    /// It adds an id when the file has none. When the body stays the same,
    /// the file keeps its old modification time.
    pub(crate) fn save(&self, rel: &NotePath, body: &str) -> Result<(Note, Stamp)> {
        let path = self.root.join(rel);
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
        let path = self.root.join(&note.info.path);
        let dir = self.root.join(note.info.path.folder());
        let mut text = Text::parse("");
        let id = note.info.id.clone().unwrap_or_else(NoteId::generate);
        text.set_id(Some(id.as_str()));
        text.set_tags(&note.info.tags);
        text.set_pinned(note.info.pinned);
        let file = temp_file(&dir, text.to_string().as_bytes(), None, None)
            .map_err(|source| write_error(&path, source))?;
        match file.persist_noclobber(&path) {
            Ok(_) => sync_dir(&dir).map_err(|source| write_error(&path, source))?,
            Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(write_error(&path, e.error)),
        }
        self.save(&note.info.path, &note.body)
    }

    /// Creates an empty note with a new id at `rel`.
    ///
    /// It fails with [`Error::NameTaken`] when something in its folder has the name.
    pub(crate) fn create(&self, rel: &NotePath) -> Result<(Note, Stamp)> {
        let folder = rel.folder();
        self.check_free(&folder, rel.name(), None)?;
        let dir = self.root.join(&folder);
        let path = self.root.join(rel);
        let mut text = Text::parse("");
        text.set_id(Some(NoteId::generate().as_str()));
        let file = temp_file(&dir, text.to_string().as_bytes(), None, None)
            .map_err(|source| write_error(&dir, source))?;
        match file.persist_noclobber(&path) {
            Ok(_) => sync_dir(&dir).map_err(|source| write_error(&path, source))?,
            Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(Error::NameTaken { path });
            }
            Err(e) => return Err(write_error(&path, e.error)),
        }
        self.read(rel)
    }

    /// Copies `content` into `attachments/` at the root as `<stem>-<hash>` + `ext`, and returns
    /// its absolute path. The hash is a short hash of the content, so a name never clashes.
    ///
    /// When a file with that name is there, it keeps that file and drops the new copy.
    /// It makes the `attachments/` folder when it is missing.
    pub(crate) fn add_attachment(
        &self,
        stem: &OsStr,
        ext: &str,
        content: impl io::Read,
    ) -> Result<PathBuf> {
        let attachments = FolderPath::root().folder_named(OsStr::new(ATTACHMENTS));
        let dir = self.root.join(ATTACHMENTS);
        match attachments.map(|folder| self.entry(&folder.scope())) {
            Some(Entry::Folder) => {}
            Some(Entry::Missing) => {
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
        let file =
            temp_file(&dir, &mut hashed, None, None).map_err(|source| write_error(&dir, source))?;
        let hash = hashed.hash.finalize();
        let mut name = stem.to_owned();
        name.push("-");
        for byte in &hash[..HASH_BYTES] {
            name.push(format!("{byte:02x}"));
        }
        name.push(ext);
        let path = dir.join(&name);
        match file.persist_noclobber(&path) {
            Ok(_) => sync_dir(&dir).map_err(|e| write_error(&path, e))?,
            Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(write_error(&path, e.error)),
        }
        Ok(path)
    }

    /// Moves the note at `from` to `to`, and returns `to`. It fixes the relative file links in
    /// the note.
    ///
    /// The note keeps its modification time and permissions. When `to` is `from`, the note
    /// stays where it is. It fails with [`Error::NameTaken`] when something in the folder of
    /// `to` has its name.
    pub(crate) fn move_note(&self, from: &NotePath, to: NotePath) -> Result<NotePath> {
        if to == *from {
            return Ok(to);
        }
        let source = self.root.join(from);
        let (old_folder, folder) = (from.folder(), to.folder());
        let own = (old_folder == folder).then(|| from.name());
        self.check_free(&folder, to.name(), own)?;
        let (old, stamp) = read_text(&source)?;
        let text = relink(&old, &self.root, &old_folder, &folder, None).unwrap_or(old);
        let permissions = fs::metadata(&source)
            .map_err(|e| Error::Read {
                path: source.clone(),
                source: e,
            })?
            .permissions();
        let dir = self.root.join(&folder);
        let temp = temp_file(
            &dir,
            text.as_bytes(),
            Some(permissions),
            Some(time(stamp.modified)),
        )
        .map_err(|source| write_error(&dir, source))?;
        let path = self.root.join(&to);
        match temp.persist_noclobber(&path) {
            Ok(_) => {
                remove(&source)?;
                sync_dir(&dir).map_err(|e| write_error(&path, e))?;
                sync_dir(folder_abs(&source)).map_err(|e| write_error(&source, e))?;
                Ok(to)
            }
            Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => {
                Err(Error::NameTaken { path })
            }
            Err(e) => Err(write_error(&path, e.error)),
        }
    }

    /// Creates the folder `name` in `parent`, and returns its path.
    ///
    /// `name` follows the rules of [`FolderPath::child`]. It fails with [`Error::NameTaken`]
    /// when something in `parent` has the name.
    pub(crate) fn create_folder(&self, parent: &FolderPath, name: &str) -> Result<FolderPath> {
        let rel = parent.child(name)?;
        self.check_free(parent, rel.name().unwrap_or_default(), None)?;
        let path = self.root.join(&rel);
        match fs::create_dir(&path) {
            Ok(()) => {
                sync_dir(folder_abs(&path)).map_err(|e| write_error(&path, e))?;
                Ok(rel)
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(Error::NameTaken { path }),
            Err(e) => Err(write_error(&path, e)),
        }
    }

    /// Moves the folder `from` to `parent` as `name`, and returns its new path.
    ///
    /// Names work as in [`Store::create_folder`]. The folder may take its own name with other
    /// capital letters. It fixes the relative file links in the notes inside. A note that can't
    /// be fixed keeps its links, with a log line.
    pub(crate) fn move_folder(
        &self,
        from: &FolderPath,
        parent: &FolderPath,
        name: &str,
    ) -> Result<FolderPath> {
        self.check_user_folder(from)?;
        if parent == from || parent.is_below(from) {
            return Err(bad_path(parent.as_ref()));
        }
        let rel = parent.child(name)?;
        if rel == *from {
            return Ok(rel);
        }
        let own = from
            .folder()
            .filter(|folder| folder == parent)
            .and(from.name());
        self.check_free(parent, rel.name().unwrap_or_default(), own)?;
        let source = self.root.join(from);
        let path = self.root.join(&rel);
        // An empty folder takes the name first. The rename then replaces it,
        // so it never replaces a folder that someone else made.
        match fs::create_dir(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                return Err(Error::NameTaken { path });
            }
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
        Ok(rel)
    }

    /// Fails with [`Error::NameTaken`] when an entry in `folder` has `name`, also with other
    /// capital letters. `own` is the name of the entry in `folder` that takes `name`, so it
    /// does not count.
    fn check_free(&self, folder: &FolderPath, name: &OsStr, own: Option<&OsStr>) -> Result<()> {
        let wanted = name.to_string_lossy().to_lowercase();
        let dir = self.root.join(folder);
        let Ok(entries) = fs::read_dir(&dir) else {
            return Ok(());
        };
        for entry in entries.flatten() {
            let found = entry.file_name();
            if found.to_string_lossy().to_lowercase() == wanted && own != Some(found.as_os_str()) {
                return Err(Error::NameTaken {
                    path: dir.join(name),
                });
            }
        }
        Ok(())
    }

    /// Deletes the note file at `rel`. A note that is already gone is no error.
    pub(crate) fn delete_note(&self, rel: &NotePath) -> Result<()> {
        let path = self.root.join(rel);
        remove(&path)?;
        sync_dir(folder_abs(&path)).map_err(|e| write_error(&path, e))
    }

    /// Deletes the folder `rel` with everything in it. A folder that is already gone is no error.
    pub(crate) fn delete_folder(&self, rel: &FolderPath) -> Result<()> {
        if self.entry(&rel.scope()) == Entry::Missing {
            return Ok(());
        }
        self.check_user_folder(rel)?;
        let path = self.root.join(rel);
        fs::remove_dir_all(&path).map_err(|e| write_error(&path, e))?;
        sync_dir(folder_abs(&path)).map_err(|e| write_error(&path, e))
    }

    /// Deletes the file at `rel` when [`Store::is_attachment`] allows it.
    pub(crate) fn delete_attachment(&self, rel: &Scope) -> Result<()> {
        if self.is_attachment(rel) {
            remove(&self.root.join(rel))?;
        }
        Ok(())
    }

    /// Whether `rel` is a file in `attachments/` that is not a note.
    /// A symlink counts as a file. No folder on the way may be a symlink.
    pub(crate) fn is_attachment(&self, rel: &Scope) -> bool {
        rel.is_attachment()
            && self.entry(rel) == Entry::Other
            && fs::symlink_metadata(self.root.join(rel)).is_ok_and(|meta| !meta.is_dir())
    }

    /// Checks that `rel` is a folder that the user may move or delete:
    /// a real folder that is not the root and not `attachments/`.
    fn check_user_folder(&self, rel: &FolderPath) -> Result<()> {
        if rel.is_root() || rel.as_ref() == Path::new(ATTACHMENTS) {
            return Err(bad_path(rel.as_ref()));
        }
        match self.entry(&rel.scope()) {
            Entry::Folder => Ok(()),
            Entry::Missing => Err(Error::Read {
                path: self.root.join(rel),
                source: io::ErrorKind::NotFound.into(),
            }),
            Entry::Note(..) | Entry::Other => Err(Error::Read {
                path: self.root.join(rel),
                source: io::ErrorKind::NotADirectory.into(),
            }),
        }
    }

    /// Fixes the relative file links of each note inside a folder that moved.
    fn relink_folder(&self, moved: Moved<'_>) -> Result<()> {
        let paths: Vec<NotePath> = self
            .files(&moved.to.scope())?
            .into_iter()
            .map(|(rel, _)| rel)
            .collect();
        let rewritten = self.rewrite(&paths, |rel, old| {
            let folder = rel.folder();
            let old_folder = folder.moved(moved.to, moved.from)?;
            relink(old, &self.root, &old_folder, &folder, Some(moved))
        });
        for (_, result) in rewritten {
            if let Err(error) = result {
                tracing::warn!(%error, "kept the file links of a moved note");
            }
        }
        Ok(())
    }

    /// Calls `visit` with each note, subfolder and Holm temp file below the folder `start`.
    /// It skips other hidden entries and symlinks, so it can't loop.
    fn walk(&self, start: &FolderPath, visit: &mut dyn FnMut(Found)) -> Result<()> {
        let mut dirs = vec![start.clone()];
        while let Some(dir) = dirs.pop() {
            let path = self.root.join(&dir);
            let entries = match fs::read_dir(&path) {
                Ok(entries) => entries,
                Err(source) if dir.is_root() => {
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
                if kind.is_file() && is_temp(&name) {
                    visit(Found::Temp(path.join(&name)));
                } else if kind.is_dir() {
                    if let Some(rel) = dir.folder_named(&name) {
                        visit(Found::Folder(rel.clone()));
                        dirs.push(rel);
                    }
                } else if kind.is_file()
                    && let Some(rel) = dir.note_named(&name)
                {
                    visit(Found::Note(rel));
                }
            }
        }
        Ok(())
    }
}

/// What is at a path in the notes folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    Note(NotePath, Stamp),
    Folder,
    /// A file that is not a note, or a symlink.
    Other,
    Missing,
}

/// What the folder walk finds.
enum Found {
    Note(NotePath),
    Folder(FolderPath),
    /// The absolute path of a Holm temp file.
    Temp(PathBuf),
}

fn note(rel: &NotePath, text: &Text, stamp: Stamp) -> Note {
    Note {
        info: NoteInfo {
            path: rel.clone(),
            id: text.id().map(NoteId::from_field),
            title: rel.title().into_owned(),
            preview: text.preview(),
            tags: text.tags(),
            pinned: text.pinned(),
            modified: time(stamp.modified),
        },
        body: text.body().to_owned(),
    }
}

fn folder_abs(path: &Path) -> &Path {
    path.parent().unwrap_or(Path::new("/"))
}

/// A file stem from any text, like the name of a pasted file.
///
/// It keeps the text as it is, but replaces `/` and a leading `.` with `-`.
/// It also cuts a very long text to [`MAX_STEM_BYTES`].
pub(crate) fn file_stem(text: &str) -> String {
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
    content: impl io::Read,
    permissions: Option<Permissions>,
    modified: Option<SystemTime>,
) -> io::Result<NamedTempFile> {
    let file = unsynced_temp_file(dir, content, permissions, modified)?;
    file.as_file().sync_all()?;
    Ok(file)
}

/// Writes `content` to a hidden temp file in `dir`, with no flush to the disk.
fn unsynced_temp_file(
    dir: &Path,
    mut content: impl io::Read,
    permissions: Option<Permissions>,
    modified: Option<SystemTime>,
) -> io::Result<NamedTempFile> {
    let mut file = tempfile::Builder::new()
        .prefix(TEMP_PREFIX)
        .suffix(TEMP_SUFFIX)
        .permissions(Permissions::from_mode(NEW_FILE_MODE))
        .tempfile_in(dir)?;
    io::copy(&mut content, &mut file)?;
    if let Some(permissions) = permissions {
        file.as_file().set_permissions(permissions)?;
    }
    if let Some(modified) = modified {
        file.as_file().set_modified(modified)?;
    }
    Ok(file)
}

/// Makes a rename in `dir` last after a crash.
fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

fn is_temp(name: &OsStr) -> bool {
    let name = name.as_encoded_bytes();
    name.starts_with(TEMP_PREFIX.as_bytes()) && name.ends_with(TEMP_SUFFIX.as_bytes())
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
