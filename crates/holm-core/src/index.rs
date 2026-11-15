//! The SQLite index of notes, tags and links, with full-text search.
//!
//! The index is a copy of the files. Holm deletes it and rebuilds it when its shape
//! is old or when SQLite fails on it. When the file can't be rebuilt, the index lives
//! in memory until Holm ends.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use holm_markdown::{Body, Target};
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension as _, params};

use crate::error::{Error, Result};
use crate::model::{Change, Link, LinkTarget, Note, NoteId, NoteInfo};
use crate::relink::resolve;
use crate::snapshot::Delta;
use crate::store::{ATTACHMENTS, Stamp, Store, folder_of, time};

const FILE: &str = "index.sqlite";
/// Change this number with each change to [`SCHEMA`] or to what a column holds.
/// An index with another number is rebuilt.
const SHAPE: i32 = 3;

const SCHEMA: &str = "
CREATE TABLE notes (
    key INTEGER PRIMARY KEY,
    path BLOB NOT NULL UNIQUE,
    folder BLOB NOT NULL,
    id TEXT UNIQUE,
    title TEXT NOT NULL,
    preview TEXT NOT NULL,
    pinned INTEGER NOT NULL,
    modified INTEGER NOT NULL,
    changed INTEGER NOT NULL,
    size INTEGER NOT NULL
);
CREATE INDEX notes_folder ON notes (folder);
CREATE TABLE tags (
    note INTEGER NOT NULL REFERENCES notes ON DELETE CASCADE,
    pos INTEGER NOT NULL,
    tag TEXT NOT NULL,
    PRIMARY KEY (note, pos)
);
CREATE INDEX tags_tag ON tags (tag);
CREATE TABLE links (
    note INTEGER NOT NULL REFERENCES notes ON DELETE CASCADE,
    pos INTEGER NOT NULL,
    kind TEXT NOT NULL,
    target BLOB NOT NULL,
    heading TEXT,
    embedded INTEGER NOT NULL,
    PRIMARY KEY (note, pos)
);
CREATE INDEX links_target ON links (kind, target);
CREATE VIRTUAL TABLE search USING fts5 (
    title, body, tags,
    tokenize = 'unicode61 remove_diacritics 2'
);
";

const NOTE_INFO: &str = "
SELECT path, id, title, pinned, modified, preview,
    (SELECT group_concat(tag, char(10) ORDER BY pos) FROM tags WHERE note = notes.key)
FROM notes";

/// What a [`Index::refresh`] found.
#[derive(Debug, Default)]
pub(crate) struct Refresh {
    /// The note changes, sorted by path. It holds no [`Change::Folders`] or [`Change::Tags`].
    pub(crate) notes: Vec<Change>,
    /// Whether the tag tree changed.
    pub(crate) tags: bool,
    /// The notes that left and joined the index.
    pub(crate) delta: Delta,
}

/// What [`Index::removal`] found.
#[derive(Debug)]
pub(crate) struct Removal {
    /// How many notes the delete removes.
    pub(crate) notes: usize,
    /// The attachment files that no other note links.
    pub(crate) files: Vec<PathBuf>,
}

/// The index file and one connection to it.
#[derive(Debug)]
pub(crate) struct Index {
    path: PathBuf,
    conn: Mutex<Connection>,
}

impl Index {
    /// Opens the index in `dir` and brings it up to date with the files.
    pub(crate) fn open(dir: &Path, store: &Store) -> Result<Self> {
        let path = dir.join(FILE);
        let opened = fs::create_dir_all(dir).map_err(Fail::Io).and_then(|()| {
            let mut conn = connect(&path)?;
            sync(&mut conn, store, &[PathBuf::new()])?;
            Ok(conn)
        });
        let conn = match opened {
            Ok(conn) => conn,
            Err(Fail::Core(error)) => return Err(error),
            Err(fail) => fresh(&path, store, &fail).map_err(|fail| fail.into_error(&path))?,
        };
        Ok(Self {
            path,
            conn: Mutex::new(conn),
        })
    }

