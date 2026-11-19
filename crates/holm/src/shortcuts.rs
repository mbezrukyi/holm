//! The shortcuts of the app. [`bind`] turns this table into the GPUI bindings, so each key
//! reaches its view as a [`Run`] action. [`list`] joins it with the editor table for the
//! shortcut list.
//!
//! Plain cursor keys, Backspace, Delete in a text field, and typing are not shortcuts, so they
//! are not in the table.

use gpui_kit::{Action, App, KeyBinding, Keystroke, NoAction};
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
    /// A popup with a list: the problem list, the backlinks, the state menu and the app menu.
    ListPopup,
    /// A text field, like the name field or the search field.
    TextField,
    /// The settings dialog.
    Settings,
    /// The shortcut list.
    ShortcutList,
    /// A place of the editor. Only [`list`] has it.
    Editor(editor::Place),
}

impl Place {
    /// The key context name of this place. The view of the place sets it while it is in the
    /// place. A [`Place::Note`] key works while the editor has the focus, so it has the
    /// context of the editor element, like each place of the editor.
    pub(crate) const fn key_context(self) -> &'static str {
        match self {
            Self::Window => "Notes",
            Self::Sidebar => "Sidebar",
            Self::Tree => "Tree",
            Self::NoteList => "NoteList",
            Self::Tags => "Tags",
            Self::Dialog => "Dialog",
            Self::ListPopup => "ListPopup",
            Self::TextField => "TextField",
            Self::Settings => "Settings",
            Self::ShortcutList => "ShortcutList",
            Self::Note | Self::Editor(_) => "Editor",
        }
    }
}

/// A shortcut of the table, from its GPUI binding.
#[derive(Clone, Debug, PartialEq, Eq, Action)]
#[action(namespace = holm, no_json)]
pub(crate) struct Run(pub(crate) Command);

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
    /// A key of the editor table that works in each text input.
    Text(editor::Command),
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
        text::KEY_CLOSE,
    ),
    row(
        P::ShortcutList,
        Keys::plain("escape"),
        C::Close,
        G::Window,
        text::KEY_CLOSE,
    ),
];

/// The command of `keystroke` in `place`.
pub(crate) fn command(place: Place, keystroke: &Keystroke) -> Option<Command> {
    ROWS.iter()
        .find(|row| row.place == place && row.keys.matches(keystroke))
        .map(|row| row.command)
}

/// The keys of `command` in `place`.
pub(crate) fn keys(place: Place, command: Command) -> Option<Keys> {
    ROWS.iter()
        .find(|row| row.place == place && row.command == command)
        .map(|row| row.keys)
}

/// The context of a text input of `gpui-component`.
const INPUT: &str = "Input";

/// The context of the window root of `gpui-component`. It binds Tab to a focus move.
const ROOT: &str = "Root";

/// Binds the keys of each row to [`Run`], and the editor [`editor::Place::Text`] rows in the
/// text fields. Then it gives the text inputs their keys back with `NoAction`:
///
/// - each text input keeps each key that types a char and that some place binds;
/// - a text field keeps each plain key of the other places, except the [`Place::Window`] keys.
///
/// Tab and Shift+Tab do nothing where no place names them.
pub(crate) fn bind(cx: &mut App) {
    let text_field = Place::TextField.key_context();
    let mut bindings: Vec<KeyBinding> = ROWS
        .iter()
        .flat_map(|row| {
            let command = row.command;
            row.keys
                .bindings(row.place.key_context(), |_| Run(command), cx)
        })
        .collect();
    for shortcut in text_keys() {
        let command = Command::Text(shortcut.command);
        bindings.extend(shortcut.keys.bindings(text_field, |_| Run(command), cx));
    }
    for keys in typed_keys() {
        bindings.extend(keys.bindings(INPUT, |_| NoAction, cx));
    }
    for keys in kept_by_text_fields() {
        bindings.extend(keys.bindings(text_field, |_| NoAction, cx));
    }
    for keys in [Keys::plain("tab"), Keys::plain("tab").shift(Shift::Down)] {
        bindings.extend(keys.bindings(ROOT, |_| NoAction, cx));
    }
    cx.bind_keys(bindings);
}

