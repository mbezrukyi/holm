//! The SQLite index of notes, tags and links, with full-text search.
//!
//! The index is a copy of the files. Holm deletes it and rebuilds it when its shape
//! is old or when SQLite fails on it. When the file can't be rebuilt, the index lives
//! in memory until Holm ends.

use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::hash::{BuildHasher as _, Hash};
use std::io;
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use holm_markdown::{Body, Target};
use rusqlite::config::DbConfig;
use rusqlite::types::{Type, Value};
use rusqlite::{Connection, OptionalExtension as _, params};

use crate::error::{Error, Result};
use crate::model::{Change, Link, LinkTarget, Note, NoteId, NoteInfo};
use crate::paths::{FolderPath, Node, NotePath, NoteReader};
use crate::relink::resolve;
use crate::snapshot::Delta;
use crate::store::{ATTACHMENTS, Stamp, Store, time};

mod search;

const FILE: &str = "index.sqlite";
/// Change this number with each change to [`SCHEMA`], to [`search::SCHEMA`], or to what a
/// column holds.
/// An index with another number is rebuilt.
const SHAPE: i32 = 7;

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
    size INTEGER NOT NULL,
    source_hash INTEGER NOT NULL,
    links_hash INTEGER NOT NULL
);
CREATE INDEX notes_folder ON notes (folder);
CREATE TABLE tags (
    note INTEGER NOT NULL REFERENCES notes ON DELETE CASCADE,
    pos INTEGER NOT NULL,
    tag TEXT NOT NULL,
    PRIMARY KEY (note, pos)
);
CREATE INDEX tags_tag ON tags (tag, note);
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
    /// The file link targets in `attachments/` that no other note links, relative to the
    /// notes folder. They are outside input.
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
            sync(&mut conn, store, &[Node::Folder(FolderPath::root())])?;
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
    pub(crate) fn search(&self, store: &Store, text: &str) -> Result<Vec<NotePath>> {
        self.run(store, |conn| Ok(search::find(conn, text)?))
    }

    /// Every note with its links, in no order.
    pub(crate) fn entries(&self, store: &Store) -> Result<Vec<(NoteInfo, Vec<Link>)>> {
        self.run(store, |conn| {
            let mut reader = NoteReader::default();
            let mut statement = conn.prepare_cached(&format!("{NOTE_INFO} ORDER BY key"))?;
            let notes = statement
                .query_map([], |row| note_info(&mut reader, row))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut statement = conn.prepare_cached(
                "SELECT kind, target, heading, embedded, notes.path FROM links
                JOIN notes ON notes.key = links.note ORDER BY links.note, pos",
            )?;
            let mut links: HashMap<NotePath, Vec<Link>> = HashMap::new();
            let rows =
                statement.query_map([], |row| Ok((note_path(&mut reader, row, 4)?, link(row)?)))?;
            for row in rows {
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

    /// Puts notes that Holm just read or wrote into the index in one transaction,
    /// and returns them in the same order.
    ///
    /// When another note file has the same id, a note gets a new id.
    pub(crate) fn update(
        &self,
        store: &Store,
        notes: &[(Note, Stamp)],
    ) -> Result<(Vec<Note>, Delta)> {
        self.run(store, |conn| {
            let tx = conn.transaction()?;
            let mut gone = Vec::new();
            let mut put_notes = Vec::with_capacity(notes.len());
            let mut delta = Delta::default();
            for (note, stamp) in notes {
                let (note, links) = put(&tx, store, note.clone(), *stamp, false, &mut gone)?;
                delta.put.push((note.info.clone(), links));
                put_notes.push(note);
            }
            tx.commit()?;
            delta.removed = gone.into_iter().map(|gone| gone.path).collect();
            Ok((put_notes, delta))
        })
    }

    /// Brings the notes at or below each of `nodes` up to date with the files.
    /// No node may be inside another.
    pub(crate) fn refresh(&self, store: &Store, nodes: &[Node]) -> Result<Refresh> {
        self.run(store, |conn| sync(conn, store, nodes))
    }

    /// What a delete of `node` removes: a note, or a folder with all its notes.
    ///
    /// The files are the ones in `attachments/` that the notes in `node` link
    /// and no note outside `node` links. They may be missing on disk.
    pub(crate) fn removal(&self, store: &Store, node: &Node) -> Result<Removal> {
        let path = node_bytes(node);
        let mut prefix = path.clone();
        prefix.push(b'/');
        let attachments = format!("{ATTACHMENTS}/").into_bytes();
        self.run(store, |conn| {
            let notes = known_in(conn, node)?.len();
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
            let rows = statement.query_map(params![path, prefix, attachments], |row| {
                row.get::<_, Vec<u8>>(0)
            })?;
            let mut files = Vec::new();
            for target in rows {
                files.push(PathBuf::from(OsString::from_vec(target?)));
            }
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
            settings(&conn)?;
            schema(&mut conn)?;
            sync(&mut conn, store, &[Node::Folder(FolderPath::root())])?;
            Ok(conn)
        }
        conn => conn,
    }
}

/// Opens the index file. A new file gets the schema. A file with another shape is deleted first.
fn connect(path: &Path) -> Result<Connection, Fail> {
    let mut conn = Connection::open(path)?;
    conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
    conn.execute_batch("PRAGMA synchronous = NORMAL;")?;
    settings(&conn)?;
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

fn settings(conn: &Connection) -> Result<(), Fail> {
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    // Without it, SQLite prepares some statements again for each new bound value,
    // like the read of the link rows of a note.
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_QPSG, true)?;
    Ok(())
}

fn schema(conn: &mut Connection) -> Result<(), Fail> {
    let tx = conn.transaction()?;
    tx.execute_batch(SCHEMA)?;
    tx.execute_batch(search::SCHEMA)?;
    tx.pragma_update(None, "user_version", SHAPE)?;
    tx.commit()?;
    Ok(())
}

fn rebuild(path: &Path, store: &Store) -> Result<Connection, Fail> {
    remove(path)?;
    let mut conn = connect(path)?;
    sync(&mut conn, store, &[Node::Folder(FolderPath::root())])?;
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

/// Brings the notes in `places` up to date with the files. It reads only the files whose stamp changed.
///
/// It handles the files the index knows before new ones. So a known file keeps its id
/// when a new file has the same id.
fn sync(conn: &mut Connection, store: &Store, nodes: &[Node]) -> Result<Refresh, Fail> {
    let mut files = Vec::new();
    for node in nodes {
        files.extend(store.files(node)?);
    }
    let tx = conn.transaction()?;
    let mut known: HashMap<NotePath, Stamp> = HashMap::new();
    for node in nodes {
        known.extend(known_in(&tx, node)?);
    }
    let on_disk: HashSet<&NotePath> = files.iter().map(|(rel, _)| rel).collect();
    let left: Vec<(&NotePath, Stamp)> = known
        .iter()
        .filter(|(rel, _)| !on_disk.contains(rel))
        .map(|(rel, stamp)| (rel, *stamp))
        .collect();
    let joined: Vec<(&NotePath, Stamp)> = files
        .iter()
        .filter(|(rel, _)| !known.contains_key(rel))
        .map(|(rel, stamp)| (rel, *stamp))
        .collect();
    let mut moves = Vec::new();
    let mut done = HashSet::new();
    for (from, to) in pairs(&left, &joined) {
        if let Some(note) = shift(&tx, from, to)? {
            done.insert(from);
            done.insert(to);
            moves.push((from.clone(), note));
        }
    }
    let mut gone = Vec::new();
    for (rel, _) in left.iter().filter(|(rel, _)| !done.contains(rel)) {
        gone.extend(delete(&tx, rel)?);
    }
    let mut todo: Vec<(bool, &NotePath)> = files
        .iter()
        .filter(|(rel, stamp)| known.get(rel) != Some(stamp) && !done.contains(rel))
        .map(|(rel, _)| (!known.contains_key(rel), rel))
        .collect();
    todo.sort_unstable();
    let mut written = Vec::new();
    let mut put_in = Vec::new();
    for (new, rel) in todo {
        let old_tags = if new { None } else { Some(tags_of(&tx, rel)?) };
        match store.read(rel) {
            Ok((note, stamp)) => {
                let (note, links) = put(&tx, store, note, stamp, new, &mut gone)?;
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
    let mut removed: Vec<NotePath> = gone.iter().map(|gone| gone.path.clone()).collect();
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
    left: &[(&'a NotePath, Stamp)],
    joined: &[(&'a NotePath, Stamp)],
) -> Vec<(&'a NotePath, &'a NotePath)> {
    type Key<'a> = (&'a OsStr, Stamp);
    type Sides<'a> = (Vec<&'a NotePath>, Vec<&'a NotePath>);
    let mut sides: HashMap<Key<'a>, Sides<'a>> = HashMap::new();
    for (rel, stamp) in left {
        sides.entry((rel.name(), *stamp)).or_default().0.push(rel);
    }
    for (rel, stamp) in joined {
        if let Some(side) = sides.get_mut(&(rel.name(), *stamp)) {
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
fn shift(
    conn: &Connection,
    from: &NotePath,
    to: &NotePath,
) -> Result<Option<(NoteInfo, Vec<Link>)>, Fail> {
    let key: Option<i64> = conn
        .prepare_cached(
            "UPDATE notes SET path = ?2, folder = ?3 WHERE path = ?1
            AND NOT EXISTS (SELECT 1 FROM links WHERE links.note = notes.key AND kind = 'file')
            RETURNING key",
        )?
        .query_row(
            params![note_bytes(from), note_bytes(to), bytes(to.folder())],
            |row| row.get(0),
        )
        .optional()?;
    let Some(key) = key else {
        return Ok(None);
    };
    let info = conn
        .prepare_cached(&format!("{NOTE_INFO} WHERE key = ?1"))?
        .query_row([key], |row| note_info(&mut NoteReader::default(), row))?;
    Ok(Some((info, stored_links(conn, key)?)))
}

/// The stamps of the known notes at or below `node`.
fn known_in(conn: &Connection, node: &Node) -> Result<Vec<(NotePath, Stamp)>, Fail> {
    let path = node_bytes(node);
    let mut prefix = path.clone();
    prefix.push(b'/');
    let all = matches!(node, Node::Folder(folder) if folder.is_root());
    let mut statement = conn.prepare_cached(
        "SELECT path, modified, changed, size FROM notes
        WHERE ?1 OR path = ?2 OR substr(path, 1, length(?3)) = ?3",
    )?;
    let mut reader = NoteReader::default();
    let rows = statement.query_map(params![all, path, prefix], |row| {
        let stamp = Stamp {
            modified: row.get(1)?,
            changed: row.get(2)?,
            size: row.get(3)?,
        };
        Ok((note_path(&mut reader, row, 0)?, stamp))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// A note that left the index.
struct Gone {
    path: NotePath,
    id: Option<NoteId>,
    tags: Vec<String>,
}

/// Turns the notes that `sync` wrote and removed into changes.
///
/// A removed note and a new note with the same id are one move. A shifted row without an id
/// is a removed note and a new one, as a read would find it.
fn refresh(
    written: Vec<(NoteInfo, Option<Vec<String>>)>,
    moves: impl Iterator<Item = (NotePath, NoteInfo)>,
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

fn change_path(change: &Change) -> Option<&NotePath> {
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
/// When `new` is true, the caller knows that the index has no note at this path.
fn put(
    conn: &Connection,
    store: &Store,
    mut note: Note,
    mut stamp: Stamp,
    new: bool,
    gone: &mut Vec<Gone>,
) -> Result<(Note, Vec<Link>), Fail> {
    if let Some(id) = &note.info.id
        && let Some(owner) = owner(conn, id)?
        && owner != note.info.path
    {
        if store.exists(&owner) {
            let new_id = NoteId::generate();
            tracing::info!(%id, new = %new_id, path = %note.info.path, "gave a copied note a new id");
            match store.edit_fields(&note.info.path, |text| text.set_id(Some(new_id.as_str()))) {
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
    let links = write(conn, store.root(), &note, stamp, new)?;
    Ok((note, links))
}

/// The links of the note, with file paths from the notes folder.
fn links_of(root: &Path, note: &Note) -> Vec<Link> {
    let folder = note.info.path.folder();
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

fn tags_of(conn: &Connection, rel: &NotePath) -> Result<Vec<String>, Fail> {
    let mut statement = conn.prepare_cached(
        "SELECT tag FROM tags JOIN notes ON notes.key = tags.note WHERE notes.path = ?1 ORDER BY pos",
    )?;
    let rows = statement.query_map([note_bytes(rel)], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn owner(conn: &Connection, id: &NoteId) -> Result<Option<NotePath>, Fail> {
    let owner = conn
        .prepare_cached("SELECT path FROM notes WHERE id = ?1")?
        .query_row([id.as_str()], |row| {
            note_path(&mut NoteReader::default(), row, 0)
        })
        .optional()?;
    Ok(owner)
}

/// Writes the rows of one note and returns its links.
///
/// The links come from the body and the folder, their source. When the source stays,
/// the links come from the stored rows. When the parsed links stay, the stored rows stay.
fn write(
    conn: &Connection,
    root: &Path,
    note: &Note,
    stamp: Stamp,
    new: bool,
) -> Result<Vec<Link>, Fail> {
    let info = &note.info;
    let folder = info.path.folder();
    let source_hash = hash((&note.body, AsRef::<Path>::as_ref(&folder)));
    let old: Option<(i64, i64)> = if new {
        None
    } else {
        conn.prepare_cached("SELECT source_hash, links_hash FROM notes WHERE path = ?1")?
            .query_row([note_bytes(&info.path)], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?
    };
    let (links, links_hash, stored) = match old {
        Some((old_source, old_links)) if old_source == source_hash => (None, old_links, true),
        _ => {
            let links = links_of(root, note);
            let links_hash = hash(&links);
            let stored = old.is_some_and(|(_, old_links)| old_links == links_hash);
            (Some(links), links_hash, stored)
        }
    };
    let key: i64 = conn
        .prepare_cached(
            "INSERT INTO notes (path, folder, id, title, preview, pinned, modified, changed, size,
                source_hash, links_hash)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT (path) DO UPDATE SET folder = excluded.folder, id = excluded.id,
                title = excluded.title, preview = excluded.preview, pinned = excluded.pinned, modified = excluded.modified,
                changed = excluded.changed, size = excluded.size,
                source_hash = excluded.source_hash, links_hash = excluded.links_hash
            RETURNING key",
        )?
        .query_row(
            params![
                note_bytes(&info.path),
                bytes(folder),
                info.id.as_ref().map(NoteId::as_str),
                info.title,
                info.preview,
                info.pinned,
                stamp.modified,
                stamp.changed,
                stamp.size,
                source_hash,
                links_hash,
            ],
            |row| row.get(0),
        )?;
    conn.prepare_cached("DELETE FROM tags WHERE note = ?1")?
        .execute([key])?;
    let mut add_tag =
        conn.prepare_cached("INSERT INTO tags (note, pos, tag) VALUES (?1, ?2, ?3)")?;
    for (pos, tag) in (0_i64..).zip(&info.tags) {
        add_tag.execute(params![key, pos, tag])?;
    }
    let links = match links {
        None => stored_links(conn, key)?,
        Some(links) => {
            if !stored {
                write_links(conn, key, &links)?;
            }
            links
        }
    };
    search::put(conn, key, &info.title, &note.body, &info.tags)?;
    Ok(links)
}

/// Replaces the link rows of the note with `key`.
fn write_links(conn: &Connection, key: i64, links: &[Link]) -> Result<(), Fail> {
    conn.prepare_cached("DELETE FROM links WHERE note = ?1")?
        .execute([key])?;
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
    Ok(())
}

/// The links of the note with `key`, from its rows.
fn stored_links(conn: &Connection, key: i64) -> Result<Vec<Link>, Fail> {
    let mut statement = conn.prepare_cached(
        "SELECT kind, target, heading, embedded FROM links WHERE note = ?1 ORDER BY pos",
    )?;
    let links = statement
        .query_map([key], link)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(links)
}

/// A hash of `value`. Two values with the same hash count as the same.
///
/// The hash may change with the `foldhash` version. Then the index writes the rows of each
/// note once more.
fn hash(value: impl Hash) -> i64 {
    let hash = foldhash::quality::FixedState::default().hash_one(value);
    i64::from_ne_bytes(hash.to_ne_bytes())
}

/// Removes the note at `rel` from the index, and returns what it was.
fn delete(conn: &Connection, rel: &NotePath) -> Result<Option<Gone>, Fail> {
    let row: Option<(i64, Option<String>)> = conn
        .prepare_cached("SELECT key, id FROM notes WHERE path = ?1")?
        .query_row([note_bytes(rel)], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()?;
    let Some((key, id)) = row else {
        return Ok(None);
    };
    let tags = tags_of(conn, rel)?;
    search::remove(conn, key)?;
    conn.prepare_cached("DELETE FROM notes WHERE key = ?1")?
        .execute([key])?;
    Ok(Some(Gone {
        path: rel.clone(),
        id: id.map(NoteId::from_field),
        tags,
    }))
}

fn note_info(reader: &mut NoteReader, row: &rusqlite::Row<'_>) -> rusqlite::Result<NoteInfo> {
    let tags: Option<String> = row.get(6)?;
    Ok(NoteInfo {
        path: note_path(reader, row, 0)?,
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

fn bytes(path: impl AsRef<Path>) -> Value {
    Value::Blob(path.as_ref().as_os_str().as_bytes().to_vec())
}

fn note_bytes(path: &NotePath) -> Value {
    Value::Blob(path.bytes().to_vec())
}

fn node_bytes(node: &Node) -> Vec<u8> {
    match node {
        Node::Note(path) => path.bytes().to_vec(),
        Node::Folder(folder) => folder.as_ref().as_os_str().as_bytes().to_vec(),
    }
}

fn path(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(OsString::from_vec(bytes))
}

/// The note path in column `index`. A path that is not a note path fails like a broken row, so
/// the index gets rebuilt.
fn note_path(
    reader: &mut NoteReader,
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<NotePath> {
    let fail = |error| rusqlite::Error::FromSqlConversionFailure(index, Type::Blob, error);
    let bytes = row.get_ref(index)?.as_blob().map_err(|e| fail(e.into()))?;
    reader
        .note(bytes)
        .ok_or_else(|| fail("not a note path".into()))
}
