//! The index is a copy of the files, for a fast start and for search. It never reads or writes a note file. When SQLite
//! fails on it or its shape is old, it starts again empty. When a new file fails too, the index lives in memory until
//! Holm ends.

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
use crate::facts::NoteFacts;
use crate::links::LinkSource;
use crate::model::{Link, LinkTarget, Note, NoteId, NoteInfo};
use crate::paths::{NotePath, NoteReader};
use crate::store::{ContentHash, Stamp, time};
use crate::tag::TagName;

mod search;

const FILE: &str = "index.sqlite";
/// Change this number with each change to [`SCHEMA`], to [`search::SCHEMA`], or to what a column holds. An index with
/// another number is rebuilt.
const SHAPE: i32 = 11;

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
    content_hash INTEGER NOT NULL,
    link_source INTEGER NOT NULL,
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
    PRIMARY KEY (note, pos)
);
CREATE INDEX links_target ON links (kind, target);
";

/// The columns that [`note_info`] reads, from the table `notes`.
const NOTE_INFO: &str = "
path, id, title, pinned, modified, preview,
    (SELECT group_concat(tag, char(10) ORDER BY pos) FROM tags WHERE note = notes.key)";

#[derive(Debug)]
pub(crate) struct Index {
    path: PathBuf,
    conn: Connection,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct NoteRows<'a> {
    pub(crate) note: &'a Note,
    pub(crate) links: &'a [Link],
    pub(crate) stamp: Stamp,
    pub(crate) hash: ContentHash,
    pub(crate) source: LinkSource,
}

#[derive(Debug)]
pub(crate) struct Rows<'a> {
    pub(crate) removed: &'a [NotePath],
    /// The notes at a new path with no other change, as old path and new path.
    pub(crate) moved: Vec<(&'a NotePath, &'a NotePath)>,
    pub(crate) put: Vec<NoteRows<'a>>,
}

impl Index {
    /// When SQLite fails on the index, it starts empty.
    pub(crate) fn open(dir: &Path) -> Result<Self> {
        let path = dir.join(FILE);
        let opened = fs::create_dir_all(dir).map_err(Fail::Io).and_then(|()| connect(&path));
        let conn = match opened {
            Ok(conn) => conn,
            Err(fail) => {
                tracing::warn!(error = %fail, path = %path.display(), "the index failed, so it starts empty");
                fresh(&path, &[]).map_err(|fail| fail.into_error(&path))?
            }
        };
        Ok(Self { path, conn })
    }

