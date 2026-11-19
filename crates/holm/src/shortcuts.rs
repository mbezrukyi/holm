//! The shortcuts of the app. Each key handler of the app matches only on a [`Command`] from
//! this table. [`list`] joins it with the editor table for the shortcut list.
//!
//! Plain cursor keys, Backspace, Delete in a text field, and typing are not shortcuts, so they
//! are not in the table.

use gpui_kit::Keystroke;
use holm_editor::shortcuts::{self as editor, Keys, Shift};

use crate::text;

/// Where a shortcut works.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Place {
    /// The whole notes view.
    Window,
    /// The notes view while the editor has the focus.
    Note,
    /// The sidebar, in the tree or the note list.
    Sidebar,
    /// The tree of the sidebar.
    Tree,
    /// The note list of the sidebar.
    NoteList,
    /// The tag row.
    Tags,
    /// A dialog that asks a question.
    Dialog,
    /// A popup with a list: the problem list, the backlinks and the disk menu.
    ListPopup,
    /// A text field, like the name field or the search field.
    TextField,
    /// The settings dialog.
    Settings,
    /// A place of the editor. Only [`list`] has it.
    Editor(editor::Place),
}

/// The group of a shortcut in the shortcut list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Group {
    Text,
    Note,
    Sidebar,
    Window,
}

impl From<editor::Group> for Group {
    fn from(group: editor::Group) -> Self {
        match group {
            editor::Group::Text => Self::Text,
            editor::Group::Note => Self::Note,
        }
    }
}

/// What a shortcut of the app does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    NewNote,
    NewFolder,
    QuickOpen,
    ToggleSidebar,
    Search,
    DiskState,
    Pin,
    Tags,
    Backlinks,
    Problems,
    Settings,
    ShortcutList,
    Quit,
    AppMenu,
    EditTitle,
    LinkNote,
    LinkFiles,
    LinkFolders,
    Move,
    /// Shows the notes of the tree item.
    ShowNotes,
    FocusEditor,
    /// Clears the search, or goes back to the tree.
    Back,
    Rename,
    Delete,
    Yes,
    No,
    SwitchAsk,
    Pick,
    Close,
    Confirm,
    Cancel,
    NextSetting,
    PreviousSetting,
    /// Switches the switch of the row, or opens the folder picker on the notes folder row.
    SwitchSetting,
    /// Opens the folder picker on the notes folder row.
    ChangeFolder,
}

/// One shortcut of the app.
struct Row {
    place: Place,
    keys: Keys,
    command: Command,
    group: Group,
    label: &'static str,
}

const fn row(place: Place, keys: Keys, command: Command, group: Group, label: &'static str) -> Row {
    Row {
        place,
        keys,
        command,
        group,
        label,
    }
}

use Command as C;
use Group as G;
use Place as P;

