//! The tag row before the first line of the note: one chip for each tag, and a pin mark.
//!
//! The row only shows the tags. A popup under it edits them, so the row never changes size.
//! The popup holds the field and one list. With an empty field, the list holds the tags of
//! the note, each with a `✓`. Typing filters it and adds the other known tags without a `✓`.
//! A typed name that no line has shows as an "Add" line. The popup changes no files itself.
//! It sends the new tags as [`TagsChanged`].
//!
//! - The field keeps the focus. `↑` and `↓` move a cursor through the list.
//! - `Enter` or `Tab` adds or removes the tag under the cursor. With no cursor, it adds the
//!   typed tag. A click on a line does the same as `Enter` on it.
//! - `Esc`, a click outside the popup, or `↓` with an empty list closes the popup. See
//!   [`Dropdown`] for the focus.
//! - `F2` on a tag line dispatches [`RenameTag`]. The app starts the rename of the tag in the
//!   sidebar, for all notes. The focus moves to the sidebar, so the popup closes.
//! - A click on the row opens the popup. A click on a chip also puts the cursor on its tag.
//!
//! The row is one line. Chips that do not fit fold into a "+N" chip. A click on it opens the
//! popup with the cursor on the first folded tag. The [`Backlinks`] count sits at the right
//! end of the row. The row always shows. A note with no tags has a quiet "+ Tag" chip, and a
//! click on it opens the popup as a click on the row does.

use std::ops::Range;

use gpui_kit::component::input::{
    Enter, Escape, IndentInline, Input, InputEvent, InputState, MoveDown, MoveUp,
};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Action, AnyElement, AppContext as _, Context, CursorStyle, ElementId, Entity, EventEmitter,
    FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, ParentElement,
    Pixels, Render, Role, StatefulInteractiveElement, Styled, Subscription, TestSupportExt as _,
    Toggled, Window, div, px,
};
use holm_core::model::Tag;

use crate::backlinks::Backlinks;
use crate::fold::Fold;
use crate::notes_folder::NotesFolder;
use crate::popup::Dropdown;
use crate::text;

/// The most known tags in the list while the user types.
const SUGGESTIONS: usize = 5;

/// The widest a chip in the row gets. A longer tag ends with `…`.
pub(crate) const CHIP_WIDTH: Pixels = px(200.);

const POPUP_WIDTH: Pixels = px(300.);

/// The user added or removed a tag. These are all the tags of the note now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TagsChanged(pub(crate) Vec<String>);

/// The user pressed `F2` on the line of this tag. As an action, it goes up the element
/// tree to the app.
#[derive(Clone, Debug, PartialEq, Eq, Action)]
#[action(namespace = holm, no_json)]
pub(crate) struct RenameTag(pub(crate) String);

/// One line of the list in the popup.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Line {
    /// A tag, and whether the note has it.
    Tag { name: String, on: bool },
    /// A typed tag that no other line has.
    Add(String),
}

pub(crate) struct TagRow {
    notes_folder: Entity<NotesFolder>,
    tags: Vec<String>,
    pinned: bool,
    backlinks: Entity<Backlinks>,
    /// Every tag name in the notes folder, for the list.
    known: Vec<String>,
    /// Whether the popup is open.
    open: bool,
    /// The line under the cursor. With `None`, `Enter` adds the typed tag.
    cursor: Option<usize>,
    /// The text of the field that the list shows. A change of it drops the cursor.
    query: String,
    input: Entity<InputState>,
    dropdown: Dropdown<Self>,
    _subscriptions: Vec<Subscription>,
}

