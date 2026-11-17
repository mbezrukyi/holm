//! Full-text search over the titles, texts and tags of the notes in the index.
//!
//! The text of a note and its tags have their own word tables. The tag words are kept once
//! per tag name, not per note. So a change of tags never parses the text of a note again,
//! and a tag rename on many notes adds one row.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use rusqlite::{Connection, OptionalExtension as _, params};

use super::{hash, path};

/// The tables of this module. `tags` and `notes` belong to the index.
pub(super) const SCHEMA: &str = "
CREATE TABLE text_hashes (
    note INTEGER PRIMARY KEY REFERENCES notes ON DELETE CASCADE,
    hash INTEGER NOT NULL
);
CREATE VIRTUAL TABLE text_words USING fts5 (
    title, body,
    content = '', contentless_delete = 1,
    tokenize = 'unicode61 remove_diacritics 2'
);
CREATE TABLE tag_names (
    key INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);
CREATE VIRTUAL TABLE tag_words USING fts5 (
    name,
    tokenize = 'unicode61 remove_diacritics 2'
);
";

/// The most words a search uses. Later words are left out.
const MAX_WORDS: usize = 64;

/// Makes the note with `key` findable by its title, its body and its tags.
///
/// The note's rows in the `tags` table must be written already.
pub(super) fn put(
    conn: &Connection,
    key: i64,
    title: &str,
    body: &str,
    tags: &[String],
) -> rusqlite::Result<()> {
    let hash = hash((title, body));
    let old: Option<i64> = conn
        .prepare_cached("SELECT hash FROM text_hashes WHERE note = ?1")?
        .query_row([key], |row| row.get(0))
        .optional()?;
    // A new row parses the whole body, so it is written only when the text changed.
    if old != Some(hash) {
        if old.is_some() {
            conn.prepare_cached("DELETE FROM text_words WHERE rowid = ?1")?
                .execute([key])?;
        }
        conn.prepare_cached("INSERT INTO text_words (rowid, title, body) VALUES (?1, ?2, ?3)")?
            .execute(params![key, title, body])?;
        conn.prepare_cached("INSERT OR REPLACE INTO text_hashes (note, hash) VALUES (?1, ?2)")?
            .execute([key, hash])?;
    }
    for tag in tags {
        let added = conn
            .prepare_cached("INSERT INTO tag_names (name) VALUES (?1) ON CONFLICT DO NOTHING")?
            .execute([tag])?;
        if added > 0 {
            conn.prepare_cached("INSERT INTO tag_words (rowid, name) VALUES (?1, ?2)")?
                .execute(params![conn.last_insert_rowid(), tag])?;
        }
    }
    Ok(())
}

/// Removes the words of the note with `key`. Call it before the note leaves `notes`.
///
/// Its tag names stay. A name with no note finds nothing.
pub(super) fn remove(conn: &Connection, key: i64) -> rusqlite::Result<()> {
    conn.prepare_cached("DELETE FROM text_words WHERE rowid = ?1")?
        .execute([key])?;
    Ok(())
}

/// The paths of the notes whose title, text or tags hold every word of `text`, in no order.
pub(super) fn find(conn: &Connection, text: &str) -> rusqlite::Result<Vec<PathBuf>> {
    let words = words(text);
    if words.is_empty() {
        return Ok(Vec::new());
    }
    let mut found = text_matches(conn, &words.join(" "))?;
    let mut tagged = tag_matches(conn, &words)?;
    if tagged.is_empty() {
        return Ok(found.into_iter().map(|(_, path)| path).collect());
    }
    for (key, _) in &found {
        tagged.remove(key);
    }
    // Notes whose tags hold the same words need the same other words in their text.
    let mut groups: HashMap<u64, HashSet<i64>> = HashMap::new();
    for (key, mask) in tagged {
        groups.entry(mask).or_default().insert(key);
    }
    for (mask, keys) in groups {
        let rest: Vec<&str> = (0..words.len())
            .filter(|n| mask & (1 << n) == 0)
            .map(|n| words[n].as_str())
            .collect();
        if rest.is_empty() {
            let keys: Vec<String> = keys.iter().map(i64::to_string).collect();
            let mut statement = conn.prepare_cached(
                "SELECT key, path FROM notes WHERE key IN (SELECT value FROM json_each(?1))",
            )?;
            let rows = statement.query_map([format!("[{}]", keys.join(","))], |row| {
                Ok((row.get(0)?, path(row.get(1)?)))
            })?;
            for row in rows {
                found.push(row?);
            }
        } else {
            let matches = text_matches(conn, &rest.join(" "))?;
            found.extend(matches.into_iter().filter(|(key, _)| keys.contains(key)));
        }
    }
    Ok(found.into_iter().map(|(_, path)| path).collect())
}

/// The key and path of each note whose title or text matches the FTS5 `query`.
fn text_matches(conn: &Connection, query: &str) -> rusqlite::Result<Vec<(i64, PathBuf)>> {
    let mut statement = conn.prepare_cached(
        "SELECT key, path FROM notes
        WHERE key IN (SELECT rowid FROM text_words WHERE text_words MATCH ?1)",
    )?;
    let rows = statement.query_map([query], |row| Ok((row.get(0)?, path(row.get(1)?))))?;
    rows.collect()
}

/// Each note with a tag that holds one of `words`, with the words its tags hold as bits.
fn tag_matches(conn: &Connection, words: &[String]) -> rusqlite::Result<HashMap<i64, u64>> {
    let mut tagged: HashMap<i64, u64> = HashMap::new();
    let any = conn
        .prepare_cached("SELECT 1 FROM tag_words WHERE tag_words MATCH ?1 LIMIT 1")?
        .exists([words.join(" OR ")])?;
    if !any {
        return Ok(tagged);
    }
    let mut names = conn.prepare_cached(
        "SELECT name FROM tag_names
        WHERE key IN (SELECT rowid FROM tag_words WHERE tag_words MATCH ?1)",
    )?;
    let mut notes = conn.prepare_cached("SELECT note FROM tags WHERE tag = ?1")?;
    for (n, word) in words.iter().enumerate() {
        let bit = 1 << n;
        let names = names
            .query_map([word], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for name in names {
            for key in notes.query_map([name], |row| row.get::<_, i64>(0))? {
                *tagged.entry(key?).or_default() |= bit;
            }
        }
    }
    Ok(tagged)
}

/// The words the user typed as FTS5 phrases. Each word matches as a prefix.
fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .take(MAX_WORDS)
        .map(|word| format!("\"{}\"*", word.replace('"', "\"\"")))
        .collect()
}