    /// Drops every row and writes `put`, which must hold every note file. The rows go into a new file, or into memory
    /// when the new file fails.
    pub(crate) fn rebuild(&mut self, put: &[NoteRows<'_>]) -> Result<()> {
        self.conn = fresh(&self.path, put).map_err(|fail| fail.into_error(&self.path))?;
        Ok(())
    }

    /// The paths of the notes whose title, text or tags hold every word of `text`, in no order.
    pub(crate) fn search(&mut self, text: &str) -> Result<Vec<NotePath>> {
        self.run(|conn| Ok(search::find(conn, text)?))
    }

    pub(crate) fn entries(&mut self) -> Result<Vec<NoteFacts>> {
        self.run(|conn| {
            let mut reader = NoteReader::default();
            let mut statement = conn.prepare_cached(&format!(
                "SELECT {NOTE_INFO}, changed, size, link_source, content_hash FROM notes ORDER BY key"
            ))?;
            let notes = statement
                .query_map([], |row| {
                    let source = LinkSource::from_stored(row.get(9)?);
                    let hash = ContentHash::from_stored(row.get(10)?);
                    Ok((note_info(&mut reader, row)?, stamp(row)?, hash, source))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut statement = conn.prepare_cached(
                "SELECT kind, target, heading, notes.path FROM links
                JOIN notes ON notes.key = links.note ORDER BY links.note, pos",
            )?;
            let mut links: HashMap<NotePath, Vec<Link>> = HashMap::new();
            let rows = statement.query_map([], |row| Ok((note_path(&mut reader, row, 3)?, link(row)?)))?;
            for row in rows {
                let (note, link) = row?;
                links.entry(note).or_default().push(link);
            }
            Ok(notes
                .into_iter()
                .map(|(info, stamp, hash, source)| {
                    let links = links.remove(&info.path).unwrap_or_default();
                    (info, links, stamp, hash, source)
                })
                .collect())
        })
    }

    /// Writes `rows` in one transaction. A moved note keeps its row, so its text, tags and links stay.
    pub(crate) fn write(&mut self, rows: &Rows<'_>) -> Result<()> {
        self.run(|conn| {
            let tx = conn.transaction()?;
            for path in rows.removed {
                delete(&tx, path)?;
            }
            for (from, to) in &rows.moved {
                shift(&tx, from, to)?;
            }
            put_rows(&tx, &rows.put)?;
            tx.commit()?;
            Ok(())
        })
    }

    fn run<T>(&mut self, op: impl FnOnce(&mut Connection) -> Result<T, Fail>) -> Result<T> {
        op(&mut self.conn).map_err(|fail| fail.into_error(&self.path))
    }
}

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

/// A new index with only the rows `put`: a new file, or else one in memory.
fn fresh(path: &Path, put: &[NoteRows<'_>]) -> Result<Connection, Fail> {
    let file = remove(path).and_then(|()| {
        let mut conn = connect(path)?;
        fill(&mut conn, put)?;
        Ok(conn)
    });
    file.or_else(|fail| {
        tracing::warn!(error = %fail, path = %path.display(), "Holm keeps the index in memory");
        let mut conn = Connection::open_in_memory()?;
        settings(&conn)?;
        schema(&mut conn)?;
        fill(&mut conn, put)?;
        Ok(conn)
    })
}

/// A new file gets the schema. A file with another shape is deleted first.
fn connect(path: &Path) -> Result<Connection, Fail> {
    let mut conn = Connection::open(path)?;
    conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
    conn.execute_batch("PRAGMA synchronous = NORMAL;")?;
    settings(&conn)?;
    let shape: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if shape == SHAPE {
        return Ok(conn);
    }
    let empty: bool = conn.query_row("SELECT count(*) = 0 FROM sqlite_schema", [], |row| row.get(0))?;
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
    // Without it, SQLite prepares some statements again for each new bound value, like the read of the link rows of a
    // note.
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

/// Writes `put` into the empty index.
fn fill(conn: &mut Connection, put: &[NoteRows<'_>]) -> Result<(), Fail> {
    let tx = conn.transaction()?;
    put_rows(&tx, put)?;
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

fn put_rows(conn: &Connection, put: &[NoteRows<'_>]) -> Result<(), Fail> {
    for row in put {
        write(conn, row)?;
    }
    Ok(())
}

/// The key of the row stays, so the text, tags and links of the note stay too.
fn shift(conn: &Connection, from: &NotePath, to: &NotePath) -> Result<(), Fail> {
    conn.prepare_cached("UPDATE notes SET path = ?2, folder = ?3 WHERE path = ?1")?
        .execute(params![bytes(from), bytes(to), bytes(to.folder())])?;
    Ok(())
}

fn write(conn: &Connection, row: &NoteRows<'_>) -> Result<(), Fail> {
    let NoteRows {
        note,
        links,
        stamp,
        hash: content_hash,
        source,
    } = *row;
    let info = &note.info;
    let folder = info.path.folder();
    let links_hash = hash(links);
    let old_links_hash: Option<i64> = conn
        .prepare_cached("SELECT links_hash FROM notes WHERE path = ?1")?
        .query_row([bytes(&info.path)], |row| row.get(0))
        .optional()?;
    let key: i64 = conn
        .prepare_cached(
            "INSERT INTO notes (path, folder, id, title, preview, pinned, modified, changed, size,
                content_hash, link_source, links_hash)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            ON CONFLICT (path) DO UPDATE SET folder = excluded.folder, id = excluded.id,
                title = excluded.title, preview = excluded.preview, pinned = excluded.pinned, modified = excluded.modified,
                changed = excluded.changed, size = excluded.size,
                content_hash = excluded.content_hash, link_source = excluded.link_source, links_hash = excluded.links_hash
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
                content_hash.stored(),
                source.stored(),
                links_hash,
            ],
            |row| row.get(0),
        )?;
    conn.prepare_cached("DELETE FROM tags WHERE note = ?1")?
        .execute([key])?;
    let mut add_tag = conn.prepare_cached("INSERT INTO tags (note, pos, tag) VALUES (?1, ?2, ?3)")?;
    for (pos, tag) in (0_i64..).zip(&info.tags) {
        add_tag.execute(params![key, pos, tag.as_str()])?;
    }
    if old_links_hash != Some(links_hash) {
        write_links(conn, key, links)?;
    }
    search::put(conn, key, &info.title, &note.body, &info.tags)?;
    Ok(())
}

fn write_links(conn: &Connection, key: i64, links: &[Link]) -> Result<(), Fail> {
    conn.prepare_cached("DELETE FROM links WHERE note = ?1")?
        .execute([key])?;
    let mut add_link = conn.prepare_cached(
        "INSERT INTO links (note, pos, kind, target, heading)
        VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for (pos, link) in (0_i64..).zip(links) {
        let (kind, target, heading) = match &link.target {
            LinkTarget::Note { id, heading } => ("note", Value::Text(id.as_str().to_owned()), heading.as_deref()),
            LinkTarget::Heading(heading) => ("heading", Value::Text(heading.clone()), None),
            LinkTarget::File(file) => ("file", bytes(file), None),
            LinkTarget::Web(address) => ("web", Value::Text(address.clone()), None),
        };
        add_link.execute(params![key, pos, kind, target, heading])?;
    }
    Ok(())
}

/// Two values with the same hash count as the same.
///
/// The hash may change with the `foldhash` version. Then the index writes the rows of each note once more.
fn hash(value: impl Hash) -> i64 {
    let hash = foldhash::quality::FixedState::default().hash_one(value);
    i64::from_ne_bytes(hash.to_ne_bytes())
}

fn delete(conn: &Connection, path: &NotePath) -> Result<(), Fail> {
    let key: Option<i64> = conn
        .prepare_cached("SELECT key FROM notes WHERE path = ?1")?
        .query_row([bytes(path)], |row| row.get(0))
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
    Ok(Link { target })
}

fn bytes(path: impl AsRef<Path>) -> Value {
    Value::Blob(path.as_ref().as_os_str().as_bytes().to_vec())
}

fn path(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(OsString::from_vec(bytes))
}

/// A path that is not a note path fails like a broken row, so the index gets rebuilt.
fn note_path(reader: &mut NoteReader, row: &rusqlite::Row<'_>, column: usize) -> rusqlite::Result<NotePath> {
    let fail = |error| rusqlite::Error::FromSqlConversionFailure(column, Type::Blob, error);
    let bytes = row.get_ref(column)?.as_blob().map_err(|e| fail(e.into()))?;
    reader.note(bytes).ok_or_else(|| fail("not a note path".into()))
}
