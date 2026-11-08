//! The SQLite index of notes, tags and links, with full-text search.
//!
//! The index is a copy of the files. Holm deletes it and rebuilds it when its shape
//! is old or when SQLite finds it broken.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use holm_markdown::{Body, Target};
use rusqlite::types::Value;
use rusqlite::{Connection, ErrorCode, OptionalExtension as _, params, params_from_iter};

use crate::error::{Error, Result};
use crate::model::{Link, LinkTarget, Note, NoteId, NoteInfo, Order, Tag};
use crate::store::{Stamp, Store, time};

const FILE: &str = "index.sqlite";
/// Change this number with each change to [`SCHEMA`]. An index with another number is rebuilt.
const SHAPE: i32 = 1;

const SCHEMA: &str = "
CREATE TABLE notes (
    key INTEGER PRIMARY KEY,
    path BLOB NOT NULL UNIQUE,
    folder BLOB NOT NULL,
    id TEXT UNIQUE,
    title TEXT NOT NULL,
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
SELECT path, id, title, pinned, modified,
    (SELECT group_concat(tag, char(10) ORDER BY pos) FROM tags WHERE note = notes.key)
FROM notes";

/// Which notes a list shows.
pub(crate) enum Filter<'a> {
    All,
    /// Only the notes right in this folder.
    Folder(&'a Path),
    /// The notes with this tag or one of its child tags.
    Tag(&'a str),
    /// The words the user typed.
    Search(&'a str),
    /// The notes that link to this note, by id or by path.
    Backlinks(&'a Path),
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
        let fail = |fail: Fail| fail.into_error(&path);
        fs::create_dir_all(dir).map_err(|e| fail(Fail::Io(e)))?;
        let conn = match connect(&path).and_then(|mut conn| {
            sync(&mut conn, store)?;
            Ok(conn)
        }) {
            Ok(conn) => conn,
            Err(Fail::Sql(error)) if broken(&error) => {
                tracing::warn!(%error, path = %path.display(), "the index is broken, so Holm rebuilds it");
                rebuild(&path, store).map_err(fail)?
            }
            Err(error) => return Err(fail(error)),
        };
        Ok(Self {
            path,
            conn: Mutex::new(conn),
        })
    }

    /// The notes that `filter` picks, in `order`.
    pub(crate) fn notes(
        &self,
        store: &Store,
        filter: &Filter<'_>,
        order: Order,
    ) -> Result<Vec<NoteInfo>> {
        let (filter, args): (&str, Vec<Value>) = match filter {
            Filter::All => ("", Vec::new()),
            Filter::Folder(folder) => ("WHERE folder = ?1", vec![bytes(folder)]),
            Filter::Tag(tag) => (
                "WHERE key IN (SELECT note FROM tags
                    WHERE tag = ?1 OR substr(tag, 1, length(?2)) = ?2)",
                vec![
                    Value::Text((*tag).to_owned()),
                    Value::Text(format!("{tag}/")),
                ],
            ),
            Filter::Search(text) => match match_query(text) {
                Some(query) => (
                    "WHERE key IN (SELECT rowid FROM search WHERE search MATCH ?1)",
                    vec![Value::Text(query)],
                ),
                None => return Ok(Vec::new()),
            },
            Filter::Backlinks(rel) => (
                "WHERE key IN (SELECT links.note FROM links, notes AS target
                    WHERE target.path = ?1 AND links.note != target.key
                    AND ((links.kind = 'note' AND links.target = target.id)
                        OR (links.kind = 'file' AND links.target = target.path)))",
                vec![bytes(rel)],
            ),
        };
        let sql = format!("{NOTE_INFO} {filter}");
        let mut notes = self.run(store, |conn| {
            let mut statement = conn.prepare_cached(&sql)?;
            let rows = statement.query_map(params_from_iter(&args), note_info)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })?;
        sort(&mut notes, order);
        Ok(notes)
    }

    /// Every tag as a tree, sorted by name.
    pub(crate) fn tags(&self, store: &Store) -> Result<Vec<Tag>> {
        let pairs = self.run(store, |conn| {
            let mut statement = conn.prepare_cached("SELECT tag, note FROM tags")?;
            let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            Ok(rows.collect::<rusqlite::Result<Vec<(String, i64)>>>()?)
        })?;
        Ok(tag_tree(&pairs))
    }

    /// The links of one note, in the order of the text.
    pub(crate) fn links(&self, store: &Store, rel: &Path) -> Result<Vec<Link>> {
        self.run(store, |conn| {
            let mut statement = conn.prepare_cached(
                "SELECT kind, target, heading, embedded FROM links
                JOIN notes ON notes.key = links.note WHERE notes.path = ?1 ORDER BY pos",
            )?;
            let rows = statement.query_map([bytes(rel)], link)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// Puts a note that Holm just read or wrote into the index, and returns it.
    ///
    /// When another note file has the same id, this note gets a new id.
    pub(crate) fn update(&self, store: &Store, note: &Note, stamp: Stamp) -> Result<Note> {
        self.run(store, |conn| {
            let tx = conn.transaction()?;
            let note = put(&tx, store, note.clone(), stamp)?;
            tx.commit()?;
            Ok(note)
        })
    }

    /// Runs `op`. When SQLite finds the index broken, it rebuilds the index and runs `op` again.
    fn run<T>(&self, store: &Store, op: impl Fn(&mut Connection) -> Result<T, Fail>) -> Result<T> {
        let mut conn = self.conn.lock().unwrap_or_else(PoisonError::into_inner);
        match op(&mut conn) {
            Err(Fail::Sql(error)) if broken(&error) => {
                tracing::warn!(%error, path = %self.path.display(), "the index is broken, so Holm rebuilds it");
                *conn = rebuild(&self.path, store).map_err(|fail| fail.into_error(&self.path))?;
                op(&mut conn)
            }
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

fn broken(error: &rusqlite::Error) -> bool {
    matches!(
        error.sqlite_error_code(),
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase)
    )
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
    let tx = conn.transaction()?;
    tx.execute_batch(SCHEMA)?;
    tx.pragma_update(None, "user_version", SHAPE)?;
    tx.commit()?;
    Ok(conn)
}

fn rebuild(path: &Path, store: &Store) -> Result<Connection, Fail> {
    remove(path)?;
    let mut conn = connect(path)?;
    sync(&mut conn, store)?;
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

/// Brings the index up to date with the files. It reads only the files whose stamp changed.
///
/// It handles the files the index knows before new ones. So a known file keeps its id
/// when a new file has the same id.
fn sync(conn: &mut Connection, store: &Store) -> Result<(), Fail> {
    let files = store.files()?;
    let tx = conn.transaction()?;
    let known: HashMap<PathBuf, Stamp> = {
        let mut statement = tx.prepare("SELECT path, modified, changed, size FROM notes")?;
        let rows = statement.query_map([], |row| {
            let stamp = Stamp {
                modified: row.get(1)?,
                changed: row.get(2)?,
                size: row.get(3)?,
            };
            Ok((path(row.get(0)?), stamp))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let on_disk: HashSet<&Path> = files.iter().map(|(rel, _)| rel.as_path()).collect();
    for rel in known.keys().filter(|rel| !on_disk.contains(rel.as_path())) {
        delete(&tx, rel)?;
    }
    let mut todo: Vec<(bool, &Path)> = files
        .iter()
        .filter(|(rel, stamp)| known.get(rel) != Some(stamp))
        .map(|(rel, _)| (!known.contains_key(rel), rel.as_path()))
        .collect();
    todo.sort_unstable();
    for (_, rel) in todo {
        match store.read(rel) {
            Ok((note, stamp)) => {
                put(&tx, store, note, stamp)?;
            }
            Err(error) => {
                tracing::warn!(%error, "skipped a note");
                delete(&tx, rel)?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

/// Writes one note into the index, after a check that no other file has its id.
fn put(conn: &Connection, store: &Store, mut note: Note, mut stamp: Stamp) -> Result<Note, Fail> {
    if let Some(id) = &note.info.id
        && let Some(owner) = owner(conn, id)?
        && owner != note.info.path
    {
        if store.exists(&owner) {
            let new = NoteId::generate();
            tracing::info!(%id, %new, path = %note.info.path.display(), "gave a copied note a new id");
            match store.set_id(&note.info.path, &new) {
                Ok((written, written_stamp)) => (note, stamp) = (written, written_stamp),
                Err(error) => {
                    tracing::warn!(%error, "kept a copied note without an id");
                    note.info.id = None;
                }
            }
        } else {
            delete(conn, &owner)?;
        }
    }
    write(conn, store.root(), &note, stamp)?;
    Ok(note)
}

fn owner(conn: &Connection, id: &NoteId) -> Result<Option<PathBuf>, Fail> {
    let owner = conn
        .prepare_cached("SELECT path FROM notes WHERE id = ?1")?
        .query_row([id.as_str()], |row| row.get(0))
        .optional()?;
    Ok(owner.map(path))
}

fn write(conn: &Connection, root: &Path, note: &Note, stamp: Stamp) -> Result<(), Fail> {
    let info = &note.info;
    let folder = info.path.parent().unwrap_or(Path::new(""));
    let key: i64 = conn
        .prepare_cached(
            "INSERT INTO notes (path, folder, id, title, pinned, modified, changed, size)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT (path) DO UPDATE SET folder = excluded.folder, id = excluded.id,
                title = excluded.title, pinned = excluded.pinned, modified = excluded.modified,
                changed = excluded.changed, size = excluded.size
            RETURNING key",
        )?
        .query_row(
            params![
                bytes(&info.path),
                bytes(folder),
                info.id.as_ref().map(NoteId::as_str),
                info.title,
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
    for (pos, link) in (0_i64..).zip(Body::parse(&note.body).links()) {
        let (kind, target, heading) = match &link.target {
            Target::Note { id, heading } => ("note", Value::Text(id.clone()), heading.as_deref()),
            Target::Heading(heading) => ("heading", Value::Text(heading.clone()), None),
            Target::File(file) => ("file", bytes(&resolve(root, &info.path, file)), None),
            Target::Web(address) => ("web", Value::Text(address.clone()), None),
        };
        add_link.execute(params![key, pos, kind, target, heading, link.embedded])?;
    }
    conn.prepare_cached("INSERT INTO search (rowid, title, body, tags) VALUES (?1, ?2, ?3, ?4)")?
        .execute(params![key, info.title, note.body, info.tags.join(" ")])?;
    Ok(())
}

fn delete(conn: &Connection, rel: &Path) -> Result<(), Fail> {
    conn.prepare_cached(
        "DELETE FROM search WHERE rowid = (SELECT key FROM notes WHERE path = ?1)",
    )?
    .execute([bytes(rel)])?;
    conn.prepare_cached("DELETE FROM notes WHERE path = ?1")?
        .execute([bytes(rel)])?;
    Ok(())
}

/// The path of a linked file: relative to the notes folder when it is inside it, else absolute.
fn resolve(root: &Path, note: &Path, file: &str) -> PathBuf {
    let file = Path::new(file);
    let full = if file.is_absolute() {
        file.to_owned()
    } else {
        root.join(note.parent().unwrap_or(Path::new(""))).join(file)
    };
    let mut clean = PathBuf::new();
    for part in full.components() {
        match part {
            Component::ParentDir => {
                clean.pop();
            }
            Component::CurDir => {}
            part => clean.push(part),
        }
    }
    match clean.strip_prefix(root) {
        Ok(rel) => rel.to_owned(),
        Err(_) => clean,
    }
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

/// Pinned notes first, then `order`. The path breaks ties, so the order never changes at random.
fn sort(notes: &mut [NoteInfo], order: Order) {
    match order {
        Order::LastChange => notes.sort_by(|a, b| {
            (b.pinned, b.modified)
                .cmp(&(a.pinned, a.modified))
                .then_with(|| a.path.cmp(&b.path))
        }),
        Order::Title => {
            notes.sort_by_cached_key(|note| {
                (!note.pinned, note.title.to_lowercase(), note.path.clone())
            });
        }
    }
}

/// Builds the tag tree from `(tag, note)` pairs. A parent tag counts the notes of its children.
fn tag_tree(pairs: &[(String, i64)]) -> Vec<Tag> {
    let mut notes: BTreeMap<&str, HashSet<i64>> = BTreeMap::new();
    for (tag, note) in pairs {
        let ends = tag.match_indices('/').map(|(i, _)| i).chain([tag.len()]);
        for end in ends {
            notes.entry(&tag[..end]).or_default().insert(*note);
        }
    }
    let mut children: HashMap<Option<&str>, Vec<&str>> = HashMap::new();
    for name in notes.keys() {
        let parent = name.rsplit_once('/').map(|(parent, _)| parent);
        children.entry(parent).or_default().push(name);
    }
    tag_children(None, &notes, &children)
}

/// The tags under `parent`, with their children.
fn tag_children(
    parent: Option<&str>,
    notes: &BTreeMap<&str, HashSet<i64>>,
    children: &HashMap<Option<&str>, Vec<&str>>,
) -> Vec<Tag> {
    let mut tags: Vec<Tag> = children
        .get(&parent)
        .into_iter()
        .flatten()
        .map(|name| Tag {
            name: (*name).to_owned(),
            notes: notes.get(name).map_or(0, HashSet::len),
            children: tag_children(Some(name), notes, children),
        })
        .collect();
    tags.sort_by_cached_key(|tag| (tag.name.to_lowercase(), tag.name.clone()));
    tags
}

fn note_info(row: &rusqlite::Row<'_>) -> rusqlite::Result<NoteInfo> {
    let tags: Option<String> = row.get(5)?;
    Ok(NoteInfo {
        path: path(row.get(0)?),
        id: row.get::<_, Option<String>>(1)?.map(NoteId::from_field),
        title: row.get(2)?,
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
