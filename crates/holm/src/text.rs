//! All UI text, in English. Other modules take their text from here.

use std::path::Path;

use holm_core::{FolderPath, TagName};

use crate::path_text::PathText;
use crate::scale::{Scale, Size};

pub(crate) const NOTES_FOLDER: &str = "Holm can't open the notes folder.";
pub(crate) const READ: &str = "Holm can't read a file in the notes folder.";
pub(crate) const NOT_UTF8: &str = "This file is not plain text, so Holm can't open it.";
pub(crate) const WRITE: &str = "Holm can't save the change. Check the free disk space and the folder permissions.";
pub(crate) const BAD_PATH: &str = "Holm can't use this name or place.";
pub(crate) const BAD_TAG: &str = "This is not a tag name.";
pub(crate) const NAME_TAKEN: &str = "This folder already has something with this name.";
pub(crate) const NAME_EMPTY: &str = "Type a name.";
pub(crate) const NAME_SLASH: &str = "A name can't hold “/”.";
pub(crate) const NAME_DOT: &str = "A name can't start with “.”.";
pub(crate) const NAME_ATTACHMENTS: &str = "The name “attachments” is for attached files.";
pub(crate) const NAME_TOO_LONG: &str = "This name is too long.";
pub(crate) const NAME_ZERO_BYTE: &str = "A name can't hold a zero byte.";
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
pub(crate) const CHANGED_REASON: &str = "Another app changed the file. Your text is still here.";
pub(crate) const RELOAD: &str = "Reload";
pub(crate) const KEEP_MINE: &str = "Keep mine";
pub(crate) const DELETED_ON_DISK: &str = "Deleted on disk";
pub(crate) const DELETED_REASON: &str = "Another app deleted the file. Your text is still here.";
pub(crate) const NOT_SAVED: &str = "Not saved";
/// The message when a close finds text that is not saved.
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

/// A tag as the screen shows it: `#work/holm`.
pub(crate) fn tag_label(tag: &TagName) -> String {
    format!("#{tag}")
}

/// The line in the tag popup that adds a new tag.
pub(crate) fn add_tag_line(tag: &TagName) -> String {
    format!("Add {}", tag_label(tag))
}
pub(crate) const RENAME: &str = "Rename";
pub(crate) const FOLDERS: &str = "FOLDERS";
pub(crate) const TAGS: &str = "TAGS";
pub(crate) const NEW_FOLDER: &str = "New folder";
/// The accessible name of a name field in the tree.
pub(crate) const NAME_FIELD: &str = "Name";
const ROOT_FOLDER: &str = "Notes folder";

/// A folder as the screen shows it: its path, or a name for the root folder.
pub(crate) fn folder_label(folder: &FolderPath) -> String {
    if folder.is_root() {
        ROOT_FOLDER.to_owned()
    } else {
        folder.to_string()
    }
}
pub(crate) const MOVE_TO: &str = "Move to";
pub(crate) const NO_FOLDER: &str = "No folder has this name";

/// The field of the move-to picker, for the note or folder `name`.
pub(crate) fn move_to(name: &str) -> String {
    format!("Move “{name}” to…")
}
pub(crate) const DELETE: &str = "Delete";
pub(crate) const CANCEL: &str = "Cancel";
pub(crate) const DO_NOT_ASK: &str = "Do not show again";

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
pub(crate) fn rename_tag_question(from: &TagName, to: &TagName, notes: usize) -> String {
    format!(
        "Rename “{}” to “{}”? This changes {}.",
        tag_label(from),
        tag_label(to),
        count(notes, "note", "notes")
    )
}

