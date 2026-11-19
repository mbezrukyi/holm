//! The shortcut list over the notes view. Its rows come from [`shortcuts::list`], so it shows
//! each key of the app and the editor tables, and no other key.
//!
//! `Up` and `Down` scroll the list. `Esc` closes it.

use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::scroll::{Scrollbar, ScrollbarMode};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    App, Context, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement, Pixels, Render, Role, ScrollHandle,
    StatefulInteractiveElement as _, Styled, TestSupportExt as _, Window, div, point, px,
    transparent_black,
};

use crate::shortcuts::{self, Command, Group, Place, Shortcut};
use crate::{popup, text};

/// The groups of each column.
const COLUMNS: [&[Group]; 2] = [
    &[Group::Text],
    &[Group::Note, Group::Sidebar, Group::Window],
];

/// The most height of the rows. A longer list scrolls.
const LIST_HEIGHT: Pixels = px(560.);

/// How far `Up` and `Down` scroll: about three rows.
const SCROLL_STEP: Pixels = px(66.);

/// The text size of a key chip: a little bigger than the `Kbd` default.
const KEY_SIZE: Pixels = px(11.);

/// The list asks its owner to close it.
pub(crate) struct Close;

fn title(group: Group) -> &'static str {
    match group {
        Group::Text => text::GROUP_TEXT,
        Group::Note => text::GROUP_NOTE,
        Group::Sidebar => text::GROUP_SIDEBAR,
        Group::Window => text::GROUP_WINDOW,
    }
}

pub(crate) struct ShortcutList {
    shortcuts: Vec<Shortcut>,
    focus: FocusHandle,
    scroll: ScrollHandle,
}

impl ShortcutList {
    /// The list at its top. It takes the focus.
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            shortcuts: shortcuts::list(),
            focus,
            scroll: ScrollHandle::new(),
        }
    }

    /// Scrolls down, or up, by [`SCROLL_STEP`]. It stops at the ends.
    fn scroll_by(&self, down: bool, cx: &mut Context<Self>) {
        let offset = self.scroll.offset();
        let step = if down { -SCROLL_STEP } else { SCROLL_STEP };
        let y = (offset.y + step).clamp(-self.scroll.max_offset().y, px(0.));
        self.scroll.set_offset(point(offset.x, y));
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if shortcuts::command(Place::ShortcutList, keystroke) == Some(Command::Close) {
            cx.emit(Close);
        } else if keystroke.modifiers.modified() {
            return;
        } else {
            match keystroke.key.as_str() {
                "up" => self.scroll_by(false, cx),
                "down" => self.scroll_by(true, cx),
                _ => return,
            }
        }
        cx.stop_propagation();
    }

    fn group(&self, group: Group, cx: &App) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let rows = self
            .shortcuts
            .iter()
            .enumerate()
            .filter(|(_, shortcut)| shortcut.group == group)
            .map(|(at, shortcut)| {
                let keystroke = shortcuts::keystroke(shortcut.keys);
                h_flex()
                    .id(("shortcut", at))
                    .role(Role::ListItem)
                    .aria_label(format!("{}: {}", shortcut.label, Kbd::format(&keystroke)))
                    .test_support()
                    .justify_between()
                    .gap_3()
                    .py(px(2.))
                    .child(div().min_w_0().truncate().child(shortcut.label))
                    .child(
                        Kbd::new(keystroke)
                            .outline()
                            .text_size(KEY_SIZE)
                            .px(px(5.))
                            .py(px(2.)),
                    )
            });
        v_flex()
            .child(
                div()
                    .pt_2()
                    .pb_1()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.muted_foreground)
                    .child(title(group)),
            )
            .children(rows)
    }
}

impl EventEmitter<Close> for ShortcutList {}

impl Focusable for ShortcutList {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ShortcutList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let columns = COLUMNS.map(|groups| {
            v_flex()
                .flex_1()
                .min_w_0()
                .children(groups.iter().map(|&group| self.group(group, cx)))
        });
        let rows = div()
            .id("shortcut-rows")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .pr(Scrollbar::width())
            .child(h_flex().items_start().gap_6().children(columns));
        let list = div()
            .relative()
            .h(LIST_HEIGHT)
            .child(rows)
            // The scrollbar fills its parent and draws its track at the right edge, which is
            // the lane of the right padding.
            .child(
                div().absolute().inset_0().child(
                    Scrollbar::vertical(&self.scroll)
                        .mode(ScrollbarMode::Always)
                        .styles(|styles| styles.track(|track| track.bg(transparent_black()))),
                ),
            );
        let panel = popup::panel(px(760.), cx)
            .pl_5()
            .pr_2()
            .py_3()
            .child(
                div()
                    .pb_2()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(text::SHORTCUT_LIST),
            )
            .child(list);
        popup::layer(cx)
            .id("shortcuts")
            .role(Role::Dialog)
            .aria_label(text::SHORTCUT_LIST)
            .test_support()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .child(panel)
    }
}