    /// The paths of the notes whose title, text or tags hold every word of `text`, in no order.
    pub(crate) fn search(&self, store: &Store, text: &str) -> Result<Vec<PathBuf>> {
        let Some(query) = match_query(text) else {
            return Ok(Vec::new());
        };
        self.run(store, |conn| {
            let mut statement = conn.prepare_cached(
                "SELECT path FROM notes
                WHERE key IN (SELECT rowid FROM search WHERE search MATCH ?1)",
            )?;
            let rows = statement.query_map([&query], |row| Ok(path(row.get(0)?)))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// Every note with its links, in no order.
    pub(crate) fn entries(&self, store: &Store) -> Result<Vec<(NoteInfo, Vec<Link>)>> {
        self.run(store, |conn| {
            let mut statement = conn.prepare_cached(&format!("{NOTE_INFO} ORDER BY key"))?;
            let notes = statement
                .query_map([], note_info)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut statement = conn.prepare_cached(
                "SELECT kind, target, heading, embedded, notes.path FROM links
                JOIN notes ON notes.key = links.note ORDER BY links.note, pos",
            )?;
            let mut links: HashMap<PathBuf, Vec<Link>> = HashMap::new();
            for row in statement.query_map([], |row| Ok((path(row.get(4)?), link(row)?)))? {
                let (note, link) = row?;
                links.entry(note).or_default().push(link);
            }
            Ok(notes
                .into_iter()
                .map(|info| {
                    let links = links.remove(&info.path).unwrap_or_default();
                    (info, links)
                })
                .collect())
        })
    }

    /// Puts a note that Holm just read or wrote into the index, and returns it.
    ///
    /// When another note file has the same id, this note gets a new id.
    pub(crate) fn update(&self, store: &Store, note: &Note, stamp: Stamp) -> Result<(Note, Delta)> {
        self.run(store, |conn| {
            let tx = conn.transaction()?;
            let mut gone = Vec::new();
            let (note, links) = put(&tx, store, note.clone(), stamp, &mut gone)?;
            tx.commit()?;
            let delta = Delta {
                removed: gone.into_iter().map(|gone| gone.path).collect(),
                put: vec![(note.info.clone(), links)],
            };
            Ok((note, delta))
        })
    }

    /// Brings the notes at or below each path in `scopes` up to date with the files.
    /// The empty path is the whole notes folder. No scope may be inside another.
    pub(crate) fn refresh(&self, store: &Store, scopes: &[PathBuf]) -> Result<Refresh> {
        self.run(store, |conn| sync(conn, store, scopes))
    }

    /// What a delete of `scope` removes: a note path, or a folder with all its notes.
    ///
    /// The files are the ones in `attachments/` that the notes in `scope` link
    /// and no note outside `scope` links. They may be missing on disk.
    pub(crate) fn removal(&self, store: &Store, scope: &Path) -> Result<Removal> {
        let mut prefix = scope.as_os_str().as_bytes().to_vec();
        prefix.push(b'/');
        let attachments = format!("{ATTACHMENTS}/").into_bytes();
        self.run(store, |conn| {
            let notes = known_in(conn, scope)?.len();
            let mut statement = conn.prepare_cached(
                "SELECT DISTINCT links.target FROM links JOIN notes ON notes.key = links.note
                WHERE links.kind = 'file' AND substr(links.target, 1, length(?3)) = ?3
                    AND (notes.path = ?1 OR substr(notes.path, 1, length(?2)) = ?2)
                    AND NOT EXISTS (
                        SELECT 1 FROM links AS other JOIN notes AS owner ON owner.key = other.note
                        WHERE other.kind = 'file' AND other.target = links.target
                            AND owner.path != ?1 AND substr(owner.path, 1, length(?2)) != ?2)
                ORDER BY links.target",
            )?;
            let rows = statement.query_map(params![bytes(scope), prefix, attachments], |row| {
                Ok(path(row.get(0)?))
            })?;
            let files = rows.collect::<rusqlite::Result<_>>()?;
            Ok(Removal { notes, files })
        })
    }

    /// Runs `op`. When SQLite fails, it rebuilds the index and runs `op` again.
    fn run<T>(&self, store: &Store, op: impl Fn(&mut Connection) -> Result<T, Fail>) -> Result<T> {
        let mut conn = self.conn.lock().unwrap_or_else(PoisonError::into_inner);
        match op(&mut conn) {
            Err(Fail::Core(error)) => Err(Fail::Core(error)),
            Err(fail) => fresh(&self.path, store, &fail).and_then(|fresh| {
                *conn = fresh;
                op(&mut conn)
            }),
            result => result,
        }
        .map_err(|fail| fail.into_error(&self.path))
    }
}

/// An error inside this module. Only [`Fail::into_error`] turns it into an [`Error`].
enum Fail {
    Sql(rusqlite::Error),
    Io(io::Error),
    Core(Error),
}

impl Fail {
    fn into_error(self, path: &Path) -> Error {
        let source: Box<dyn std::error::Error + Send + Sync> = match self {
            Self::Sql(error) => Box::new(error),
            Self::Io(error) => Box::new(error),
            Self::Core(error) => return error,
        };
        Error::Index {
            path: path.to_owned(),
            source,
        }
    }
}

impl From<rusqlite::Error> for Fail {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sql(error)
    }
}

impl From<Error> for Fail {
    fn from(error: Error) -> Self {
        Self::Core(error)
    }
}

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sql(error) => error.fmt(f),
            Self::Io(error) => error.fmt(f),
            Self::Core(error) => error.fmt(f),
        }
    }
}

