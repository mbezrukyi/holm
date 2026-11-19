//! The settings dialog over the notes view. Each change goes to [`Config::change`], which
//! applies it at once.
//!
//! `Tab` and `Shift+Tab` move the cursor between rows and wrap at the ends. `Left` and
//! `Right` pick an option. `Space` switches a switch. `Space` or `Enter` on the notes folder
//! opens the folder picker. `Esc` closes the dialog.

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, Global,
    InteractiveElement as _, IntoElement, KeyBinding, KeyDownEvent, Keystroke, ParentElement,
    Render, Role, StatefulInteractiveElement as _, Styled, Subscription, TestSupportExt as _,
    Window, actions, div, px, transparent_black,
};
use holm_core::model::Order;
use holm_editor::WritingHelp;

use crate::config::{AppearanceSetting, Config, Settings};
use crate::path_text::PathText;
use crate::shortcuts::{self, Command, Place};
use crate::{popup, text, window};

actions!(settings, [Tab, ShiftTab]);

const KEY_CONTEXT: &str = "Settings";

/// Marks that the app has the bindings of [`Tab`] and [`ShiftTab`].
struct TabKeys;

impl Global for TabKeys {}

/// The dialog asks its owner to close it.
pub(crate) struct Close;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Folder,
    Theme,
    Order,
    FormatKeys,
    ContinueLists,
    IndentLists,
}

const ROWS: [Row; 6] = [
    Row::Folder,
    Row::Theme,
    Row::Order,
    Row::FormatKeys,
    Row::ContinueLists,
    Row::IndentLists,
];

const THEMES: [(AppearanceSetting, &str); 3] = [
    (AppearanceSetting::System, text::THEME_SYSTEM),
    (AppearanceSetting::Light, text::THEME_LIGHT),
    (AppearanceSetting::Dark, text::THEME_DARK),
];

const ORDERS: [(Order, &str); 2] = [
    (Order::LastChange, text::ORDER_LAST_CHANGE),
    (Order::Title, text::ORDER_TITLE),
];

impl Row {
    fn label(self) -> &'static str {
        match self {
            Self::Folder => text::SETTING_FOLDER,
            Self::Theme => text::SETTING_THEME,
            Self::Order => text::SETTING_ORDER,
            Self::FormatKeys => text::SETTING_FORMAT_KEYS,
            Self::ContinueLists => text::SETTING_CONTINUE_LISTS,
            Self::IndentLists => text::SETTING_INDENT_LISTS,
        }
    }

    /// The switch of a writing help row.
    fn switch(self, help: &mut WritingHelp) -> Option<&mut bool> {
        match self {
            Self::FormatKeys => Some(&mut help.format_keys),
            Self::ContinueLists => Some(&mut help.continue_lists),
            Self::IndentLists => Some(&mut help.indent_lists),
            Self::Folder | Self::Theme | Self::Order => None,
        }
    }
}

/// The option after or before `current` in `options`. It stops at the ends.
fn step<T: Copy + PartialEq>(options: &[(T, &str)], current: T, forward: bool) -> T {
    let at = options
        .iter()
        .position(|(option, _)| *option == current)
        .unwrap_or(0);
    let at = if forward {
        (at + 1).min(options.len() - 1)
    } else {
        at.saturating_sub(1)
    };
    options[at].0
}

pub(crate) struct SettingsDialog {
    config: Entity<Config>,
    /// The place of the cursor in [`ROWS`].
    cursor: usize,
    focus: FocusHandle,
    _config: Subscription,
}

impl SettingsDialog {
    /// The dialog with the cursor on the first row. It takes the focus.
    pub(crate) fn new(config: Entity<Config>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // A key binding runs before the key handlers. So the dialog needs bindings for Tab
        // and Shift+Tab, or the focus bindings of the window root take them.
        if !cx.has_global::<TabKeys>() {
            cx.set_global(TabKeys);
            cx.bind_keys([
                KeyBinding::new("tab", Tab, Some(KEY_CONTEXT)),
                KeyBinding::new("shift-tab", ShiftTab, Some(KEY_CONTEXT)),
            ]);
        }
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let subscription = cx.observe(&config, |_, _, cx| cx.notify());
        Self {
            config,
            cursor: 0,
            focus,
            _config: subscription,
        }
    }

    fn change(&self, change: impl FnOnce(&mut Settings), cx: &mut App) {
        self.config
            .update(cx, |config, cx| config.change(change, cx));
    }

    /// Opens the folder picker. The picked folder goes to the settings, also after the dialog
    /// closes.
    fn pick_folder(&self, cx: &mut Context<Self>) {
        let picked = window::pick_folder(cx);
        let config = self.config.downgrade();
        cx.spawn(async move |_, cx| {
            if let Some(folder) = picked.await {
                config
                    .update(cx, |config, cx| {
                        config.change(|settings| settings.notes_folder = Some(folder), cx);
                    })
                    .ok();
            }
        })
        .detach();
    }

    fn row(&self) -> Row {
        ROWS[self.cursor]
    }

    fn move_cursor(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.cursor = if forward {
            (self.cursor + 1) % ROWS.len()
        } else {
            (self.cursor + ROWS.len() - 1) % ROWS.len()
        };
        cx.notify();
    }

    /// Picks the next or the previous option of the row.
    fn pick_option(&self, forward: bool, cx: &mut App) {
        match self.row() {
            Row::Theme => self.change(
                |settings| settings.theme = step(&THEMES, settings.theme, forward),
                cx,
            ),
            Row::Order => self.change(
                |settings| settings.note_order = step(&ORDERS, settings.note_order, forward),
                cx,
            ),
            _ => {}
        }
    }