/// The editor rows that work in each text input.
fn text_keys() -> impl Iterator<Item = &'static editor::Shortcut> {
    editor::shortcuts()
        .iter()
        .filter(|shortcut| shortcut.place == editor::Place::Text)
}

/// The keys of every place that type a char, like Space.
fn typed_keys() -> Vec<Keys> {
    list()
        .into_iter()
        .map(|shortcut| shortcut.keys)
        .filter(|keys| !keys.ctrl && (keys.key == "space" || keys.key.chars().count() == 1))
        .collect()
}

/// The plain keys of the places other than [`Place::Window`] that no text field row names.
fn kept_by_text_fields() -> Vec<Keys> {
    let named: Vec<Keys> = ROWS
        .iter()
        .filter(|row| row.place == Place::TextField)
        .map(|row| row.keys)
        .chain(text_keys().map(|shortcut| shortcut.keys))
        .collect();
    ROWS.iter()
        .filter(|row| !matches!(row.place, Place::TextField | Place::Window) && !row.keys.ctrl)
        .map(|row| row.keys)
        .filter(|keys| !named.iter().any(|name| name.overlaps(keys)))
        .collect()
}

/// The keystroke the shortcut list shows for `keys`. With [`Shift::Either`] it has no Shift,
/// so Ctrl+? shows as "Ctrl+?".
pub(crate) fn keystroke(keys: Keys) -> Keystroke {
    let mut keystroke = Keystroke {
        key: keys.key.to_owned(),
        ..Keystroke::default()
    };
    keystroke.modifiers.control = keys.ctrl;
    keystroke.modifiers.shift = keys.shift == Shift::Down;
    keystroke
}

/// One shortcut in the shortcut list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Shortcut {
    pub(crate) place: Place,
    pub(crate) keys: Keys,
    pub(crate) group: Group,
    pub(crate) label: &'static str,
}

/// Every shortcut of the app and the editor.
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
                    | (
                        Place::Editor(E::Text),
                        Place::Editor(E::Editor | E::Find) | Place::TextField
                    )
            )
        };
        one == other || one_way(one, other) || one_way(other, one)
    }

    #[test]
    fn each_row_of_the_app_binds_its_keys_in_its_place() {
        let cx = gpui_kit::TestAppContext::build(gpui_kit::TestDispatcher::new(0), None);
        cx.update(|cx| {
            for row in ROWS {
                let context = row.place.key_context();
                let bindings = row.keys.bindings(context, |_| Run(row.command), cx);
                let count = if row.keys.shift == Shift::Either {
                    2
                } else {
                    1
                };
                assert_eq!(bindings.len(), count, "{:?} {:?}", row.place, row.keys);
            }
        });
    }

    #[test]
    fn text_inputs_keep_space_and_text_fields_keep_the_plain_keys_of_other_places() {
        let typed = typed_keys();
        assert!(typed.iter().all(|keys| !keys.ctrl), "{typed:?}");
        assert!(typed.iter().any(|keys| keys.key == "space"), "{typed:?}");
        assert!(
            typed.iter().any(|keys| keys.key == "1"),
            "a color of the color window"
        );
        assert!(!typed.iter().any(|keys| keys.key == "enter"));

        let kept = kept_by_text_fields();
        let has = |keys: Keys| kept.iter().any(|kept| kept.overlaps(&keys));
        assert!(has(Keys::plain("tab")), "the tab of the settings");
        assert!(has(Keys::plain("space")));
        assert!(has(Keys::plain("delete")), "the delete of the tree");
        assert!(has(Keys::plain("f2")), "the rename of the tree");
        assert!(!has(Keys::plain("enter")), "the text field names it");
        assert!(!has(Keys::plain("escape")), "the text field names it");
        assert!(!has(Keys::plain("f10")), "a window key passes");
        assert!(kept.iter().all(|keys| !keys.ctrl));
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