/// A new index after `fail`: the file rebuilt from the files, or else one in memory.
fn fresh(path: &Path, store: &Store, fail: &Fail) -> Result<Connection, Fail> {
    tracing::warn!(error = %fail, path = %path.display(), "the index failed, so Holm rebuilds it");
    match rebuild(path, store) {
        Err(Fail::Core(error)) => Err(Fail::Core(error)),
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "Holm keeps the index in memory");
            let mut conn = Connection::open_in_memory()?;
            conn.execute_batch("PRAGMA foreign_keys = ON;")?;
            schema(&mut conn)?;
            sync(&mut conn, store, &[PathBuf::new()])?;
            Ok(conn)
        }
        conn => conn,
    }
}

/// Opens the index file. A new file gets the schema. A file with another shape is deleted first.
fn connect(path: &Path) -> Result<Connection, Fail> {
    let mut conn = Connection::open(path)?;
    conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
    conn.execute_batch("PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;")?;
    let shape: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if shape == SHAPE {
        return Ok(conn);
    }
    let empty: bool = conn.query_row("SELECT count(*) = 0 FROM sqlite_schema", [], |row| {
        row.get(0)
    })?;
    if !empty {
        tracing::info!(shape, path = %path.display(), "the index has another shape, so Holm rebuilds it");
        drop(conn);
        remove(path)?;
        return connect(path);
    }
    schema(&mut conn)?;
    Ok(conn)
}

fn schema(conn: &mut Connection) -> Result<(), Fail> {
    let tx = conn.transaction()?;
    tx.execute_batch(SCHEMA)?;
    tx.pragma_update(None, "user_version", SHAPE)?;
    tx.commit()?;
    Ok(())
}

fn rebuild(path: &Path, store: &Store) -> Result<Connection, Fail> {
    remove(path)?;
    let mut conn = connect(path)?;
    sync(&mut conn, store, &[PathBuf::new()])?;
    Ok(conn)
}

/// Deletes the index file and the files SQLite keeps next to it.
fn remove(path: &Path) -> Result<(), Fail> {
    for suffix in ["", "-wal", "-shm"] {
        let mut file = path.as_os_str().to_owned();
        file.push(suffix);
        match fs::remove_file(&file) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(Fail::Io(error)),
            _ => {}
        }
    }
    Ok(())
}

