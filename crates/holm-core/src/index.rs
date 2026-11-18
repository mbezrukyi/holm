//! The SQLite index of notes, tags and links, with full-text search.
//!
//! The index is a copy of the files, for a fast start and for search. It never reads or
//! writes a note file. When SQLite fails on it or its shape is old, it starts again empty.
//! When a new file fails too, the index lives in memory until Holm ends.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::hash::{BuildHasher as _, Hash};
use std::io;
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::path::{Path, PathBuf};

use rusqlite::config::DbConfig;
use rusqlite::types::{Type, Value};
use rusqlite::{Connection, OptionalExtension as _, params};

use crate::error::{Error, Result};
use crate::model::{Link, LinkTarget, Note, NoteId, NoteInfo};
use crate::paths::{NotePath, NoteReader};
use crate::reconcile::Settled;
use crate::relink;
use crate::store::{Stamp, time};
use crate::tag::TagName;

mod search;

const FILE: &str = "index.sqlite";
/// Change this number with each change to [`SCHEMA`], to [`search::SCHEMA`], or to what a
/// column holds.
/// An index with another number is rebuilt.
const SHAPE: i32 = 8;

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

/// The columns that [`note_info`] reads, from the table `notes`.
const NOTE_INFO: &str = "
path, id, title, pinned, modified, preview,
    (SELECT group_concat(tag, char(10) ORDER BY pos) FROM tags WHERE note = notes.key)";

/// The index file and one connection to it.
#[derive(Debug)]
pub(crate) struct Index {
    path: PathBuf,
    /// The notes folder, for the paths of file links.
    root: PathBuf,
    conn: Connection,
}

impl Index {
    /// Opens the index in `dir`, for the notes folder `root`. When SQLite fails on it, the
    /// index starts empty.
    pub(crate) fn open(dir: &Path, root: &Path) -> Result<Self> {
        let path = dir.join(FILE);
        let opened = fs::create_dir_all(dir)
            .map_err(Fail::Io)
            .and_then(|()| connect(&path));
        let conn = match opened {
            Ok(conn) => conn,
            Err(fail) => {
                tracing::warn!(error = %fail, path = %path.display(), "the index failed, so it starts empty");
                fresh(&path, root, &Settled::default()).map_err(|fail| fail.into_error(&path))?
            }
        };
        Ok(Self {
            path,
            root: root.to_owned(),
            conn,
        })
    }

    /// Drops every row and writes the rows of `settled`, which holds every note file.
    /// The rows go into a new file, or into memory when the new file fails.
    pub(crate) fn rebuild(&mut self, settled: &Settled) -> Result<()> {
        self.conn =
            fresh(&self.path, &self.root, settled).map_err(|fail| fail.into_error(&self.path))?;
        Ok(())
    }

    /// The paths of the notes whose title, text or tags hold every word of `text`, in no order.
    pub(crate) fn search(&mut self, text: &str) -> Result<Vec<NotePath>> {
        self.run(|conn| Ok(search::find(conn, text)?))
    }

    /// Every note with its links and its stamp, in no order.
    pub(crate) fn entries(&mut self) -> Result<Vec<(NoteInfo, Vec<Link>, Stamp)>> {
        self.run(|conn| {
            let mut reader = NoteReader::default();
            let mut statement = conn.prepare_cached(&format!(
                "SELECT {NOTE_INFO}, changed, size FROM notes ORDER BY key"
            ))?;
            let notes = statement
                .query_map([], |row| Ok((note_info(&mut reader, row)?, stamp(row)?)))?
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
                .map(|(info, stamp)| {
                    let links = links.remove(&info.path).unwrap_or_default();
                    (info, links, stamp)
                })
                .collect())
        })
    }

    /// Writes the rows of `settled` in one transaction, and returns the links of each put note,
    /// in order. A moved note keeps its row, so its text, tags and links stay.
    pub(crate) fn write(&mut self, settled: &Settled) -> Result<Vec<Vec<Link>>> {
        let write = |conn: &mut Connection| {
            let tx = conn.transaction()?;
            let links = put_rows(&tx, &self.root, settled)?;
            tx.commit()?;
            Ok(links)
        };
        write(&mut self.conn).map_err(|fail: Fail| fail.into_error(&self.path))
    }

    fn run<T>(&mut self, op: impl FnOnce(&mut Connection) -> Result<T, Fail>) -> Result<T> {
        op(&mut self.conn).map_err(|fail| fail.into_error(&self.path))
    }
}

