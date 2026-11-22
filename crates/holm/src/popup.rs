//! The look of a small window over the notes: a dim layer over the whole window, a panel in
//! its middle, and [`PickList`], a list with a search field in the panel. Also [`Dropdown`], a
//! popup under a part of a row.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::{Scrollbar, ScrollbarMode};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Bounds, ClickEvent, Context, CursorStyle, Div, ElementId, Entity, EventEmitter, FocusHandle,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, ParentElement as _, Pixels, Render, Role,
    ScrollStrategy, SharedString, Stateful, StatefulInteractiveElement as _, Styled, Subscription, TestSupportExt as _,
    UniformListScrollHandle, Window, div, phi, px, transparent_black, uniform_list,
};
use holm_core::NotePath;
use holm_core::model::NoteInfo;

use crate::icons::Icon;
use crate::list_cursor;
use crate::shortcuts::{Command, Place, Run};
use crate::text;

/// A layer over the whole window that centers its child. The parts under it get no mouse
/// input.
pub(crate) fn layer(cx: &App) -> Div {
    div()
        .occlude()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(cx.theme().overlay)
}

/// The panel in the layer, `width` wide.
pub(crate) fn panel(width: Pixels, cx: &App) -> Div {
    let theme = cx.theme();
    v_flex()
        .w(width)
        .bg(theme.popover)
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius)
}

/// The height of a [`PickList`] row.
const ROW_HEIGHT: Pixels = px(30.);
/// The inset of the rows from the edges of the list.
const LIST_INSET: Pixels = px(4.);
/// The most height of the rows of a [`PickList`]. A longer list scrolls.
const LIST_HEIGHT: Pixels = px(300.);

/// A row of a [`PickList`]: a label, and some detail on the right in a muted color. A long
/// label or detail ends in "…".
#[derive(Clone)]
pub(crate) struct PickRow {
    label: SharedString,
    detail: Option<SharedString>,
    icon: Option<Icon>,
}

impl PickRow {
    pub(crate) fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            detail: None,
            icon: None,
        }
    }

    /// The row of a note: its title, and its folder as the detail. A note in the root folder
    /// has no detail.
    pub(crate) fn note(note: &NoteInfo) -> Self {
        Self {
            detail: folder(&note.path),
            ..Self::new(note.title.clone())
        }
    }

    pub(crate) fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    fn view(&self, at: usize, current: bool, cx: &App) -> Stateful<Div> {
        let theme = cx.theme();
        h_flex()
            .id(ElementId::NamedInteger("row".into(), at as u64))
            .role(Role::ListBoxOption)
            .aria_label(self.label.clone())
            .aria_selected(current)
            .h(ROW_HEIGHT)
            .w_full()
            .px_2()
            .gap_2()
            .text_sm()
            .rounded(theme.radius)
            .cursor_default()
            .when(current, |row| row.bg(theme.accent).text_color(theme.accent_foreground))
            .children(self.icon.map(|icon| Icon::view(icon, theme.muted_foreground)))
            .child(div().min_w(px(48.)).truncate().child(self.label.clone()))
            .children(self.detail.clone().map(|detail| {
                div()
                    .id("folder")
                    .role(Role::Label)
                    .aria_label(detail.clone())
                    .test_support()
                    .ml_auto()
                    .min_w_0()
                    // The detail gets short before the label does.
                    .flex_shrink(4.)
                    .truncate()
                    .text_color(theme.muted_foreground)
                    .child(detail)
            }))
    }
}

fn folder(note: &NotePath) -> Option<SharedString> {
    let folder = note.folder();
    (!folder.is_root()).then(|| text::folder_label(folder).into())
}

/// What a [`PickList`] tells its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PickEvent {
    /// The text of the field changed. The owner gives new rows with [`PickList::set_rows`].
    Query,
    /// The user picked the row at this place, with `Enter` or a click.
    Pick(usize),
    /// The user pressed `Esc`.
    Close,
}

/// A search field over a list of rows, for the panel of a small window.
///
/// Its keys are the rows of [`Place::Picker`]. `Tab` and `Shift+Tab` move the cursor and wrap.
/// A row under the mouse gets the cursor too. A list that is too long for the panel scrolls,
/// and its scrollbar takes a lane on the right of the rows.
pub(crate) struct PickList {
    field: Entity<InputState>,
    rows: Rc<[PickRow]>,
    cursor: usize,
    empty: &'static str,
    scroll: UniformListScrollHandle,
    _field: Subscription,
}