/// Brings the notes in `scopes` up to date with the files. It reads only the files whose stamp changed.
///
/// It handles the files the index knows before new ones. So a known file keeps its id
/// when a new file has the same id.
fn sync(conn: &mut Connection, store: &Store, scopes: &[PathBuf]) -> Result<Refresh, Fail> {
    let mut files = Vec::new();
    for scope in scopes {
        files.extend(store.files(scope)?);
    }
    let tx = conn.transaction()?;
    let mut known: HashMap<PathBuf, Stamp> = HashMap::new();
    for scope in scopes {
        known.extend(known_in(&tx, scope)?);
    }
    let on_disk: HashSet<&Path> = files.iter().map(|(rel, _)| rel.as_path()).collect();
    let left: Vec<(&Path, Stamp)> = known
        .iter()
        .filter(|(rel, _)| !on_disk.contains(rel.as_path()))
        .map(|(rel, stamp)| (rel.as_path(), *stamp))
        .collect();
    let joined: Vec<(&Path, Stamp)> = files
        .iter()
        .filter(|(rel, _)| !known.contains_key(rel))
        .map(|(rel, stamp)| (rel.as_path(), *stamp))
        .collect();
    let mut moves = Vec::new();
    let mut done = HashSet::new();
    for (from, to) in pairs(&left, &joined) {
        if let Some(note) = shift(&tx, from, to)? {
            done.insert(from);
            done.insert(to);
            moves.push((from.to_owned(), note));
        }
    }
    let mut gone = Vec::new();
    for (rel, _) in left.iter().filter(|(rel, _)| !done.contains(rel)) {
        gone.extend(delete(&tx, rel)?);
    }
    let mut todo: Vec<(bool, &Path)> = files
        .iter()
        .filter(|(rel, stamp)| known.get(rel) != Some(stamp) && !done.contains(rel.as_path()))
        .map(|(rel, _)| (!known.contains_key(rel), rel.as_path()))
        .collect();
    todo.sort_unstable();
    let mut written = Vec::new();
    let mut put_in = Vec::new();
    for (new, rel) in todo {
        let old_tags = if new { None } else { Some(tags_of(&tx, rel)?) };
        match store.read(rel) {
            Ok((note, stamp)) => {
                let (note, links) = put(&tx, store, note, stamp, &mut gone)?;
                put_in.push((note.info.clone(), links));
                written.push((note.info, old_tags));
            }
            Err(error) => {
                tracing::warn!(%error, "skipped a note");
                gone.extend(delete(&tx, rel)?);
            }
        }
    }
    tx.commit()?;
    let mut removed: Vec<PathBuf> = gone.iter().map(|gone| gone.path.clone()).collect();
    for (from, (info, links)) in &moves {
        removed.push(from.clone());
        put_in.push((info.clone(), links.clone()));
    }
    let delta = Delta {
        removed,
        put: put_in,
    };
    let moves = moves.into_iter().map(|(from, (info, _))| (from, info));
    Ok(Refresh {
        delta,
        ..refresh(written, moves, gone)
    })
}

