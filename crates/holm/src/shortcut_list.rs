//! The shortcut list over the notes view. Its rows come from [`shortcuts::list`], so it shows
//! each key of the app and the editor tables, and no other key.
//!
//! Each group first shows the rows that work everywhere in it. Then the rows of each other place
//! follow under a heading, like "In a question", so their labels can stay short.
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

use holm_editor::shortcuts as editor;

use crate::shortcuts::{self, Command, Group, Place, Run, Shortcut};
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

/// The heading over the rows of `place`. The rows of the whole view, of the note, and of each
/// text input have none.
fn heading(place: Place) -> Option<&'static str> {
    match place {
        Place::Window
        | Place::Note
        | Place::Editor(editor::Place::Text | editor::Place::Editor) => None,
        Place::Sidebar => Some(text::IN_SIDEBAR),
        Place::Tree => Some(text::IN_TREE),
        Place::NoteList => Some(text::IN_NOTE_LIST),
        Place::Tags => Some(text::IN_TAG_ROW),
        Place::Dialog => Some(text::IN_QUESTION),
        Place::ListPopup => Some(text::IN_POPUP_LIST),
        Place::TextField => Some(text::IN_NAME_FIELD),
        Place::Settings => Some(text::IN_SETTINGS),
        Place::ShortcutList => Some(text::IN_SHORTCUT_LIST),
        Place::Editor(editor::Place::Find) => Some(text::IN_FIND),
        Place::Editor(editor::Place::ColorWindow) => Some(text::IN_COLOR_WINDOW),
        Place::Editor(editor::Place::HexField) => Some(text::IN_HEX_FIELD),
    }
}

/// The rows of `group` as places in `shortcuts`, in sections. The section with no heading
/// comes first. Then each heading follows in the order of its first row.
fn sections(shortcuts: &[Shortcut], group: Group) -> Vec<(Option<&'static str>, Vec<usize>)> {
    let mut sections: Vec<(Option<&'static str>, Vec<usize>)> = vec![(None, Vec::new())];
    for (at, shortcut) in shortcuts.iter().enumerate() {
        if shortcut.group != group {
            continue;
        }
        let heading = heading(shortcut.place);
        match sections.iter_mut().find(|(other, _)| *other == heading) {
            Some((_, rows)) => rows.push(at),
            None => sections.push((heading, vec![at])),
        }
    }
    sections.retain(|(_, rows)| !rows.is_empty());
    sections
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
        if keystroke.modifiers.modified() {
            return;
        }
        match keystroke.key.as_str() {
            "up" => self.scroll_by(false, cx),
            "down" => self.scroll_by(true, cx),
            _ => return,
        }
        cx.stop_propagation();
    }
    fn group(&self, group: Group, cx: &App) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let mut column = v_flex().child(
            div()
                .pt_2()
                .pb_1()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(muted)
                .child(title(group)),
        );
        for (heading, rows) in sections(&self.shortcuts, group) {
            if let Some(heading) = heading {
                column = column.child(
                    div()
                        .id(heading)
                        .role(Role::Heading)
                        .aria_label(heading)
                        .test_support()
                        .pt_2()
                        .text_sm()
                        .text_color(muted)
                        .child(heading),
                );
            }
            column = column.children(rows.into_iter().map(|at| self.row(at)));
        }
        column
    }

    /// The row of the shortcut at `at`. The label never shrinks, so a test can find a label
    /// that runs into its keys.
    fn row(&self, at: usize) -> impl IntoElement + use<> {
        let shortcut = self.shortcuts[at];
        let keystroke = shortcuts::keystroke(shortcut.keys);
        h_flex()
            .id(("shortcut", at))
            .role(Role::ListItem)
            .aria_label(format!("{}: {}", shortcut.label, Kbd::format(&keystroke)))
            .test_support()
            .justify_between()
            .gap_3()
            .py(px(2.))
            .child(
                div()
                    .id("label")
                    .test_support()
                    .flex_none()
                    .whitespace_nowrap()
                    .child(shortcut.label),
            )
            .child(
                div()
                    .id("keys")
                    .test_support()
                    .flex_none()
                    .child(key_chip(shortcut.keys)),
            )
    }
}

/// The chip of `keys`, as the shortcut list shows it.
pub(crate) fn key_chip(keys: editor::Keys) -> Kbd {
    Kbd::new(shortcuts::keystroke(keys))
        .outline()
        .text_size(KEY_SIZE)
        .px(px(5.))
        .py(px(2.))
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
            .key_context(Place::ShortcutList.key_context())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(|_, Run(command): &Run, _, cx| match command {
                Command::Close => cx.emit(Close),
                _ => cx.propagate(),
            }))
            .child(panel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_group_shows_each_of_its_rows_once_and_the_plain_rows_first() {
        let list = shortcuts::list();
        let mut shown = Vec::new();
        for group in [Group::Text, Group::Note, Group::Sidebar, Group::Window] {
            let sections = sections(&list, group);
            let mut headings: Vec<_> = sections.iter().map(|(heading, _)| *heading).collect();
            assert!(headings.iter().skip(1).all(Option::is_some), "{group:?}");
            headings.dedup();
            assert_eq!(headings.len(), sections.len(), "one section per heading");
            for (heading, rows) in sections {
                for at in rows {
                    assert_eq!(list[at].group, group);
                    assert_eq!(super::heading(list[at].place), heading);
                    shown.push(at);
                }
            }
        }
        shown.sort_unstable();
        assert_eq!(shown, (0..list.len()).collect::<Vec<_>>());
    }
}