impl PickList {
    /// An empty list. The field shows `placeholder` and gets the focus. A list with no rows
    /// shows `empty`.
    pub(crate) fn new(
        placeholder: impl Into<SharedString>,
        empty: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let field = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        field.update(cx, |field, cx| field.focus(window, cx));
        let subscription = cx.subscribe(&field, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.emit(PickEvent::Query);
            }
        });
        Self {
            field,
            rows: Rc::from([]),
            cursor: 0,
            empty,
            scroll: UniformListScrollHandle::new(),
            _field: subscription,
        }
    }

    /// The text of the field.
    pub(crate) fn query(&self, cx: &App) -> SharedString {
        self.field.read(cx).value()
    }

    /// Puts `query` in the field. It sends no [`PickEvent::Query`].
    pub(crate) fn set_query(&self, query: String, window: &mut Window, cx: &mut Context<Self>) {
        self.field.update(cx, |field, cx| field.set_value(query, window, cx));
    }

    /// Shows `rows`, with the cursor on the first one.
    pub(crate) fn set_rows(&mut self, rows: Vec<PickRow>, cx: &mut Context<Self>) {
        self.rows = rows.into();
        self.cursor = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// The place of the row under the cursor, or `None` when the list has no rows.
    pub(crate) fn cursor(&self) -> Option<usize> {
        (self.cursor < self.rows.len()).then_some(self.cursor)
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        if let Some(at) = list_cursor::step(self.cursor(), self.rows.len(), forward, |_| true) {
            self.cursor = at;
            self.scroll.scroll_to_item(at, ScrollStrategy::Nearest);
            cx.notify();
        }
    }

    fn run(&mut self, Run(command): &Run, _: &mut Window, cx: &mut Context<Self>) {
        match command {
            Command::Previous => self.step(false, cx),
            Command::Next => self.step(true, cx),
            Command::Pick => {
                if let Some(at) = self.cursor() {
                    cx.emit(PickEvent::Pick(at));
                }
            }
            Command::Close => cx.emit(PickEvent::Close),
            _ => cx.propagate(),
        }
    }

    fn list(&self, cx: &Context<Self>) -> Stateful<Div> {
        let count = self.rows.len();
        let full = ROW_HEIGHT * count + LIST_INSET * 2.;
        let scrolls = full > LIST_HEIGHT;
        let rows = uniform_list(
            "rows",
            count,
            cx.processor(|list, range: std::ops::Range<usize>, _, cx| {
                range
                    .filter_map(|at| list.rows.get(at).map(|row| (at, row)))
                    .map(|(at, row)| {
                        row.view(at, at == list.cursor, cx)
                            .on_hover(cx.listener(move |list, hovered: &bool, _, cx| {
                                if *hovered && list.cursor != at {
                                    list.cursor = at;
                                    cx.notify();
                                }
                            }))
                            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                cx.emit(PickEvent::Pick(at));
                            }))
                            .test_support()
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .size_full()
        .p(LIST_INSET)
        // The lane keeps the rows clear of the expanded thumb by the same inset.
        .when(scrolls, Styled::pr_0);
        div()
            .id("list")
            .role(Role::ListBox)
            .relative()
            .h(full.min(LIST_HEIGHT))
            .when(scrolls, |list| list.pr(Scrollbar::width()))
            .child(rows)
            .when(scrolls, |list| {
                // The scrollbar fills its parent and draws its track at the right edge, which
                // is the lane of the right padding.
                list.child(
                    div().absolute().inset_0().child(
                        Scrollbar::vertical(&self.scroll)
                            .mode(ScrollbarMode::Always)
                            .viewport_from_layout()
                            .styles(|styles| styles.track(|track| track.bg(transparent_black()))),
                    ),
                )
            })
    }
}

impl EventEmitter<PickEvent> for PickList {}

impl Render for PickList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .w_full()
            .key_context(Place::Picker.key_context())
            .on_action(cx.listener(Self::run))
            .child(
                div().flex_none().px_3().border_b_1().border_color(theme.border).child(
                    Input::new(&self.field)
                        .prefix(Icon::view(Icon::Search, theme.muted_foreground))
                        .appearance(false)
                        .p_0(),
                ),
            )
            .map(|this| {
                if self.rows.is_empty() {
                    this.child(
                        div()
                            .p_1()
                            .child(div().px_3().py_2().text_color(theme.muted_foreground).child(self.empty)),
                    )
                } else {
                    this.child(self.list(cx))
                }
            })
    }
}

/// The rows of a [`Dropdown`] list, inset from the panel by the same space on every side.
pub(crate) fn options() -> Div {
    v_flex().p_1().gap(px(2.))
}