/// An error inside this module. Only [`Fail::into_error`] turns it into an [`Error`].
enum Fail {
    Sql(rusqlite::Error),
    Io(io::Error),
}

impl Fail {
    fn into_error(self, path: &Path) -> Error {
        let source: Box<dyn std::error::Error + Send + Sync> = match self {
            Self::Sql(error) => Box::new(error),
            Self::Io(error) => Box::new(error),
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

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sql(error) => error.fmt(f),
            Self::Io(error) => error.fmt(f),
        }
    }
}

/// A new index with only the rows of `settled`: a new file, or else one in memory.
fn fresh(path: &Path, root: &Path, settled: &Settled) -> Result<Connection, Fail> {
    let file = remove(path).and_then(|()| {
        let mut conn = connect(path)?;
        fill(&mut conn, root, settled)?;
        Ok(conn)
    });
    file.or_else(|fail| {
        tracing::warn!(error = %fail, path = %path.display(), "Holm keeps the index in memory");
        let mut conn = Connection::open_in_memory()?;
        settings(&conn)?;
        schema(&mut conn)?;
        fill(&mut conn, root, settled)?;
        Ok(conn)
    })
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

/// Writes the rows of `settled` into the empty index.
fn fill(conn: &mut Connection, root: &Path, settled: &Settled) -> Result<(), Fail> {
    let tx = conn.transaction()?;
    put_rows(&tx, root, settled)?;
    tx.commit()?;
    Ok(())
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

/// Writes the rows of `settled`, and returns the links of each put note, in order.
fn put_rows(conn: &Connection, root: &Path, settled: &Settled) -> Result<Vec<Vec<Link>>, Fail> {
    for path in &settled.removed {
        delete(conn, path)?;
    }
    for moved in &settled.moved {
        shift(conn, &moved.from, &moved.info.path)?;
    }
    settled
        .put
        .iter()
        .map(|put| write(conn, root, &put.note, put.stamp, !put.known))
        .collect()
}

/// Moves the row of the note at `from` to `to`. Its key stays, so its text, tags and links
/// stay too.
fn shift(conn: &Connection, from: &NotePath, to: &NotePath) -> Result<(), Fail> {
    conn.prepare_cached("UPDATE notes SET path = ?2, folder = ?3 WHERE path = ?1")?
        .execute(params![bytes(from), bytes(to), bytes(to.folder())])?;
    Ok(())
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
            .query_row([bytes(&info.path)], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?
    };
    let (links, links_hash, stored) = match old {
        Some((old_source, old_links)) if old_source == source_hash => (None, old_links, true),
        _ => {
            let links = relink::links(root, note);
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
                bytes(&info.path),
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
        add_tag.execute(params![key, pos, tag.as_str()])?;
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

/// Removes the note at `rel` from the index.
fn delete(conn: &Connection, rel: &NotePath) -> Result<(), Fail> {
    let key: Option<i64> = conn
        .prepare_cached("SELECT key FROM notes WHERE path = ?1")?
        .query_row([bytes(rel)], |row| row.get(0))
        .optional()?;
    let Some(key) = key else {
        return Ok(());
    };
    search::remove(conn, key)?;
    conn.prepare_cached("DELETE FROM notes WHERE key = ?1")?
        .execute([key])?;
    Ok(())
}

fn note_info(reader: &mut NoteReader, row: &rusqlite::Row<'_>) -> rusqlite::Result<NoteInfo> {
    let tags: Option<String> = row.get(6)?;
    Ok(NoteInfo {
        path: note_path(reader, row, 0)?,
        id: row.get::<_, Option<String>>(1)?.map(NoteId::from_field),
        title: row.get(2)?,
        preview: row.get(5)?,
        tags: tags
            .map(|tags| {
                tags.split('\n')
                    .map(|tag| TagName::from_written(tag.to_owned()))
                    .collect()
            })
            .unwrap_or_default(),
        pinned: row.get(3)?,
        modified: time(row.get(4)?),
    })
}

/// The stamp of a row of [`NOTE_INFO`] with `changed` and `size` after it.
fn stamp(row: &rusqlite::Row<'_>) -> rusqlite::Result<Stamp> {
    Ok(Stamp {
        modified: row.get(4)?,
        changed: row.get(7)?,
        size: row.get(8)?,
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
