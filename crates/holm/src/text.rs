//! All UI text, in English. Other modules take their text from here.

use std::path::Path;

use crate::path_text::PathText;

pub(crate) const NOTES_FOLDER: &str = "Holm can't open the notes folder.";
pub(crate) const READ: &str = "Holm can't read a file in the notes folder.";
pub(crate) const NOT_UTF8: &str = "This file is not plain text, so Holm can't open it.";
pub(crate) const WRITE: &str =
    "Holm can't save the change. Check the free disk space and the folder permissions.";
pub(crate) const BAD_PATH: &str = "Holm can't use this name or place.";
pub(crate) const BAD_TAG: &str = "This is not a tag name.";
pub(crate) const NAME_TAKEN: &str = "This folder already has something with this name.";
pub(crate) const NAME_EMPTY: &str = "Type a name.";
pub(crate) const NAME_SLASH: &str = "A name can't hold “/”.";
pub(crate) const NAME_DOT: &str = "A name can't start with “.”.";
pub(crate) const NAME_ATTACHMENTS: &str = "The name “attachments” is for attached files.";
pub(crate) const NAME_TOO_LONG: &str = "This name is too long.";
pub(crate) const UNKNOWN: &str = "Something went wrong. The log has the details.";
pub(crate) const CONFIG_WRITE: &str = "Holm can't save the settings.";
pub(crate) const LOG: &str = "Holm can't write its log.";
pub(crate) const WINDOW: &str = "Holm can't open its window.";
pub(crate) const NO_HOME: &str = "Holm can't find your home folder.";

pub(crate) const TITLE: &str = "Holm";
pub(crate) const ASK_FOLDER: &str = "Where do your notes live?";
pub(crate) const ABOUT_FOLDER: &str = "Holm keeps each note as a Markdown file in one folder.";
pub(crate) const USE_DEFAULT: &str = "Use ~/Notes";
pub(crate) const PICK_FOLDER: &str = "Pick a folder";
pub(crate) const PICK_PROMPT: &str = "Use this folder";
pub(crate) const OPENING: &str = "Holm opens the notes folder.";
pub(crate) const CHANGED_ON_DISK: &str = "Changed on disk";
pub(crate) const RELOAD: &str = "Reload";
pub(crate) const KEEP_MINE: &str = "Keep mine";
pub(crate) const DELETED_ON_DISK: &str = "Deleted on disk";
pub(crate) const NOT_SAVED: &str = "Not saved";
/// The answer when a close finds text that is not saved.
pub(crate) const CLOSE_AGAIN: &str = "Not saved. Close again.";
pub(crate) const WRITING: &str = "Writing…";
/// The mark before a state in the message line.
pub(crate) const STATE_MARK: &str = "●";

/// The gray mark in the message line with the count of the problems.
pub(crate) fn problem_mark(count: usize) -> String {
    format!("⚠ {count}")
}

pub(crate) const NO_NOTE_OPEN: &str = "No note is open.";
pub(crate) const NEW_NOTE: &str = "New note";
pub(crate) const CLOSE: &str = "Close";

pub(crate) fn folder_missing(folder: &Path) -> PathText {
    PathText::default()
        .plain("Holm can't find the notes folder ")
        .path(home_path(folder))
        .plain(".")
}

/// `path` with `~` for the home folder.
pub(crate) fn home_path(path: &Path) -> String {
    match std::env::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_owned)) {
        Some(rest) => Path::new("~").join(rest).display().to_string(),
        None => path.display().to_string(),
    }
}

pub(crate) fn crashed(log: &Path) -> PathText {
    PathText::default()
        .plain("Holm closed after an error. The log is at ")
        .path(home_path(log))
        .plain(".")
}

pub(crate) const ALL_NOTES: &str = "All notes";
pub(crate) const NO_NOTES: &str = "No notes";
pub(crate) const PIN_MARK: &str = "●";
pub(crate) const PINNED: &str = "Pinned";
pub(crate) const TAGS_LABEL: &str = "Tags";
pub(crate) const ADD_TAG: &str = "Add tag";
pub(crate) const ADD_TAG_CHIP: &str = "+ Tag";
pub(crate) const ON_THIS_NOTE: &str = "On this note";
pub(crate) const NOTE_TITLE: &str = "Title";
/// The mark between two folders, or a folder and the title, in the tag row.
pub(crate) const PATH_GAP: &str = " › ";
/// The mark between the path and the backlinks count.
pub(crate) const PATH_DOT: &str = "·";

/// The line in the tag popup that adds a new tag.
pub(crate) fn add_tag_line(tag: &str) -> String {
    format!("Add #{tag}")
}
pub(crate) const RENAME: &str = "Rename";
pub(crate) const FOLDERS: &str = "FOLDERS";
pub(crate) const TAGS: &str = "TAGS";
pub(crate) const NEW_FOLDER: &str = "New folder";
/// The accessible name of a name field in the tree.
pub(crate) const NAME_FIELD: &str = "Name";
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

pub(crate) const LINK_TO_NOTE: &str = "Link to note";
pub(crate) const NO_HEADING: &str = "No heading matches";
pub(crate) const NOTE_GONE: &str = "Holm can't find the note of this link.";
pub(crate) const LINKED_FROM: &str = "Linked from";

/// The backlink count button in the tag row.
pub(crate) fn backlink_count(count: usize) -> String {
    if count == 1 {
        "← 1 note".to_owned()
    } else {
        format!("← {count} notes")
    }
}
pub(crate) const LINK_FILE: &str = "Link";
/// The link text of a pasted image, before its extension.
pub(crate) const PASTED_IMAGE: &str = "Pasted image";

/// The last row of the link picker, which makes a note.
pub(crate) fn create_note(title: &str) -> String {
    format!("Create “{title}”")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_in_the_home_folder_starts_with_a_tilde() {
        let home = std::env::home_dir().expect("a home folder");
        assert_eq!(
            home_path(&home.join(".config/holm/config.toml")),
            "~/.config/holm/config.toml"
        );
        assert_eq!(home_path(Path::new("/etc/holm.toml")), "/etc/holm.toml");
    }
}
