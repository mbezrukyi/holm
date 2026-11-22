//! The tag row before the first line of the note: one chip for each tag, and a pin mark.
//!
//! The row only shows the tags. A popup under it edits them, so the row never changes size.
//! The popup holds the field and one list. With an empty field, the list holds the tags of
//! the note, each with a `✓`. Typing keeps the tags whose name holds the typed text, with no
//! regard to capital letters. It adds the other known tags that match, without a `✓`.
//! A typed name that no line has shows as an "Add" line. The popup changes no files itself.
//! It sends each added or removed tag as a [`TagChange`].
//!
//! - The field keeps the focus. `Tab` and `Shift+Tab` move a cursor through the list and
//!   wrap. With no cursor, `Tab` goes to the first line and `Shift+Tab` to the last.
//! - `Enter` adds or removes the tag under the cursor. With no cursor, it adds the typed tag.
//!   A click on a line does the same as `Enter` on it.
//! - `Esc`, or a click outside the popup, closes it. See [`Dropdown`] for the focus.
//! - `F2` on a tag line dispatches [`RenameTag`]. The app starts the rename of the tag in the
//!   sidebar, for all notes. The focus moves to the sidebar, so the popup closes.
//! - A click on the row opens the popup. A click on a chip also puts the cursor on its tag.
//!
//! The row is one line. Chips that do not fit fold into a "+N" chip. A click on it opens the
//! popup with the cursor on the first folded tag. The path of the note and the [`Backlinks`]
//! count sit at the right end of the row: `Work › 2026 › Plans · ← 2 notes`. See
//! [`TitleField`]. The row always shows. A note with no tags has a quiet "+ Tag" chip, and a
//! click on it opens the popup as a click on the row does.
//!
//! A note with no file yet shows [`new_note_row`]: the same row with its tag and its title
//! field, and no popup.

use std::ops::Range;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Action, AnyElement, App, AppContext as _, Context, CursorStyle, ElementId, Entity, EventEmitter, FocusHandle,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Render, Role, StatefulInteractiveElement,
    Styled, Subscription, TestSupportExt as _, Toggled, Window, div, px,
};
use holm_core::model::Tag;
use holm_core::{NotePath, TagName};

use crate::backlinks::{self, Backlinks};
use crate::core_door::CoreDoor;
use crate::fold::Fold;
use crate::list_cursor;
use crate::lowercase::Lowercase;
use crate::popup::{self, Dropdown};
use crate::shortcuts::{Command, Place, Run};
use crate::text;
use crate::title::TitleField;

/// The most known tags in the list while the user types.
const SUGGESTIONS: usize = 5;

/// The widest a chip in the row gets. A longer tag ends with `…`.
pub(crate) const CHIP_WIDTH: Pixels = px(200.);

const POPUP_WIDTH: Pixels = px(300.);

/// The gap on each side of the dot before the backlinks count.
const TAIL_GAP: Pixels = px(6.);

/// The user added or removed one tag of the note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TagChange {
    Add(TagName),
    Remove(TagName),
}

/// The user pressed `F2` on the line of this tag. As an action, it goes up the element
/// tree to the app.
#[derive(Clone, Debug, PartialEq, Eq, Action)]
#[action(namespace = holm, no_json)]
pub(crate) struct RenameTag(pub(crate) TagName);

/// One line of the list in the popup.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Line {
    /// A tag, and whether the note has it.
    Tag { name: TagName, on: bool },
    /// A typed tag that no other line has.
    Add(TagName),
}