const ROWS: &[Row] = &[
    row(
        P::Window,
        Keys::ctrl("n"),
        C::NewNote,
        G::Note,
        text::KEY_NEW_NOTE,
    ),
    row(
        P::Window,
        Keys::ctrl_shift("n"),
        C::NewFolder,
        G::Sidebar,
        text::KEY_NEW_FOLDER,
    ),
    row(
        P::Window,
        Keys::ctrl_shift("o"),
        C::QuickOpen,
        G::Window,
        text::KEY_QUICK_OPEN,
    ),
    row(
        P::Window,
        Keys::ctrl_shift("l"),
        C::ToggleSidebar,
        G::Window,
        text::KEY_TOGGLE_SIDEBAR,
    ),
    row(
        P::Window,
        Keys::ctrl_shift("f"),
        C::Search,
        G::Sidebar,
        text::KEY_SEARCH,
    ),
    row(
        P::Window,
        Keys::ctrl_shift("d"),
        C::DiskState,
        G::Note,
        text::KEY_DISK_STATE,
    ),
    row(
        P::Window,
        Keys::ctrl_shift("p"),
        C::Pin,
        G::Note,
        text::KEY_PIN,
    ),
    row(
        P::Window,
        Keys::ctrl_shift("t"),
        C::Tags,
        G::Note,
        text::KEY_TAGS,
    ),
    row(
        P::Window,
        Keys::ctrl_shift("b"),
        C::Backlinks,
        G::Note,
        text::KEY_BACKLINKS,
    ),
    row(
        P::Window,
        Keys::ctrl_shift("e"),
        C::Problems,
        G::Window,
        text::KEY_PROBLEMS,
    ),
    row(
        P::Window,
        Keys::ctrl_shift("s"),
        C::Settings,
        G::Window,
        text::KEY_SETTINGS,
    ),
    // GPUI drops Shift for a key whose char has no case.
    row(
        P::Window,
        Keys::ctrl("?").shift(Shift::Either),
        C::ShortcutList,
        G::Window,
        text::KEY_SHORTCUT_LIST,
    ),
    row(
        P::Window,
        Keys::ctrl_shift("q"),
        C::Quit,
        G::Window,
        text::KEY_QUIT,
    ),
    row(
        P::Window,
        Keys::plain("f10"),
        C::AppMenu,
        G::Window,
        text::KEY_APP_MENU,
    ),
    row(
        P::Note,
        Keys::plain("f2"),
        C::EditTitle,
        G::Note,
        text::KEY_EDIT_TITLE,
    ),
    row(
        P::Note,
        Keys::ctrl("k"),
        C::LinkNote,
        G::Note,
        text::KEY_LINK_NOTE,
    ),
    row(
        P::Note,
        Keys::ctrl("o"),
        C::LinkFiles,
        G::Note,
        text::KEY_LINK_FILES,
    ),
    row(
        P::Note,
        Keys::ctrl("j"),
        C::LinkFolders,
        G::Note,
        text::KEY_LINK_FOLDERS,
    ),
    row(
        P::Sidebar,
        Keys::ctrl_shift("m"),
        C::Move,
        G::Sidebar,
        text::KEY_MOVE,
    ),
    row(
        P::Tree,
        Keys::plain("enter"),
        C::ShowNotes,
        G::Sidebar,
        text::KEY_SHOW_NOTES,
    ),
    row(
        P::Tree,
        Keys::plain("escape"),
        C::FocusEditor,
        G::Sidebar,
        text::KEY_FOCUS_EDITOR,
    ),
    row(
        P::Tree,
        Keys::plain("f2"),
        C::Rename,
        G::Sidebar,
        text::KEY_RENAME_ITEM,
    ),
    row(
        P::Tree,
        Keys::plain("delete"),
        C::Delete,
        G::Sidebar,
        text::KEY_DELETE_ITEM,
    ),
    row(
        P::NoteList,
        Keys::plain("enter"),
        C::FocusEditor,
        G::Sidebar,
        text::KEY_FOCUS_EDITOR,
    ),
    row(
        P::NoteList,
        Keys::plain("escape"),
        C::Back,
        G::Sidebar,
        text::KEY_BACK,
    ),
    row(
        P::NoteList,
        Keys::plain("f2"),
        C::Rename,
        G::Sidebar,
        text::KEY_RENAME_NOTE,
    ),
    row(
        P::NoteList,
        Keys::plain("delete"),
        C::Delete,
        G::Sidebar,
        text::KEY_DELETE_NOTE,
    ),
    row(
        P::Tags,
        Keys::plain("f2"),
        C::Rename,
        G::Note,
        text::KEY_RENAME_TAG,
    ),
    row(
        P::Dialog,
        Keys::plain("enter"),
        C::Yes,
        G::Window,
        text::KEY_YES,
    ),
    row(
        P::Dialog,
        Keys::plain("escape"),
        C::No,
        G::Window,
        text::KEY_NO,
    ),
    row(
        P::Dialog,
        Keys::plain("space"),
        C::SwitchAsk,
        G::Window,
        text::KEY_SWITCH_ASK,
    ),
    row(
        P::ListPopup,
        Keys::plain("enter"),
        C::Pick,
        G::Window,
        text::KEY_PICK,
    ),
    row(
        P::ListPopup,
        Keys::plain("escape"),
        C::Close,
        G::Window,
        text::KEY_CLOSE,
    ),
    row(
        P::TextField,
        Keys::plain("enter").shift(Shift::Either),
        C::Confirm,
        G::Text,
        text::KEY_CONFIRM,
    ),
    row(
        P::TextField,
        Keys::plain("escape").shift(Shift::Either),
        C::Cancel,
        G::Text,
        text::KEY_CANCEL,
    ),
    row(
        P::Settings,
        Keys::plain("tab"),
        C::NextSetting,
        G::Window,
        text::KEY_NEXT_SETTING,
    ),
    row(
        P::Settings,
        Keys::plain("tab").shift(Shift::Down),
        C::PreviousSetting,
        G::Window,
        text::KEY_PREVIOUS_SETTING,
    ),
    row(
        P::Settings,
        Keys::plain("space"),
        C::SwitchSetting,
        G::Window,
        text::KEY_SWITCH_SETTING,
    ),
    row(
        P::Settings,
        Keys::plain("enter"),
        C::ChangeFolder,
        G::Window,
        text::KEY_CHANGE_FOLDER,
    ),
    row(
        P::Settings,
        Keys::plain("escape"),
        C::Close,
        G::Window,
        text::KEY_CLOSE_SETTINGS,
    ),
];