    fn toggle(&self, row: Row, cx: &mut App) {
        self.change(
            |settings| {
                if let Some(on) = row.switch(&mut settings.writing_help) {
                    *on = !*on;
                }
            },
            cx,
        );
    }

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.key(&event.keystroke, cx) {
            cx.stop_propagation();
        }
    }

    /// Tab or Shift+Tab from the key binding.
    fn tab(&mut self, keys: &'static str, cx: &mut Context<Self>) {
        if let Ok(keystroke) = Keystroke::parse(keys)
            && !self.key(&keystroke, cx)
        {
            cx.propagate();
        }
    }

    /// A key down. It returns whether the key stops here.
    fn key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        let row = self.row();
        match shortcuts::command(Place::Settings, keystroke) {
            Some(Command::NextSetting) => self.move_cursor(true, cx),
            Some(Command::PreviousSetting) => self.move_cursor(false, cx),
            Some(Command::SwitchSetting) if row == Row::Folder => self.pick_folder(cx),
            Some(Command::SwitchSetting) => self.toggle(row, cx),
            Some(Command::ChangeFolder) => {
                if row == Row::Folder {
                    self.pick_folder(cx);
                }
            }
            Some(Command::Close) => cx.emit(Close),
            _ => {
                let modifiers = keystroke.modifiers;
                if modifiers.modified() {
                    return false;
                }
                match keystroke.key.as_str() {
                    "left" => self.pick_option(false, cx),
                    "right" => self.pick_option(true, cx),
                    _ => return false,
                }
            }
        }
        true
    }

    fn control(row: Row, settings: &Settings, cx: &mut Context<Self>) -> impl IntoElement {
        match row {
            Row::Folder => {
                let folder = settings
                    .notes_folder
                    .as_deref()
                    .map(text::home_path)
                    .unwrap_or_default();
                h_flex()
                    .gap_2()
                    .min_w_0()
                    .child(
                        div()
                            .min_w_0()
                            .child(PathText::default().path(folder).one_line()),
                    )
                    .child(
                        Button::new("change-folder")
                            .label(text::CHANGE_FOLDER)
                            .outline()
                            .small()
                            .tab_stop(false)
                            .on_click(cx.listener(|dialog, _, _, cx| dialog.pick_folder(cx))),
                    )
                    .into_any_element()
            }
            Row::Theme => options("theme", &THEMES, settings.theme, cx, |settings, theme| {
                settings.theme = theme;
            })
            .into_any_element(),
            Row::Order => options(
                "order",
                &ORDERS,
                settings.note_order,
                cx,
                |settings, order| {
                    settings.note_order = order;
                },
            )
            .into_any_element(),
            Row::FormatKeys | Row::ContinueLists | Row::IndentLists => {
                let mut help = settings.writing_help;
                let on = row.switch(&mut help).is_some_and(|on| *on);
                Switch::new(row.label())
                    .checked(on)
                    .small()
                    .tab_stop(false)
                    .accessibility_label(row.label())
                    .on_click(cx.listener(move |dialog, _: &bool, _, cx| dialog.toggle(row, cx)))
                    .into_any_element()
            }
        }
    }
}

/// The buttons of a row with options. A click picks the option.
fn options<T: Copy + PartialEq + 'static>(
    id: &'static str,
    options: &'static [(T, &'static str)],
    current: T,
    cx: &mut Context<SettingsDialog>,
    set: fn(&mut Settings, T),
) -> ButtonGroup {
    ButtonGroup::new(id)
        .small()
        .children(options.iter().map(|(option, label)| {
            let selected = *option == current;
            Button::new(*label)
                .label(*label)
                .selected(selected)
                .when(selected, ButtonVariants::primary)
                .when(!selected, Button::outline)
                .tab_stop(false)
        }))
        .on_click(cx.listener(move |dialog, clicked: &Vec<usize>, _, cx| {
            if let Some((option, _)) = clicked.first().and_then(|at| options.get(*at)) {
                let option = *option;
                dialog.change(|settings| set(settings, option), cx);
            }
        }))
}

impl EventEmitter<Close> for SettingsDialog {}

impl Focusable for SettingsDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SettingsDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.config.read(cx).settings().clone();
        let theme = cx.theme();
        // The cursor row has a bar on its left and no fill, so the outlines of its controls stay
        // visible.
        let (bar, muted) = (theme.primary, theme.muted_foreground);
        let rows: Vec<_> = ROWS
            .iter()
            .enumerate()
            .map(|(at, &row)| {
                let current = at == self.cursor;
                let detail = (row == Row::FormatKeys).then(|| {
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(text::FORMAT_KEYS_DETAIL)
                });
                h_flex()
                    .id(("setting", at))
                    .role(Role::ListItem)
                    .aria_label(row.label())
                    .aria_selected(current)
                    .test_support()
                    .justify_between()
                    .gap_4()
                    .px_2()
                    .py_1p5()
                    .border_l(px(3.))
                    .border_color(if current { bar } else { transparent_black() })
                    .child(
                        v_flex()
                            .flex_none()
                            .child(
                                div()
                                    .when(current, |label| label.font_weight(FontWeight::MEDIUM))
                                    .child(row.label()),
                            )
                            .children(detail),
                    )
                    .child(Self::control(row, &settings, cx))
            })
            .collect();
        let panel = popup::panel(px(480.), cx)
            .p_3()
            .gap_1()
            .child(
                div()
                    .px_2()
                    .pb_1()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(text::SETTINGS),
            )
            .children(rows);
        popup::layer(cx)
            .id("settings")
            .role(Role::Dialog)
            .aria_label(text::SETTINGS)
            .test_support()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(|dialog, _: &Tab, _, cx| dialog.tab("tab", cx)))
            .on_action(cx.listener(|dialog, _: &ShiftTab, _, cx| dialog.tab("shift-tab", cx)))
            .child(panel)
    }
}