pub(crate) struct TagRow {
    core_door: Entity<CoreDoor>,
    tags: Vec<TagName>,
    pinned: bool,
    title: Entity<TitleField<NotePath>>,
    backlinks: Entity<Backlinks>,
    /// Every tag name in the notes folder, for the list.
    known: Vec<TagName>,
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
        core_door: Entity<CoreDoor>,
        title: Entity<TitleField<NotePath>>,
        backlinks: Entity<Backlinks>,
        text: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(text::ADD_TAG));
        let subscriptions = vec![
            cx.observe(&backlinks, |_, _, cx| cx.notify()),
            cx.observe(&title, |_, _, cx| cx.notify()),
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
            core_door,
            tags: Vec::new(),
            pinned: false,
            title,
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
    pub(crate) fn show(&mut self, tags: &[TagName], pinned: bool, cx: &mut Context<Self>) {
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

    /// Opens the popup with an empty field and the cursor on `cursor`. It takes the known
    /// tags for the list from the snapshot.
    fn open_at(&mut self, cursor: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.open = true;
        self.clear(window, cx);
        self.cursor = cursor.filter(|at| *at < self.tags.len());
        self.input.update(cx, |input, cx| input.focus(window, cx));
        let tags = self.core_door.read(cx).snapshot().tags();
        self.known.clear();
        self.known.extend(Tag::walk(&tags).map(|tag| tag.name.clone()));
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
        self.input.update(cx, |input, cx| input.set_value("", window, cx));
    }

    fn typed(&self) -> &str {
        self.query.trim()
    }

    /// The lines of the list, from the typed text.
    fn lines(&self) -> Vec<Line> {
        let typed = self.typed();
        let line = |name: &TagName, on| Line::Tag { name: name.clone(), on };
        if typed.is_empty() {
            return self.tags.iter().map(|name| line(name, true)).collect();
        }
        let typed_lower = Lowercase::new(typed);
        let matches = |name: &&TagName| Lowercase::new(name.as_str()).holds(&typed_lower);
        let mut lines: Vec<Line> = self.tags.iter().filter(matches).map(|name| line(name, true)).collect();
        lines.extend(
            self.known
                .iter()
                .filter(|name| !self.tags.contains(name))
                .filter(matches)
                .take(SUGGESTIONS)
                .map(|name| line(name, false)),
        );
        if let Ok(name) = TagName::typed(typed) {
            let listed = lines
                .iter()
                .any(|line| matches!(line, Line::Tag { name: listed, .. } if *listed == name));
            if !listed {
                lines.push(Line::Add(name));
            }
        }
        lines
    }

    fn move_cursor(&mut self, forward: bool, cx: &mut Context<Self>) {
        if let Some(at) = list_cursor::step(self.cursor, self.lines().len(), forward, |_| true) {
            self.cursor = Some(at);
            cx.notify();
        }
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
            None => TagName::typed(self.typed()).ok().map(Line::Add),
        };
        match line {
            Some(Line::Tag { name, on: true }) => cx.emit(TagChange::Remove(name)),
            Some(Line::Tag { name, on: false } | Line::Add(name)) => {
                if !self.tags.contains(&name) {
                    cx.emit(TagChange::Add(name));
                }
                self.clear(window, cx);
            }
            None => {}
        }
        cx.notify();
    }

    /// A key of [`Place::TagPopup`].
    fn run(&mut self, Run(command): &Run, window: &mut Window, cx: &mut Context<Self>) {
        match command {
            Command::Pick => self.choose(window, cx),
            Command::Close => self.close(window, cx),
            Command::Previous => self.move_cursor(false, cx),
            Command::Next => self.move_cursor(true, cx),
            Command::Rename => self.rename(window, cx),
            _ => cx.propagate(),
        }
    }

    /// `F2`: dispatches [`RenameTag`] for the tag line under the cursor.
    fn rename(&self, window: &mut Window, cx: &mut Context<Self>) {
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
                    row.update(cx, |row, cx| row.open_at(Some(first), window, cx)).ok();
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

    fn row_chip(at: usize, tag: &TagName, cx: &Context<Self>) -> impl IntoElement {
        chip(at, tag, cx).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |row, _, window, cx| {
                row.open_at(Some(at), window, cx);
                // The row and the editor would take the focus after this.
                window.prevent_default();
            }),
        )
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
                    text::tag_label(&name),
                    if on { "✓" } else { "" },
                    Some(if on { Toggled::True } else { Toggled::False }),
                ),
                Line::Add(name) => (text::add_tag_line(&name), "+", None),
            };
            popup::option(ElementId::NamedInteger("tag-option".into(), at as u64), current, cx)
                .aria_label(label.clone())
                .when_some(toggled, StatefulInteractiveElement::aria_toggled)
                .flex()
                .items_center()
                .gap_2()
                .child(div().w(px(12.)).flex_none().text_color(theme.primary).child(mark))
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
            v_flex().border_t_1().border_color(theme.border).children(label).child(
                popup::options()
                    .id("tag-options")
                    .role(Role::ListBox)
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
            .key_context(Place::TagPopup.key_context())
            .on_action(cx.listener(Self::run))
            .child(
                div().p_1().child(
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

/// The frame of the tag row.
fn frame() -> impl StatefulInteractiveElement + ParentElement + Styled + IntoElement {
    h_flex()
        .id("tag-row")
        .role(Role::List)
        .aria_label(text::TAGS_LABEL)
        .test_support()
        .w_full()
        .h(px(40.))
        .pt_3()
        .pb_1()
        .items_center()
}

/// The chip of `tag`, the tag at `at` in the row.
fn chip(at: usize, tag: &TagName, cx: &App) -> impl InteractiveElement + IntoElement + use<> {
    let theme = cx.theme();
    div()
        .id(ElementId::NamedInteger("tag-chip".into(), at as u64))
        .role(Role::ListItem)
        .aria_label(tag.to_string())
        .test_support()
        .max_w(CHIP_WIDTH)
        .truncate()
        .px_2()
        .rounded(theme.radius)
        .text_size(px(12.))
        .bg(theme.secondary)
        .text_color(theme.secondary_foreground)
        .child(text::tag_label(tag))
}

/// The tag row of a note with no file yet: its tag, and `title` at the right end.
pub(crate) fn new_note_row(tag: Option<&TagName>, title: AnyElement, cx: &App) -> impl IntoElement + use<> {
    let chips = tag.map(|tag| chip(0, tag, cx).into_any_element());
    frame()
        .font_family(cx.theme().font_family.clone())
        .child(Fold::new(px(6.), chips, |_| div().into_any_element()).tail(Some(title)))
}

impl EventEmitter<TagChange> for TagRow {}

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
        // The count text starts after its own padding.
        let tail = h_flex()
            .gap(TAIL_GAP)
            .child(self.title.clone())
            .when(has_backlinks, |tail| {
                tail.child(
                    h_flex()
                        .gap(TAIL_GAP - backlinks::BUTTON_PAD)
                        .child(
                            div()
                                .id("path-dot")
                                .test_support()
                                .text_size(px(12.))
                                .text_color(theme.muted_foreground)
                                .child(text::PATH_DOT),
                        )
                        .child(self.backlinks.clone()),
                )
            })
            // A click between the parts must not open the tag popup.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default());
        let lead = (pin.is_some() || add.is_some()).then(|| h_flex().gap(px(6.)).children(pin).children(add));
        let row = frame()
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
                .tail(Some(tail)),
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