/// The command of `keystroke` in `place`.
pub(crate) fn command(place: Place, keystroke: &Keystroke) -> Option<Command> {
    ROWS.iter()
        .find(|row| row.place == place && row.keys.matches(keystroke))
        .map(|row| row.command)
}

/// The command of `keystroke` in each text input. Its keys come from the editor table.
pub(crate) fn text_command(keystroke: &Keystroke) -> Option<editor::Command> {
    editor::command(editor::Place::Text, keystroke)
}

/// One shortcut in the shortcut list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), expect(dead_code, reason = "the shortcut list reads it"))]
pub(crate) struct Shortcut {
    pub(crate) place: Place,
    pub(crate) keys: Keys,
    pub(crate) group: Group,
    pub(crate) label: &'static str,
}

/// Every shortcut of the app and the editor.
#[cfg_attr(not(test), expect(dead_code, reason = "the shortcut list reads it"))]
pub(crate) fn list() -> Vec<Shortcut> {
    let app = ROWS.iter().map(|row| Shortcut {
        place: row.place,
        keys: row.keys,
        group: row.group,
        label: row.label,
    });
    let editor = editor::shortcuts().iter().map(|shortcut| Shortcut {
        place: Place::Editor(shortcut.place),
        keys: shortcut.keys,
        group: shortcut.group.into(),
        label: shortcut.label,
    });
    app.chain(editor).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether one key can reach both places.
    fn overlap(one: Place, other: Place) -> bool {
        use editor::Place as E;
        let one_way = |a: Place, b: Place| {
            matches!(
                (a, b),
                (Place::Window, _)
                    | (Place::Sidebar, Place::Tree | Place::NoteList)
                    | (Place::Note, Place::Editor(_))
                    | (Place::Editor(E::Text), Place::Editor(_) | Place::TextField)
            )
        };
        one == other || one_way(one, other) || one_way(other, one)
    }

    #[test]
    fn no_two_shortcuts_share_a_keystroke_in_one_place() {
        let list = list();
        for (at, one) in list.iter().enumerate() {
            for other in &list[at + 1..] {
                assert!(
                    !(overlap(one.place, other.place) && one.keys.overlaps(&other.keys)),
                    "{one:?} and {other:?} share a keystroke",
                );
            }
        }
    }
}