/// The gone paths and new paths that are one file at a new place: the same file name
/// and stamp, with no other gone or new path that has both.
///
/// A folder rename keeps the stamps of the files inside it.
fn pairs<'a>(
    left: &[(&'a Path, Stamp)],
    joined: &[(&'a Path, Stamp)],
) -> Vec<(&'a Path, &'a Path)> {
    type Key<'a> = (Option<&'a std::ffi::OsStr>, Stamp);
    let mut sides: HashMap<Key<'a>, (Vec<&'a Path>, Vec<&'a Path>)> = HashMap::new();
    for (rel, stamp) in left {
        sides
            .entry((rel.file_name(), *stamp))
            .or_default()
            .0
            .push(rel);
    }
    for (rel, stamp) in joined {
        if let Some(side) = sides.get_mut(&(rel.file_name(), *stamp)) {
            side.1.push(rel);
        }
    }
    sides
        .into_values()
        .filter_map(|(from, to)| match (from.as_slice(), to.as_slice()) {
            (&[from], &[to]) => Some((from, to)),
            _ => None,
        })
        .collect()
}

/// Moves the index row of the note at `from` to `to`, when the note has no file link.
/// Its key stays, so its text, tags and links stay too. It returns the moved note.
///
/// A relative file link points to another file after a move, so such a note must be
/// read again. The index does not keep whether a link was relative, so this
/// skips every note with a file link.
fn shift(conn: &Connection, from: &Path, to: &Path) -> Result<Option<(NoteInfo, Vec<Link>)>, Fail> {
    let count = conn
        .prepare_cached(
            "UPDATE notes SET path = ?2, folder = ?3 WHERE path = ?1
            AND NOT EXISTS (SELECT 1 FROM links WHERE links.note = notes.key AND kind = 'file')",
        )?
        .execute(params![bytes(from), bytes(to), bytes(folder_of(to))])?;
    if count == 0 {
        return Ok(None);
    }
    let info = conn
        .prepare_cached(&format!("{NOTE_INFO} WHERE path = ?1"))?
        .query_row([bytes(to)], note_info)?;
    let mut statement = conn.prepare_cached(
        "SELECT kind, target, heading, embedded FROM links
        JOIN notes ON notes.key = links.note WHERE notes.path = ?1 ORDER BY pos",
    )?;
    let links = statement
        .query_map([bytes(to)], link)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Some((info, links)))
}

/// The stamps of the known notes at or below `scope`.
fn known_in(conn: &Connection, scope: &Path) -> Result<Vec<(PathBuf, Stamp)>, Fail> {
    let mut prefix = scope.as_os_str().as_bytes().to_vec();
    prefix.push(b'/');
    let mut statement = conn.prepare_cached(
        "SELECT path, modified, changed, size FROM notes
        WHERE ?1 OR path = ?2 OR substr(path, 1, length(?3)) = ?3",
    )?;
    let rows = statement.query_map(
        params![scope.as_os_str().is_empty(), bytes(scope), prefix],
        |row| {
            let stamp = Stamp {
                modified: row.get(1)?,
                changed: row.get(2)?,
                size: row.get(3)?,
            };
            Ok((path(row.get(0)?), stamp))
        },
    )?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// A note that left the index.
struct Gone {
    path: PathBuf,
    id: Option<NoteId>,
    tags: Vec<String>,
}

/// Turns the notes that `sync` wrote and removed into changes.
///
/// A removed note and a new note with the same id are one move. A shifted row without an id
/// is a removed note and a new one, as a read would find it.
fn refresh(
    written: Vec<(NoteInfo, Option<Vec<String>>)>,
    moves: impl Iterator<Item = (PathBuf, NoteInfo)>,
    mut gone: Vec<Gone>,
) -> Refresh {
    let mut out = Refresh::default();
    for (from, note) in moves {
        if note.id.is_some() {
            out.notes.push(Change::Moved { from, note });
        } else {
            out.tags |= !note.tags.is_empty();
            out.notes.push(Change::Removed(from));
            out.notes.push(Change::Added(note));
        }
    }
    for (note, old_tags) in written {
        let from = match (&old_tags, &note.id) {
            (None, Some(id)) => gone
                .iter()
                .position(|g| g.id.as_ref() == Some(id))
                .map(|i| gone.swap_remove(i)),
            _ => None,
        };
        let change = match (from, old_tags) {
            (Some(from), _) => {
                out.tags |= !same_tags(&from.tags, &note.tags);
                Change::Moved {
                    from: from.path,
                    note,
                }
            }
            (None, None) => {
                out.tags |= !note.tags.is_empty();
                Change::Added(note)
            }
            (None, Some(old)) => {
                out.tags |= !same_tags(&old, &note.tags);
                Change::Changed(note)
            }
        };
        out.notes.push(change);
    }
    for gone in gone {
        out.tags |= !gone.tags.is_empty();
        out.notes.push(Change::Removed(gone.path));
    }
    out.notes
        .sort_by(|a, b| change_path(a).cmp(&change_path(b)));
    out
}

fn same_tags(a: &[String], b: &[String]) -> bool {
    a.iter().collect::<HashSet<_>>() == b.iter().collect::<HashSet<_>>()
}

fn change_path(change: &Change) -> Option<&Path> {
    match change {
        Change::Added(note) | Change::Changed(note) | Change::Moved { note, .. } => {
            Some(&note.path)
        }
        Change::Removed(path) => Some(path),
        _ => None,
    }
}

/// Writes one note into the index, after a check that no other file has its id.
/// It returns the note and its links.
///
/// When the other file is gone, its note leaves the index and goes into `gone`.
fn put(
    conn: &Connection,
    store: &Store,
    mut note: Note,
    mut stamp: Stamp,
    gone: &mut Vec<Gone>,
) -> Result<(Note, Vec<Link>), Fail> {
    if let Some(id) = &note.info.id
        && let Some(owner) = owner(conn, id)?
        && owner != note.info.path
    {
        if store.exists(&owner) {
            let new = NoteId::generate();
            tracing::info!(%id, %new, path = %note.info.path.display(), "gave a copied note a new id");
            match store.edit_fields(&note.info.path, |text| text.set_id(Some(new.as_str()))) {
                Ok((written, written_stamp)) => (note, stamp) = (written, written_stamp),
                Err(error) => {
                    tracing::warn!(%error, "kept a copied note without an id");
                    note.info.id = None;
                }
            }
        } else {
            gone.extend(delete(conn, &owner)?);
        }
    }
    let links = links_of(store.root(), &note);
    write(conn, &note, &links, stamp)?;
    Ok((note, links))
}

/// The links of the note, with file paths from the notes folder.
fn links_of(root: &Path, note: &Note) -> Vec<Link> {
    let folder = folder_of(&note.info.path);
    Body::parse(&note.body)
        .links()
        .iter()
        .map(|link| Link {
            target: match &link.target {
                Target::Note { id, heading } => LinkTarget::Note {
                    id: NoteId::from_field(id.clone()),
                    heading: heading.clone(),
                },
                Target::Heading(heading) => LinkTarget::Heading(heading.clone()),
                Target::File(file) => LinkTarget::File(resolve(root, folder, file)),
                Target::Web(address) => LinkTarget::Web(address.clone()),
            },
            embedded: link.embedded,
        })
        .collect()
}

fn tags_of(conn: &Connection, rel: &Path) -> Result<Vec<String>, Fail> {
    let mut statement = conn.prepare_cached(
        "SELECT tag FROM tags JOIN notes ON notes.key = tags.note WHERE notes.path = ?1 ORDER BY pos",
    )?;
    let rows = statement.query_map([bytes(rel)], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn owner(conn: &Connection, id: &NoteId) -> Result<Option<PathBuf>, Fail> {
    let owner = conn
        .prepare_cached("SELECT path FROM notes WHERE id = ?1")?
        .query_row([id.as_str()], |row| row.get(0))
        .optional()?;
    Ok(owner.map(path))
}

fn write(conn: &Connection, note: &Note, links: &[Link], stamp: Stamp) -> Result<(), Fail> {
    let info = &note.info;
    let folder = folder_of(&info.path);
    let key: i64 = conn
        .prepare_cached(
            "INSERT INTO notes (path, folder, id, title, preview, pinned, modified, changed, size)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT (path) DO UPDATE SET folder = excluded.folder, id = excluded.id,
                title = excluded.title, preview = excluded.preview, pinned = excluded.pinned, modified = excluded.modified,
                changed = excluded.changed, size = excluded.size
            RETURNING key",
        )?
        .query_row(
            params![
                bytes(&info.path),
                bytes(folder),
                info.id.as_ref().map(NoteId::as_str),
                info.title,
                info.preview,
                info.pinned,
                stamp.modified,
                stamp.changed,
                stamp.size,
            ],
            |row| row.get(0),
        )?;
    for sql in [
        "DELETE FROM tags WHERE note = ?1",
        "DELETE FROM links WHERE note = ?1",
        "DELETE FROM search WHERE rowid = ?1",
    ] {
        conn.prepare_cached(sql)?.execute([key])?;
    }
    let mut add_tag =
        conn.prepare_cached("INSERT INTO tags (note, pos, tag) VALUES (?1, ?2, ?3)")?;
    for (pos, tag) in (0_i64..).zip(&info.tags) {
        add_tag.execute(params![key, pos, tag])?;
    }
    let mut add_link = conn.prepare_cached(
        "INSERT INTO links (note, pos, kind, target, heading, embedded)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for (pos, link) in (0_i64..).zip(links) {
        let (kind, target, heading) = match &link.target {
            LinkTarget::Note { id, heading } => (
                "note",
                Value::Text(id.as_str().to_owned()),
                heading.as_deref(),
            ),
            LinkTarget::Heading(heading) => ("heading", Value::Text(heading.clone()), None),
            LinkTarget::File(file) => ("file", bytes(file), None),
            LinkTarget::Web(address) => ("web", Value::Text(address.clone()), None),
        };
        add_link.execute(params![key, pos, kind, target, heading, link.embedded])?;
    }
    conn.prepare_cached("INSERT INTO search (rowid, title, body, tags) VALUES (?1, ?2, ?3, ?4)")?
        .execute(params![key, info.title, note.body, info.tags.join(" ")])?;
    Ok(())
}

/// Removes the note at `rel` from the index, and returns what it was.
fn delete(conn: &Connection, rel: &Path) -> Result<Option<Gone>, Fail> {
    let id: Option<Option<String>> = conn
        .prepare_cached("SELECT id FROM notes WHERE path = ?1")?
        .query_row([bytes(rel)], |row| row.get(0))
        .optional()?;
    let Some(id) = id else { return Ok(None) };
    let tags = tags_of(conn, rel)?;
    conn.prepare_cached(
        "DELETE FROM search WHERE rowid = (SELECT key FROM notes WHERE path = ?1)",
    )?
    .execute([bytes(rel)])?;
    conn.prepare_cached("DELETE FROM notes WHERE path = ?1")?
        .execute([bytes(rel)])?;
    Ok(Some(Gone {
        path: rel.to_owned(),
        id: id.map(NoteId::from_field),
        tags,
    }))
}

/// Turns the words the user typed into an FTS5 query. Each word matches as a prefix.
fn match_query(text: &str) -> Option<String> {
    let words: Vec<String> = text
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .map(|word| format!("\"{}\"*", word.replace('"', "\"\"")))
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

fn note_info(row: &rusqlite::Row<'_>) -> rusqlite::Result<NoteInfo> {
    let tags: Option<String> = row.get(6)?;
    Ok(NoteInfo {
        path: path(row.get(0)?),
        id: row.get::<_, Option<String>>(1)?.map(NoteId::from_field),
        title: row.get(2)?,
        preview: row.get(5)?,
        tags: tags
            .map(|tags| tags.split('\n').map(str::to_owned).collect())
            .unwrap_or_default(),
        pinned: row.get(3)?,
        modified: time(row.get(4)?),
    })
}

fn link(row: &rusqlite::Row<'_>) -> rusqlite::Result<Link> {
    let kind: String = row.get(0)?;
    let text = || -> rusqlite::Result<String> { row.get(1) };
    let target = match kind.as_str() {
        "note" => LinkTarget::Note {
            id: NoteId::from_field(text()?),
            heading: row.get(2)?,
        },
        "heading" => LinkTarget::Heading(text()?),
        "file" => LinkTarget::File(path(row.get(1)?)),
        _ => LinkTarget::Web(text()?),
    };
    Ok(Link {
        target,
        embedded: row.get(3)?,
    })
}

fn bytes(path: &Path) -> Value {
    Value::Blob(path.as_os_str().as_bytes().to_vec())
}

fn path(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(OsString::from_vec(bytes))
}
