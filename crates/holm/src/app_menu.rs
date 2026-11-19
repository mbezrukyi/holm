//! The app menu: Settings, Shortcuts, Pin note and Quit, each with its keys.
//!
//! - It opens under the ☰ button of the sidebar head, or at the top-left corner of the window
//!   while the sidebar is hidden.
//! - It opens with the cursor on the first item. `↑` and `↓` move the cursor past an item
//!   that is off. `Enter` or a click picks the item under it.
//! - `Esc`, a pick, or a second open closes it and gives the focus back to the part that had
//!   it before. A click outside the menu and the button closes it too. See [`Dropdown`] for
//!   the focus.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, Bounds, Context, ElementId, EventEmitter, FocusHandle, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, ParentElement, Pixels, Render, Role,
    StatefulInteractiveElement as _, Styled, TestSupportExt as _, Window, anchored, deferred, div,
    point, px,
};

use crate::popup::{self, Dropdown};
use crate::shortcuts::{self, Command, Place, Run};
use crate::{shortcut_list, text};

/// The items of the menu, as the commands they run.
const ITEMS: [Command; 4] = [
    Command::Settings,
    Command::ShortcutList,
    Command::Pin,
    Command::Quit,
];

const WIDTH: Pixels = px(240.);
/// The space between the menu and its button, or the window corner.
const GAP: Pixels = px(4.);

/// Where the menu opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Spot {
    /// Under the button of the sidebar head.
    Button,
    /// At the top-left corner of the window.
    Corner,
}

/// The user picked the item of this command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Picked(pub(crate) Command);

struct Open {
    spot: Spot,
    /// Whether a note is open, so Pin note works.
    note: bool,
    /// The part that had the focus before the menu.
    back: Option<FocusHandle>,
}

pub(crate) struct AppMenu {
    /// `None` while the menu is closed.
    open: Option<Open>,
    cursor: usize,
    /// The window bounds of the button in the last frame.
    button: Rc<Cell<Bounds<Pixels>>>,
    dropdown: Dropdown<Self>,
}

impl AppMenu {
    /// `text` gets the focus when the menu closes with it and no part had it before. The
    /// sidebar keeps the bounds of the button in `button`.
    pub(crate) fn new(
        text: FocusHandle,
        button: Rc<Cell<Bounds<Pixels>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            open: None,
            cursor: 0,
            button: button.clone(),
            dropdown: Dropdown::new(text, |menu, _, cx| menu.close(cx), window, cx)
                .with_opener(button),
        }
    }

    /// Opens the menu at `spot`, or closes it when it is open. Pin note is off without an
    /// open `note`.
    pub(crate) fn toggle(
        &mut self,
        spot: Spot,
        note: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open.is_some() {
            self.leave(window, cx);
            return;
        }
        if spot == Spot::Corner {
            // The hidden button keeps its last bounds, and a click there must close the menu.
            self.button.set(Bounds::default());
        }
        self.open = Some(Open {
            spot,
            note,
            back: window.focused(cx),
        });
        self.cursor = 0;
        window.focus(self.dropdown.focus_handle(), cx);
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if self.open.take().is_some() {
            cx.notify();
        }
    }

    /// Closes the menu and gives the focus back.
    fn leave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(open) = self.open.take() {
            if let Some(back) = open.back {
                window.focus(&back, cx);
            }
            cx.notify();
        }
    }

    fn works(&self, at: usize) -> bool {
        match ITEMS.get(at) {
            Some(Command::Pin) => self.open.as_ref().is_some_and(|open| open.note),
            Some(_) => true,
            None => false,
        }
    }

    fn pick(&mut self, at: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.works(at) {
            return;
        }
        self.leave(window, cx);
        cx.emit(Picked(ITEMS[at]));
    }

    /// Moves the cursor to the next item that works, up or down. It stays at the last one.
    fn step(&mut self, down: bool) {
        let mut at = self.cursor;
        loop {
            let next = if down {
                at.checked_add(1)
            } else {
                at.checked_sub(1)
            };
            match next.filter(|next| *next < ITEMS.len()) {
                Some(next) if self.works(next) => {
                    self.cursor = next;
                    return;
                }
                Some(next) => at = next,
                None => return,
            }
        }
    }

    fn run(&mut self, Run(command): &Run, window: &mut Window, cx: &mut Context<Self>) {
        match command {
            Command::Pick => self.pick(self.cursor, window, cx),
            Command::Close => self.leave(window, cx),
            _ => return cx.propagate(),
        }
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.modified() {
            return;
        }
        match keystroke.key.as_str() {
            "up" => self.step(false),
            "down" => self.step(true),
            _ => return,
        }
        cx.notify();
        cx.stop_propagation();
    }
}

impl EventEmitter<Picked> for AppMenu {}

impl Render for AppMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(open) = &self.open else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let (muted, border) = (theme.muted_foreground, theme.border);
        let rows = ITEMS.into_iter().enumerate().map(|(at, command)| {
            let label = match command {
                Command::Settings => text::MENU_SETTINGS,
                Command::ShortcutList => text::MENU_SHORTCUTS,
                Command::Pin => text::MENU_PIN,
                _ => text::MENU_QUIT,
            };
            let works = self.works(at);
            let id = ElementId::NamedInteger("menu-item".into(), at as u64);
            let row = if works {
                popup::option(id, self.cursor == at, cx).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |menu, _, window, cx| menu.pick(at, window, cx)),
                )
            } else {
                // The shape of an option, with no hover and no click.
                div()
                    .id(id)
                    .role(Role::ListBoxOption)
                    .px_2()
                    .py_1()
                    .text_color(muted)
            }
            .aria_label(label)
            .test_support()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(label)
            .children(shortcuts::window_keys(command).map(shortcut_list::key_chip));
            // A line sets Quit apart from the other items.
            div()
                .when(command == Command::Quit, |item| {
                    item.border_t_1().border_color(border).mt_1().pt_1()
                })
                .child(row)
        });
        let place = match open.spot {
            Spot::Button => {
                let button = self.dropdown.opener();
                point(button.left(), button.bottom() + GAP)
            }
            Spot::Corner => point(GAP, GAP),
        };
        let panel = self
            .dropdown
            .frame(WIDTH, window, cx)
            .key_context(Place::ListPopup.key_context())
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(Self::run))
            .id("app-menu")
            .role(Role::ListBox)
            .test_support()
            .child(popup::options().children(rows));
        deferred(
            anchored()
                .anchor(Anchor::TopLeft)
                .position(place)
                .snap_to_window_with_margin(GAP)
                .child(panel),
        )
        .into_any_element()
    }
}