/// The question before a tag delete in all notes.
pub(crate) fn delete_tag_question(tag: &TagName, notes: usize) -> String {
    format!(
        "Remove “{}” from all notes? This changes {}.",
        tag_label(tag),
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

pub(crate) const SETTINGS: &str = "Settings";
/// The answer to a zoom key.
pub(crate) fn scale_answer(scale: Scale) -> String {
    let percent = scale.percent();
    if scale.at_end(Size::Smaller) {
        format!("Scale {percent} %, the smallest")
    } else if scale.at_end(Size::Bigger) {
        format!("Scale {percent} %, the largest")
    } else {
        format!("Scale {percent} %")
    }
}

pub(crate) const SETTING_FOLDER: &str = "Notes folder";
pub(crate) const SETTING_THEME: &str = "Theme";
pub(crate) const THEME_SYSTEM: &str = "System";
pub(crate) const THEME_LIGHT: &str = "Light";
pub(crate) const THEME_DARK: &str = "Dark";
pub(crate) const SETTING_SCALE: &str = "Scale";
pub(crate) const SETTING_TEXT_WIDTH: &str = "Text width";
pub(crate) const WIDTH_FULL: &str = "Full";
pub(crate) const WIDTH_LIMITED: &str = "Limited";
/// The accessible name of the width field.
pub(crate) const TEXT_WIDTH_CHARS: &str = "Text width in chars";
pub(crate) const CHARS_UNIT: &str = "chars";
pub(crate) const SETTING_ORDER: &str = "Note order";
pub(crate) const ORDER_LAST_CHANGE: &str = "Last change";
pub(crate) const ORDER_TITLE: &str = "Title";
pub(crate) const SETTING_FORMAT_KEYS: &str = "Format keys";
pub(crate) const SETTING_CONTINUE_LISTS: &str = "Continue lists on Enter";
pub(crate) const SETTING_INDENT_LISTS: &str = "Indent lists with Tab";

pub(crate) const APP_MENU: &str = "App menu";
pub(crate) const MENU_SETTINGS: &str = "Settings";
pub(crate) const MENU_SHORTCUTS: &str = "Shortcuts";
pub(crate) const MENU_PIN: &str = "Pin note";
pub(crate) const MENU_ABOUT: &str = "About Holm";
pub(crate) const MENU_QUIT: &str = "Quit";

pub(crate) const KEY_NEW_NOTE: &str = "New note";
pub(crate) const KEY_NEW_FOLDER: &str = "New folder";
pub(crate) const KEY_QUICK_OPEN: &str = "Open a note by its title";
pub(crate) const KEY_TOGGLE_SIDEBAR: &str = "Show or hide the sidebar";
pub(crate) const KEY_SEARCH: &str = "Search the notes";
pub(crate) const KEY_DISK_STATE: &str = "Open the state menu";
pub(crate) const KEY_PIN: &str = "Pin or unpin the note";
pub(crate) const KEY_TAGS: &str = "Edit the tags";
pub(crate) const KEY_BACKLINKS: &str = "Show the backlinks";
pub(crate) const KEY_PROBLEMS: &str = "Show or hide the problem list";
pub(crate) const KEY_SETTINGS: &str = "Settings";
pub(crate) const KEY_SHORTCUT_LIST: &str = "Shortcuts";
pub(crate) const KEY_QUIT: &str = "Quit";
pub(crate) const KEY_APP_MENU: &str = "App menu";
pub(crate) const KEY_SCALE_UP: &str = "Make everything bigger";
pub(crate) const KEY_SCALE_DOWN: &str = "Make everything smaller";
pub(crate) const KEY_EDIT_TITLE: &str = "Rename the note";
pub(crate) const KEY_LINK_NOTE: &str = "Link a note";
pub(crate) const KEY_LINK_FILES: &str = "Attach files";
pub(crate) const KEY_LINK_FOLDERS: &str = "Link folders";
pub(crate) const KEY_MOVE: &str = "Move to a folder";
pub(crate) const KEY_NEXT_ITEM: &str = "Next item";
pub(crate) const KEY_PREVIOUS_ITEM: &str = "Previous item";
pub(crate) const KEY_OPEN_CLOSE_ITEM: &str = "Open or close the item";
pub(crate) const KEY_NEXT_NOTE: &str = "Open the next note";
pub(crate) const KEY_PREVIOUS_NOTE: &str = "Open the previous note";
pub(crate) const KEY_SHOW_NOTES: &str = "Show the notes";
pub(crate) const KEY_FOCUS_EDITOR: &str = "Go to the note";
pub(crate) const KEY_BACK: &str = "Clear the search or go back";
pub(crate) const KEY_RENAME_ITEM: &str = "Rename the folder or tag";
pub(crate) const KEY_DELETE_ITEM: &str = "Delete the folder or tag";
pub(crate) const KEY_RENAME_NOTE: &str = "Rename the note";
pub(crate) const KEY_DELETE_NOTE: &str = "Delete the note";
pub(crate) const KEY_RENAME_TAG: &str = "Rename the tag";
pub(crate) const KEY_SWITCH_TAG: &str = "Add or remove the tag";
pub(crate) const KEY_PREVIOUS_TAG: &str = "Previous tag";
pub(crate) const KEY_NEXT_TAG: &str = "Next tag";
pub(crate) const KEY_SHOW_RESULTS: &str = "Go to the results";
pub(crate) const KEY_CLEAR_SEARCH: &str = "Clear the search";
pub(crate) const KEY_PREVIOUS_ROW: &str = "Previous row";
pub(crate) const KEY_NEXT_ROW: &str = "Next row";
pub(crate) const KEY_YES: &str = "Yes";
pub(crate) const KEY_NO: &str = "No";
pub(crate) const KEY_SWITCH_ASK: &str = "Switch “Do not show again”";
pub(crate) const KEY_PICK: &str = "Pick the row";
pub(crate) const KEY_CLOSE: &str = "Close";
pub(crate) const KEY_CONFIRM: &str = "Keep the text";
pub(crate) const KEY_CANCEL: &str = "Drop the text";
pub(crate) const KEY_NEXT_SETTING: &str = "Next setting";
pub(crate) const KEY_PREVIOUS_SETTING: &str = "Previous setting";
pub(crate) const KEY_SWITCH_SETTING: &str = "Switch a setting";
pub(crate) const KEY_PREVIOUS_OPTION: &str = "Previous option";
pub(crate) const KEY_SCROLL_DOWN: &str = "Scroll down";
pub(crate) const KEY_SCROLL_UP: &str = "Scroll up";
pub(crate) const KEY_CHANGE_FOLDER: &str = "Change the notes folder";
pub(crate) const KEY_NEXT_LINK: &str = "Next link";
pub(crate) const KEY_PREVIOUS_LINK: &str = "Previous link";
pub(crate) const KEY_OPEN_LINK: &str = "Open the link";

pub(crate) const ABOUT: &str = "About Holm";
pub(crate) fn version(version: &str) -> String {
    format!("Version {version}")
}
pub(crate) const HOLM: &str = "Holm";
pub(crate) const COPYRIGHT: &str = "© 2026 Mykola Bezrukyi";
pub(crate) const LICENCE: &str = "Free software under the GNU GPL, version 3 or later.";
pub(crate) const NO_WARRANTY: &str = "It comes with no warranty.";
pub(crate) const LICENCE_LINK: &str = "Licence";
pub(crate) const NOTICES_LINK: &str = "Third-party notices";

pub(crate) const SHORTCUT_LIST: &str = "Shortcuts";
pub(crate) const GROUP_TEXT: &str = "TEXT";
pub(crate) const GROUP_NOTE: &str = "NOTE";
pub(crate) const GROUP_SIDEBAR: &str = "SIDEBAR";
pub(crate) const GROUP_WINDOW: &str = "WINDOW";
pub(crate) const ANYWHERE: &str = "Anywhere";
pub(crate) const IN_SIDEBAR: &str = "In the sidebar";
pub(crate) const IN_TREE: &str = "In the tree";
pub(crate) const IN_NOTE_LIST: &str = "In the note list";
pub(crate) const IN_TAG_POPUP: &str = "In the tag popup";
pub(crate) const IN_SEARCH_FIELD: &str = "In the search field";
pub(crate) const IN_PICKER: &str = "In a picker";
pub(crate) const IN_QUESTION: &str = "In a question";
pub(crate) const IN_POPUP_LIST: &str = "In a popup list";
pub(crate) const IN_NAME_FIELD: &str = "In a name field";
pub(crate) const IN_SETTINGS: &str = "In the settings";
pub(crate) const IN_SHORTCUT_LIST: &str = "In this list";
pub(crate) const IN_ABOUT: &str = "In About Holm";
pub(crate) const IN_FIND: &str = "In find";
pub(crate) const IN_COLOR_WINDOW: &str = "In the color window";
pub(crate) const IN_HEX_FIELD: &str = "In the hex field";

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

    #[test]
    fn the_root_folder_has_a_name() {
        assert_eq!(folder_label(&FolderPath::root()), "Notes folder");
        let folder = FolderPath::new("work/holm").expect("a plain path");
        assert_eq!(folder_label(&folder), "work/holm");
    }
}