/// A row of [`options`]. The row under the cursor, `current`, has the selection color.
pub(crate) fn option(id: impl Into<ElementId>, current: bool, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    // The row corners follow the panel corners across the inset.
    let radius = (theme.radius - px(2.)).max(px(0.));
    div()
        .id(id)
        .role(Role::ListBoxOption)
        .aria_selected(current)
        .px_2()
        .py_1()
        .rounded(radius)
        .cursor(CursorStyle::PointingHand)
        .when(current, |row| row.bg(theme.selection))
        .when(!current, |row| row.hover(|row| row.bg(theme.secondary)))
}

/// A popup under a part of a row, the opener. Its owner `T` decides when it is open.
///
/// - A mouse down in the popup keeps the focus where it is.
/// - A mouse down outside the popup and the opener closes it.
/// - The popup closes when the focus leaves it, in the same frame. When the focus goes to no
///   other part, it goes back to the text. So a closed popup never keeps the focus, and never
///   takes it from another part.
/// - The focus may move only to parts that the open popup already showed in the last frame.
///   A new part takes the panel away until the next frame.
pub(crate) struct Dropdown<T> {
    focus: FocusHandle,
    close: fn(&mut T, &mut Window, &mut Context<T>),
    /// The window bounds of the opener in the last frame.
    opener: Rc<Cell<Bounds<Pixels>>>,
    /// Whether the popup had the focus in the last frame.
    held: Rc<Cell<bool>>,
    _focus: [Subscription; 2],
}

impl<T: 'static> Dropdown<T> {
    /// `close` closes the popup in `T`. `text` is the part that gets the focus back.
    pub(crate) fn new(
        text: FocusHandle,
        close: fn(&mut T, &mut Window, &mut Context<T>),
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Self {
        let focus = cx.focus_handle();
        let held = Rc::new(Cell::new(false));
        let focus_in = cx.on_focus_in(&focus, window, {
            let held = held.clone();
            move |_, _, _| held.set(true)
        });
        let focus_out = cx.on_focus_out(&focus, window, {
            let held = held.clone();
            move |_, event, window, cx| {
                held.set(false);
                // GPUI calls this during a draw. A notify there asks for no new frame.
                cx.defer_in(window, close);
                // The focus still names the blurred part when the frame no longer has it.
                if window.focused(cx).is_some_and(|focused| event.blurred == focused) {
                    window.focus(&text, cx);
                }
            }
        });
        Self {
            focus,
            close,
            opener: Rc::default(),
            held,
            _focus: [focus_in, focus_out],
        }
    }

    /// Takes the bounds of the opener from `opener`, for an opener that another part draws and
    /// keeps the bounds of. Without it, [`Self::root`] keeps them.
    pub(crate) fn with_opener(mut self, opener: Rc<Cell<Bounds<Pixels>>>) -> Self {
        self.opener = opener;
        self
    }

    /// The window bounds of the opener in the last frame.
    pub(crate) fn opener(&self) -> Bounds<Pixels> {
        self.opener.get()
    }

    /// The handle that the popup tracks. The owner focuses it, or a part in the popup.
    pub(crate) fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    /// The root of the owner. Its first child is the opener, and the panel comes after it.
    pub(crate) fn root(&self) -> Div {
        let opener = self.opener.clone();
        div()
            .on_children_prepainted(move |bounds, _, _| {
                if let Some(bounds) = bounds.first() {
                    opener.set(*bounds);
                }
            })
            .relative()
    }

    /// The popup panel under the opener, `width` wide. The owner places it to the left or to
    /// the right.
    pub(crate) fn panel(&self, width: Pixels, window: &Window, cx: &Context<T>) -> Div {
        self.frame(width, window, cx).absolute().top_full().mt_1()
    }

    /// The popup panel, `width` wide, with no place. The owner places it.
    pub(crate) fn frame(&self, width: Pixels, window: &Window, cx: &Context<T>) -> Div {
        // GPUI tells about a focus move only after the next frame. This hides the panel in
        // that frame already.
        let left = self.held.get() && !self.focus.contains_focused(window, cx);
        let opener = self.opener.clone();
        let close = self.close;
        panel(width, cx)
            .track_focus(&self.focus)
            .occlude()
            .text_size(px(12.))
            // A popup in the note would take the line height of the note text.
            .line_height(phi())
            .when(left, Styled::hidden)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, window, cx| {
                    // The editor under the popup would move its cursor and take the focus.
                    window.prevent_default();
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down_out(cx.listener(move |owner, event: &MouseDownEvent, window, cx| {
                // The opener handles its own clicks.
                if !opener.get().contains(&event.position) {
                    close(owner, window, cx);
                }
            }))
    }
}