impl TagRow {
    /// `text` gets the focus back when the popup closes with it.
    pub(crate) fn new(
        notes_folder: Entity<NotesFolder>,
        backlinks: Entity<Backlinks>,
        text: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(text::ADD_TAG));
        let subscriptions = vec![
            cx.observe(&backlinks, |_, _, cx| cx.notify()),
            cx.subscribe(&input, |row, input, event, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    if value != row.query {
                        row.query = value;
                        row.cursor = None;
                        cx.notify();
                    }
                }
            }),
        ];
        Self {
            notes_folder,
            tags: Vec::new(),
            pinned: false,
            backlinks,
            known: Vec::new(),
            open: false,
            cursor: None,
            query: String::new(),
            input,
            dropdown: Dropdown::new(text, Self::close, window, cx),
            _subscriptions: subscriptions,
        }
    }

    /// Shows the tags and the pin of the open note.
    pub(crate) fn show(&mut self, tags: &[String], pinned: bool, cx: &mut Context<Self>) {
        if self.tags != tags || self.pinned != pinned {
            self.tags = tags.to_vec();
            self.pinned = pinned;
            self.clamp_cursor();
            cx.notify();
        }
    }

    /// Opens the popup with an empty field and no cursor.
    pub(crate) fn enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_at(None, window, cx);
    }

    /// Pins or unpins the note in the row, and gives the new pin.
    pub(crate) fn toggle_pin(&mut self, cx: &mut Context<Self>) -> bool {
        self.pinned = !self.pinned;
        cx.notify();
        self.pinned
    }

    /// Opens the popup with an empty field and the cursor on `cursor`. It takes the known
    /// tags for the list from the snapshot.
    fn open_at(&mut self, cursor: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.open = true;
        self.clear(window, cx);
        self.cursor = cursor.filter(|at| *at < self.tags.len());
        self.input.update(cx, |input, cx| input.focus(window, cx));
        let tags = self.notes_folder.read(cx).snapshot().tags();
        self.known.clear();
        flatten(&tags, &mut self.known);
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.clear(window, cx);
        cx.notify();
    }

    /// Empties the field and drops the cursor.
    fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query.clear();
        self.cursor = None;
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    fn typed(&self) -> &str {
        self.query.trim()
    }

    /// The lines of the list, from the typed text.
    fn lines(&self) -> Vec<Line> {
        let typed = self.typed();
        let line = |name: &String, on| Line::Tag {
            name: name.clone(),
            on,
        };
        if typed.is_empty() {
            return self.tags.iter().map(|name| line(name, true)).collect();
        }
        let lower = typed.to_lowercase();
        let matches = |name: &&String| name.to_lowercase().contains(&lower);
        let mut lines: Vec<Line> = self
            .tags
            .iter()
            .filter(matches)
            .map(|name| line(name, true))
            .collect();
        lines.extend(
            self.known
                .iter()
                .filter(|name| !self.tags.contains(name))
                .filter(matches)
                .take(SUGGESTIONS)
                .map(|name| line(name, false)),
        );
        if let Some(name) = holm_markdown::normalize_tag(typed) {
            let listed = lines
                .iter()
                .any(|line| matches!(line, Line::Tag { name: listed, .. } if *listed == name));
            if !listed {
                lines.push(Line::Add(name));
            }
        }
        lines
    }

    /// Moves the cursor. It returns whether the list has lines.
    fn move_cursor(&mut self, down: bool, cx: &mut Context<Self>) -> bool {
        let count = self.lines().len();
        if count == 0 {
            return false;
        }
        self.cursor = match (self.cursor, down) {
            (None, true) => Some(0),
            (Some(at), true) => Some((at + 1).min(count - 1)),
            (Some(at), false) => at.checked_sub(1),
            (None, false) => None,
        };
        cx.notify();
        true
    }

    /// Keeps the cursor on a line after the list got shorter.
    fn clamp_cursor(&mut self) {
        let last = self.lines().len().checked_sub(1);
        self.cursor = self.cursor.zip(last).map(|(at, last)| at.min(last));
    }

    /// Adds or removes the tag under the cursor. With no cursor, it adds the typed tag.
    fn choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let line = match self.cursor {
            Some(at) => self.lines().get(at).cloned(),
            None => holm_markdown::normalize_tag(self.typed()).map(Line::Add),
        };
        match line {
            Some(Line::Tag { name, on: true }) => {
                self.tags.retain(|tag| *tag != name);
                cx.emit(TagsChanged(self.tags.clone()));
                self.clamp_cursor();
            }
            Some(Line::Tag { name, on: false } | Line::Add(name)) => {
                if !self.tags.contains(&name) {
                    self.tags.push(name);
                    cx.emit(TagsChanged(self.tags.clone()));
                }
                self.clear(window, cx);
            }
            None => {}
        }
        cx.notify();
    }

    /// `F2`: dispatches [`RenameTag`] for the tag line under the cursor.
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.key != "f2" || keystroke.modifiers.modified() {
            return;
        }
        cx.stop_propagation();
        let line = self.cursor.and_then(|at| self.lines().get(at).cloned());
        if let Some(Line::Tag { name, .. }) = line {
            window.dispatch_action(Box::new(RenameTag(name)), cx);
        }
    }

    /// Builds the quiet "+N" chip for a range of folded tags.
    fn more_chip(cx: &Context<Self>) -> impl Fn(Range<usize>) -> AnyElement + 'static {
        let theme = cx.theme();
        let (radius, text, hover) = (theme.radius, theme.muted_foreground, theme.secondary);
        let row = cx.entity().downgrade();
        move |folded| {
            let first = folded.start;
            let row = row.clone();
            div()
                .id("tag-more")
                .role(Role::Button)
                .px_2()
                .rounded(radius)
                .text_size(px(12.))
                .text_color(text)
                .cursor(CursorStyle::PointingHand)
                .hover(move |chip| chip.bg(hover))
                .child(format!("+{}", folded.len()))
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    row.update(cx, |row, cx| row.open_at(Some(first), window, cx))
                        .ok();
                    // The row and the editor would take the focus after this.
                    window.prevent_default();
                })
                .test_support()
                .into_any_element()
        }
    }

    /// The quiet chip of a note with no tags. The row takes its click.
    fn add_chip(cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .id("tag-add")
            .role(Role::Button)
            .aria_label(text::ADD_TAG)
            .px(px(10.))
            .border_1()
            .border_dashed()
            .border_color(theme.border)
            .rounded_full()
            .text_size(px(12.))
            .line_height(px(16.))
            .text_color(theme.muted_foreground)
            .cursor(CursorStyle::PointingHand)
            .hover(|chip| chip.bg(theme.secondary))
            .child(text::ADD_TAG_CHIP)
            .test_support()
    }

    fn row_chip(at: usize, tag: &str, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .id(ElementId::NamedInteger("tag-chip".into(), at as u64))
            .role(Role::ListItem)
            .aria_label(tag.to_owned())
            .max_w(CHIP_WIDTH)
            .truncate()
            .px_2()
            .rounded(theme.radius)
            .text_size(px(12.))
            .bg(theme.secondary)
            .text_color(theme.secondary_foreground)
            .child(format!("#{tag}"))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |row, _, window, cx| {
                    row.open_at(Some(at), window, cx);
                    // The row and the editor would take the focus after this.
                    window.prevent_default();
                }),
            )
            .test_support()
    }

    /// The list under the field, while it has lines.
    fn list(&self, cx: &Context<Self>) -> Option<impl IntoElement + use<>> {
        let theme = cx.theme();
        let lines = self.lines();
        if lines.is_empty() {
            return None;
        }
        let label = self.typed().is_empty().then(|| {
            div()
                .px_3()
                .pt_1p5()
                .text_size(px(11.))
                .text_color(theme.muted_foreground)
                .child(text::ON_THIS_NOTE)
        });
        let rows = lines.into_iter().enumerate().map(|(at, line)| {
            let current = self.cursor == Some(at);
            let (label, mark, toggled) = match line {
                Line::Tag { name, on } => (
                    format!("#{name}"),
                    if on { "✓" } else { "" },
                    Some(if on { Toggled::True } else { Toggled::False }),
                ),
                Line::Add(name) => (text::add_tag_line(&name), "+", None),
            };
            div()
                .id(ElementId::NamedInteger("tag-option".into(), at as u64))
                .role(Role::ListBoxOption)
                .aria_label(label.clone())
                .aria_selected(current)
                .when_some(toggled, StatefulInteractiveElement::aria_toggled)
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .cursor(CursorStyle::PointingHand)
                .when(current, |line| line.bg(theme.selection))
                .when(!current, |line| line.hover(|line| line.bg(theme.secondary)))
                .child(
                    div()
                        .w(px(12.))
                        .flex_none()
                        .text_color(theme.primary)
                        .child(mark),
                )
                .child(div().flex_1().min_w_0().truncate().child(label))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |row, _, window, cx| {
                        row.cursor = Some(at);
                        row.choose(window, cx);
                    }),
                )
                .test_support()
        });
        Some(
            v_flex()
                .border_t_1()
                .border_color(theme.border)
                .mt_1()
                .children(label)
                .child(
                    v_flex()
                        .id("tag-options")
                        .role(Role::ListBox)
                        .py_1()
                        .children(rows)
                        .test_support(),
                ),
        )
    }

    /// The popup under the row: the field and the list.
    fn popup(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        self.dropdown
            .panel(POPUP_WIDTH, window, cx)
            .left_0()
            .key_context("TagRow")
            .on_key_down(cx.listener(Self::key_down))
            .capture_action(cx.listener(|row, _: &Enter, window, cx| {
                row.choose(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|row, _: &IndentInline, window, cx| {
                row.choose(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|row, _: &Escape, window, cx| {
                row.close(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|row, _: &MoveDown, window, cx| {
                if !row.move_cursor(true, cx) {
                    row.close(window, cx);
                }
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|row, _: &MoveUp, _, cx| {
                row.move_cursor(false, cx);
                cx.stop_propagation();
            }))
            .child(
                div().px_1().child(
                    Input::new(&self.input)
                        .id("tag-input")
                        .aria_label(text::ADD_TAG)
                        .appearance(false),
                ),
            )
            .children(self.list(cx))
            .id("tag-popup")
            .role(Role::Dialog)
            .aria_label(text::TAGS_LABEL)
            .test_support()
    }
}

/// Every tag name in the tree, parents first.
fn flatten(tags: &[Tag], names: &mut Vec<String>) {
    for tag in tags {
        names.push(tag.name.clone());
        flatten(&tag.children, names);
    }
}

impl EventEmitter<TagsChanged> for TagRow {}

impl Render for TagRow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let has_backlinks = !self.backlinks.read(cx).is_empty();
        let pin = self.pinned.then(|| {
            div()
                .id("pin-mark")
                .role(Role::Image)
                .aria_label(text::PINNED)
                .test_support()
                .text_size(px(10.))
                .text_color(theme.primary)
                .child(text::PIN_MARK)
        });
        let add = self.tags.is_empty().then(|| Self::add_chip(cx));
        let lead = (pin.is_some() || add.is_some())
            .then(|| h_flex().gap(px(6.)).children(pin).children(add));
        let row = h_flex()
            .id("tag-row")
            .role(Role::List)
            .aria_label(text::TAGS_LABEL)
            .test_support()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|row, _, window, cx| {
                    // A chip took the click already.
                    if !window.default_prevented() {
                        row.enter(window, cx);
                        window.prevent_default();
                    }
                }),
            )
            .w_full()
            .h(px(40.))
            .pt_3()
            .pb_1()
            .items_center()
            .child(
                Fold::new(
                    px(6.),
                    self.tags
                        .iter()
                        .enumerate()
                        .map(|(at, tag)| Self::row_chip(at, tag, cx).into_any_element()),
                    Self::more_chip(cx),
                )
                .lead(lead)
                .tail(has_backlinks.then(|| self.backlinks.clone())),
            );
        self.dropdown
            .root()
            .w_full()
            .cursor(CursorStyle::Arrow)
            // The row lies in the editor, which sets the note font.
            .font_family(theme.font_family.clone())
            .child(row)
            .when(self.open, |head| head.child(self.popup(window, cx)))
    }
}
