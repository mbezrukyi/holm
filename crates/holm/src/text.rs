//! All UI text, in English. Other modules take their text from here.

use std::path::Path;

pub(crate) const NOTES_FOLDER: &str = "Holm can't open the notes folder.";
pub(crate) const READ: &str = "Holm can't read a file in the notes folder.";
pub(crate) const NOT_UTF8: &str = "This file is not plain text, so Holm can't open it.";
pub(crate) const WRITE: &str =
    "Holm can't save the change. Check the free disk space and the folder permissions.";
pub(crate) const INDEX: &str = "Holm can't use its search index.";
pub(crate) const BAD_PATH: &str = "Holm can't use this name or place.";
pub(crate) const BAD_TAG: &str = "This is not a tag name.";
pub(crate) const UNKNOWN: &str = "Something went wrong. The log has the details.";
pub(crate) const CONFIG_WRITE: &str = "Holm can't save the settings.";
pub(crate) const LOG: &str = "Holm can't write its log.";
pub(crate) const WINDOW: &str = "Holm can't open its window.";
pub(crate) const NO_HOME: &str = "Holm can't find your home folder.";
pub(crate) const CONFIG_BROKEN: &str =
    "The config file has an error. Holm uses the default settings and does not change the file.";

pub(crate) const TITLE: &str = "Holm";
pub(crate) const ASK_FOLDER: &str = "Where do your notes live?";
pub(crate) const ABOUT_FOLDER: &str = "Holm keeps each note as a Markdown file in one folder.";
pub(crate) const USE_DEFAULT: &str = "Use ~/Notes";
pub(crate) const PICK_FOLDER: &str = "Pick a folder";
pub(crate) const PICK_PROMPT: &str = "Use this folder";
pub(crate) const OPENING: &str = "Holm opens the notes folder.";
pub(crate) const CHANGED_ON_DISK: &str = "Changed on disk. Reload or keep mine?";
pub(crate) const RELOAD: &str = "Reload";
pub(crate) const KEEP_MINE: &str = "Keep mine";
pub(crate) const CLOSE: &str = "Close";

pub(crate) fn folder_missing(folder: &Path) -> String {
    format!("Holm can't find the notes folder {}.", folder.display())
}

pub(crate) fn crashed(log: &Path) -> String {
    format!(
        "Holm closed after an error. The log is at {}.",
        log.display()
    )
}

pub(crate) const ALL_NOTES: &str = "All notes";
pub(crate) const NO_NOTES: &str = "No notes";
pub(crate) const PIN_MARK: &str = "●";
pub(crate) const PINNED: &str = "Pinned";
pub(crate) const TAGS_LABEL: &str = "Tags";
pub(crate) const ADD_TAG: &str = "Add tag";
pub(crate) const RENAME: &str = "Rename";
pub(crate) const FOLDERS: &str = "FOLDERS";
pub(crate) const TAGS: &str = "TAGS";
pub(crate) const NEW_FOLDER: &str = "New folder";
pub(crate) const ROOT_FOLDER: &str = "Notes folder";
pub(crate) const MOVE_TO: &str = "Move to";
pub(crate) const DELETE: &str = "Delete";
pub(crate) const CANCEL: &str = "Cancel";
pub(crate) const DO_NOT_ASK: &str = "Do not show again";

/// The question of the delete dialog.
pub(crate) fn delete_question(name: &str, notes: usize, files: usize) -> String {
    let notes = count(notes, "note", "notes");
    let what = match files {
        0 => notes,
        files => format!("{notes} and {}", count(files, "file", "files")),
    };
    format!("Delete “{name}”? This deletes {what} forever.")
}

fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// The question before a tag rename in all notes.
pub(crate) fn rename_tag_question(from: &str, to: &str, notes: usize) -> String {
    format!(
        "Rename “#{from}” to “#{to}”? This changes {}.",
        count(notes, "note", "notes")
    )
}

/// The question before a tag delete in all notes.
pub(crate) fn delete_tag_question(tag: &str, notes: usize) -> String {
    format!(
        "Remove “#{tag}” from all notes? This changes {}.",
        count(notes, "note", "notes")
    )
}

pub(crate) const SEARCH: &str = "Search";
pub(crate) const QUICK_OPEN: &str = "Open a note";
pub(crate) const NO_MATCH: &str = "No note has this title";

/// The head of the note list with search results.
pub(crate) fn search_header(query: &str) -> String {
    format!("Search: {query}")
}
