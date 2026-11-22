//! [`crate::init`] turns this table into the GPUI bindings, so each key reaches the editor as a `Run` action. The app
//! shows the same table in the shortcut list.
//!
//! Plain cursor keys, Backspace, Delete, Enter for a new line, and typing are not shortcuts, so
//! they are not in the table.

use std::rc::Rc;

use gpui_kit::{Action, App, KeyBinding, KeyBindingContextPredicate, Keystroke};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keys {
    /// The GPUI key name, like `b`, `enter` or `f2`.
    pub key: &'static str,
    pub ctrl: bool,
    pub shift: Shift,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shift {
    Up,
    Down,
    /// With a cursor move, Shift extends the selection.
    Either,
}

impl Keys {
    pub const fn plain(key: &'static str) -> Self {
        Self {
            key,
            ctrl: false,
            shift: Shift::Up,
        }
    }

    pub const fn ctrl(key: &'static str) -> Self {
        Self {
            key,
            ctrl: true,
            shift: Shift::Up,
        }
    }

    pub const fn ctrl_shift(key: &'static str) -> Self {
        Self {
            key,
            ctrl: true,
            shift: Shift::Down,
        }
    }

    #[must_use]
    pub const fn shift(self, shift: Shift) -> Self {
        Self { shift, ..self }
    }

    /// The typed char also matches the key, so `+` matches on each layout.
    pub fn matches(&self, keystroke: &Keystroke) -> bool {
        let modifiers = keystroke.modifiers;
        if modifiers.alt || modifiers.platform || modifiers.function || modifiers.control != self.ctrl {
            return false;
        }
        let shift = match self.shift {
            Shift::Up => !modifiers.shift,
            Shift::Down => modifiers.shift,
            Shift::Either => true,
        };
        shift && (keystroke.key == self.key || keystroke.key_char.as_deref() == Some(self.key))
    }

    /// `context` is a key context predicate. `action` makes the action of each binding from whether its keystroke has
    /// Shift, so [`Shift::Either`] makes two bindings. A keystroke that GPUI cannot parse makes none.
    pub fn bindings<A: Action>(self, context: &str, action: impl Fn(bool) -> A, cx: &App) -> Vec<KeyBinding> {
        let predicate = KeyBindingContextPredicate::parse(context).ok().map(Rc::new);
        let shifts: &[bool] = match self.shift {
            Shift::Up => &[false],
            Shift::Down => &[true],
            Shift::Either => &[false, true],
        };
        shifts
            .iter()
            .filter_map(|&shift| {
                let ctrl = if self.ctrl { "ctrl-" } else { "" };
                let shift_down = if shift { "shift-" } else { "" };
                let keystroke = format!("{ctrl}{shift_down}{}", self.key);
                let action = Box::new(action(shift));
                KeyBinding::load(&keystroke, action, predicate.clone(), false, None, cx).ok()
            })
            .collect()
    }

    pub fn overlaps(&self, other: &Keys) -> bool {
        self.key == other.key
            && self.ctrl == other.ctrl
            && (self.shift == other.shift || self.shift == Shift::Either || other.shift == Shift::Either)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// Each text input: the editor, find and the text fields of the app.
    Text,
    /// While find and the color window are closed.
    Editor,
    Find,
    /// While its hex field is closed.
    ColorWindow,
    HexField,
}

impl Place {
    /// [`Place::Text`] is no state of the editor, so it has the context of [`Place::Editor`].
    pub(crate) fn key_context(self) -> &'static str {
        match self {
            Self::Text | Self::Editor => "Editor",
            Self::Find => "Editor Find",
            Self::ColorWindow => "Editor ColorWindow",
            Self::HexField => "Editor HexField",
        }
    }

    /// A GPUI key context predicate.
    fn binding_context(self) -> &'static str {
        match self {
            Self::Text => "Editor && !ColorWindow && !HexField",
            Self::Editor => "Editor && !Find && !ColorWindow && !HexField",
            Self::Find => "Find",
            Self::ColorWindow => "ColorWindow",
            Self::HexField => "HexField",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Action)]
#[action(namespace = holm_editor, no_json)]
pub(crate) struct Run {
    pub(crate) command: Command,
    /// With a cursor move, Shift extends the selection.
    pub(crate) shift: bool,
}

/// For the shortcut list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Text,
    Note,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Command {
    SelectAll,
    Copy,
    Cut,
    Paste,
    WordLeft,
    WordRight,
    NoteStart,
    NoteEnd,
    DeleteWordLeft,
    DeleteWordRight,
    Undo,
    Redo,
    // The format commands turn their style on or off.
    Bold,
    Italic,
    Underline,
    Strikethrough,
    /// With the default color.
    Highlight,
    Code,
    /// Makes the lines a heading of this level, or plain lines with 0.
    Heading(u8),
    /// Opens the color window for the text color.
    TextColor,
    /// Opens the color window for the highlight color.
    HighlightColor,
    /// Checks or unchecks the tasks of the lines.
    Tasks,
    Find,
    /// The link at the cursor.
    OpenLink,
    /// Indents the list item, or types spaces outside a list.
    Indent,
    /// Outdents the list item.
    Outdent,
    NextMatch,
    PreviousMatch,
    CloseFind,
    /// The number counts from 1.
    PaletteColor(u8),
    /// The number counts from 1, the newest first.
    CustomColor(u8),
    RemoveColor,
    HexField,
    /// Applies the color in the hex field.
    TakeHex,
    /// Closes the color window.
    CloseColor,
}

impl Command {
    /// The user can turn these off.
    pub fn is_format_key(self) -> bool {
        matches!(
            self,
            Self::Bold
                | Self::Italic
                | Self::Underline
                | Self::Strikethrough
                | Self::Highlight
                | Self::Code
                | Self::Heading(_)
                | Self::TextColor
                | Self::HighlightColor
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub place: Place,
    pub keys: Keys,
    pub command: Command,
    pub group: Group,
    /// Shown in the shortcut list.
    pub label: &'static str,
}

const fn row(place: Place, keys: Keys, command: Command, group: Group, label: &'static str) -> Shortcut {
    Shortcut {
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

const SHORTCUTS: &[Shortcut] = &[
    row(P::Text, Keys::ctrl("a"), C::SelectAll, G::Text, "Select all"),
    row(P::Text, Keys::ctrl("c"), C::Copy, G::Text, "Copy"),
    row(P::Text, Keys::ctrl("x"), C::Cut, G::Text, "Cut"),
    row(P::Text, Keys::ctrl("v"), C::Paste, G::Text, "Paste"),
    row(P::Editor, Keys::ctrl("z"), C::Undo, G::Text, "Undo"),
    row(P::Editor, Keys::ctrl("y"), C::Redo, G::Text, "Redo"),
    row(P::Editor, Keys::ctrl("b"), C::Bold, G::Text, "Bold"),
    row(P::Editor, Keys::ctrl("i"), C::Italic, G::Text, "Italic"),
    row(P::Editor, Keys::ctrl("u"), C::Underline, G::Text, "Underline"),
    row(P::Editor, Keys::ctrl("d"), C::Strikethrough, G::Text, "Strikethrough"),
    row(P::Editor, Keys::ctrl("h"), C::Highlight, G::Text, "Highlight"),
    row(P::Editor, Keys::ctrl("e"), C::Code, G::Text, "Code"),
    row(P::Editor, Keys::ctrl("t"), C::TextColor, G::Text, "Text color"),
    row(
        P::Editor,
        Keys::ctrl("m"),
        C::HighlightColor,
        G::Text,
        "Highlight color",
    ),
    row(P::Editor, Keys::ctrl("1"), C::Heading(1), G::Text, "Heading 1"),
    row(P::Editor, Keys::ctrl("2"), C::Heading(2), G::Text, "Heading 2"),
    row(P::Editor, Keys::ctrl("3"), C::Heading(3), G::Text, "Heading 3"),
    row(P::Editor, Keys::ctrl("4"), C::Heading(4), G::Text, "Heading 4"),
    row(P::Editor, Keys::ctrl("5"), C::Heading(5), G::Text, "Heading 5"),
    row(P::Editor, Keys::ctrl("6"), C::Heading(6), G::Text, "Heading 6"),
    row(P::Editor, Keys::ctrl("0"), C::Heading(0), G::Text, "Plain line"),
    row(P::Editor, Keys::ctrl("l"), C::Tasks, G::Text, "Check or uncheck a task"),
    row(P::Editor, Keys::plain("tab"), C::Indent, G::Text, "Indent a list item"),
    row(
        P::Editor,
        Keys::plain("tab").shift(Shift::Down),
        C::Outdent,
        G::Text,
        "Outdent a list item",
    ),
    row(
        P::Editor,
        Keys::ctrl("left").shift(Shift::Either),
        C::WordLeft,
        G::Text,
        "Word left",
    ),
    row(
        P::Editor,
        Keys::ctrl("right").shift(Shift::Either),
        C::WordRight,
        G::Text,
        "Word right",
    ),
    row(
        P::Editor,
        Keys::ctrl("home").shift(Shift::Either),
        C::NoteStart,
        G::Text,
        "Note start",
    ),
    row(
        P::Editor,
        Keys::ctrl("end").shift(Shift::Either),
        C::NoteEnd,
        G::Text,
        "Note end",
    ),
    row(
        P::Editor,
        Keys::ctrl("backspace"),
        C::DeleteWordLeft,
        G::Text,
        "Delete the word left",
    ),
    row(
        P::Editor,
        Keys::ctrl("delete"),
        C::DeleteWordRight,
        G::Text,
        "Delete the word right",
    ),
    row(P::Editor, Keys::ctrl("enter"), C::OpenLink, G::Note, "Open the link"),
    row(P::Editor, Keys::ctrl("f"), C::Find, G::Note, "Find in the note"),
    row(P::Find, Keys::plain("enter"), C::NextMatch, G::Note, "Next match"),
    row(
        P::Find,
        Keys::plain("enter").shift(Shift::Down),
        C::PreviousMatch,
        G::Note,
        "Previous match",
    ),
    row(
        P::Find,
        Keys::plain("escape").shift(Shift::Either),
        C::CloseFind,
        G::Note,
        "Close find",
    ),
    row(
        P::ColorWindow,
        Keys::plain("1"),
        C::PaletteColor(1),
        G::Text,
        "Palette color 1",
    ),
    row(
        P::ColorWindow,
        Keys::plain("2"),
        C::PaletteColor(2),
        G::Text,
        "Palette color 2",
    ),
    row(
        P::ColorWindow,
        Keys::plain("3"),
        C::PaletteColor(3),
        G::Text,
        "Palette color 3",
    ),
    row(
        P::ColorWindow,
        Keys::plain("4"),
        C::PaletteColor(4),
        G::Text,
        "Palette color 4",
    ),
    row(
        P::ColorWindow,
        Keys::plain("5"),
        C::PaletteColor(5),
        G::Text,
        "Palette color 5",
    ),
    row(
        P::ColorWindow,
        Keys::plain("6"),
        C::PaletteColor(6),
        G::Text,
        "Palette color 6",
    ),
    row(
        P::ColorWindow,
        Keys::plain("7"),
        C::CustomColor(1),
        G::Text,
        "Custom color 1",
    ),
    row(
        P::ColorWindow,
        Keys::plain("8"),
        C::CustomColor(2),
        G::Text,
        "Custom color 2",
    ),
    row(
        P::ColorWindow,
        Keys::plain("9"),
        C::CustomColor(3),
        G::Text,
        "Custom color 3",
    ),
    row(
        P::ColorWindow,
        Keys::plain("0"),
        C::RemoveColor,
        G::Text,
        "Remove the color",
    ),
    row(
        P::ColorWindow,
        Keys::plain("+"),
        C::HexField,
        G::Text,
        "Type a hex color",
    ),
    row(
        P::ColorWindow,
        Keys::plain("escape").shift(Shift::Either),
        C::CloseColor,
        G::Text,
        "Close the color window",
    ),
    row(
        P::HexField,
        Keys::plain("enter").shift(Shift::Either),
        C::TakeHex,
        G::Text,
        "Apply the hex color",
    ),
    row(P::HexField, Keys::ctrl("v"), C::Paste, G::Text, "Paste a hex color"),
    row(
        P::HexField,
        Keys::plain("escape").shift(Shift::Either),
        C::CloseColor,
        G::Text,
        "Close the color window",
    ),
];

pub(crate) fn bind(cx: &mut App) {
    let bindings: Vec<KeyBinding> = SHORTCUTS
        .iter()
        .flat_map(|shortcut| {
            let command = shortcut.command;
            let context = shortcut.place.binding_context();
            shortcut.keys.bindings(context, |shift| Run { command, shift }, cx)
        })
        .collect();
    cx.bind_keys(bindings);
}

pub fn shortcuts() -> &'static [Shortcut] {
    SHORTCUTS
}

#[cfg(test)]
mod tests {
    use gpui_kit::{TestAppContext, TestDispatcher};

    use super::*;

    #[test]
    fn no_key_with_no_case_carries_shift() {
        // GPUI on Linux drops Shift for a key whose char has no case, so a binding with Shift
        // never matches it.
        for shortcut in SHORTCUTS {
            let mut chars = shortcut.keys.key.chars();
            let no_case = matches!(
                (chars.next(), chars.next()),
                (Some(char), None) if !char.is_lowercase() && !char.is_uppercase()
            );
            assert!(!no_case || shortcut.keys.shift == Shift::Up, "{shortcut:?}");
        }
    }

    #[test]
    fn each_row_binds_its_keys_in_its_place() {
        let cx = TestAppContext::build(TestDispatcher::new(0), None);
        cx.update(|cx| {
            for shortcut in SHORTCUTS {
                let command = shortcut.command;
                let bindings =
                    shortcut
                        .keys
                        .bindings(shortcut.place.binding_context(), |shift| Run { command, shift }, cx);
                let count = if shortcut.keys.shift == Shift::Either { 2 } else { 1 };
                assert_eq!(bindings.len(), count, "{shortcut:?}");
                assert!(
                    bindings.iter().all(|binding| binding.predicate().is_some()),
                    "{shortcut:?}"
                );
            }
        });
    }
}
