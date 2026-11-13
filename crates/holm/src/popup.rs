//! The look of a small window over the notes: a dim layer over the whole window, a panel in
//! its middle, and the rows of a list in the panel. Also [`Dropdown`], a popup under a part of
//! a row.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use gpui_kit::component::command::CommandItem;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Bounds, Context, Div, FocusHandle, InteractiveElement as _, MouseButton, MouseDownEvent,
    ParentElement as _, Pixels, Role, SharedString, StatefulInteractiveElement as _, Styled,
    Subscription, TestSupportExt as _, Window, div, px,
};
use holm_core::model::NoteInfo;

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

/// The row that a list in the panel shows when it has no rows.
pub(crate) fn empty(message: &'static str, cx: &App) -> Div {
    div()
        .px_3()
        .py_2()
        .text_color(cx.theme().muted_foreground)
        .child(message)
}

/// A row of a list in the panel.
pub(crate) fn item(label: impl Into<SharedString>, cx: &App) -> CommandItem {
    row(label.into(), None, cx)
}

/// The row of a note: its title, and its folder on the right. A note in the root folder shows
/// no folder.
pub(crate) fn note_item(note: &NoteInfo, cx: &App) -> CommandItem {
    row(note.title.clone().into(), folder(&note.path), cx)
}

fn row(label: SharedString, folder: Option<SharedString>, cx: &App) -> CommandItem {
    let shown = label.clone();
    let muted = cx.theme().muted_foreground;
    CommandItem::new().label(label).child(move |_, _| {
        h_flex()
            .flex_1()
            .min_w_0()
            .gap_2()
            .child(div().truncate().child(shown.clone()))
            .children(folder.clone().map(|folder| {
                div()
                    .id("folder")
                    .role(Role::Label)
                    .aria_label(folder.clone())
                    .test_support()
                    .ml_auto()
                    .flex_none()
                    .text_color(muted)
                    .child(folder)
            }))
    })
}

fn folder(note: &Path) -> Option<SharedString> {
    note.parent()
        .filter(|folder| !folder.as_os_str().is_empty())
        .map(|folder| folder.display().to_string().into())
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
                if window
                    .focused(cx)
                    .is_some_and(|focused| event.blurred == focused)
                {
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
        // GPUI tells about a focus move only after the next frame. This hides the panel in
        // that frame already.
        let left = self.held.get() && !self.focus.contains_focused(window, cx);
        let opener = self.opener.clone();
        let close = self.close;
        panel(width, cx)
            .track_focus(&self.focus)
            .absolute()
            .top_full()
            .mt_1()
            .py_1()
            .occlude()
            .text_size(px(12.))
            .when(left, Styled::hidden)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, window, cx| {
                    // The editor under the popup would move its cursor and take the focus.
                    window.prevent_default();
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down_out(
                cx.listener(move |owner, event: &MouseDownEvent, window, cx| {
                    // The opener handles its own clicks.
                    if !opener.get().contains(&event.position) {
                        close(owner, window, cx);
                    }
                }),
            )
    }
}
