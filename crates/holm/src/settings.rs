//! The settings dialog over the notes view. Each change goes to [`Config::change`], which
//! applies it at once.
//!
//! `Tab` and `Shift+Tab` move the cursor between rows and wrap at the ends. On a choice row,
//! `Space` picks the next option and `Shift+Space` the previous one, and the options wrap.
//! `Space` switches a switch. `Space` or `Enter` on the notes folder opens the folder picker.
//! `Esc` closes the dialog.
//!
//! With a limited text width, the width field shows after its options, and `Tab` puts the
//! cursor and the focus in it. The field takes digits only. A width in the range applies at
//! once. When the field loses the focus, it shows the width of the settings again.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement, Pixels, Render, Role,
    StatefulInteractiveElement as _, Styled, Subscription, TestSupportExt as _, Window, div, px,
    transparent_black,
};
use holm_core::model::Order;
use holm_editor::WritingHelp;
use holm_editor::shortcuts::{self as editor, Keys};

use crate::config::{AppearanceSetting, Config, Settings};
use crate::icons::Icon;
use crate::list_cursor;
use crate::scale::Scale;
use crate::shortcuts::{self, Command, Place, Run};
use crate::{popup, text, window};

/// The dialog asks its owner to close it.
pub(crate) struct Close;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Folder,
    Theme,
    Scale,
    TextWidth,
    /// The width field in the text width row. The cursor reaches it only with a limited width.
    TextChars,
    Order,
    FormatKeys,
    ContinueLists,
    IndentLists,
}

/// The places of the cursor, in order.
const ROWS: [Row; 9] = [
    Row::Folder,
    Row::Theme,
    Row::Scale,
    Row::TextWidth,
    Row::TextChars,
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

/// The height of an option button inside its frame: the height of a small button.
const BUTTON_HEIGHT: Pixels = px(22.);

/// The size of the path in the folder button: the size of a path chip.
const PATH_SIZE: Pixels = px(12.5);

/// Whether the text width is limited.
const WIDTHS: [(bool, &str); 2] = [(false, text::WIDTH_FULL), (true, text::WIDTH_LIMITED)];

/// The width of the width field, for 3 digits.
const CHARS_FIELD_WIDTH: Pixels = px(52.);

const ORDERS: [(Order, &str); 2] = [
    (Order::LastChange, text::ORDER_LAST_CHANGE),
    (Order::Title, text::ORDER_TITLE),
];

impl Row {
    fn label(self) -> &'static str {
        match self {
            Self::Folder => text::SETTING_FOLDER,
            Self::Theme => text::SETTING_THEME,
            Self::Scale => text::SETTING_SCALE,
            Self::TextWidth => text::SETTING_TEXT_WIDTH,
            Self::TextChars => text::TEXT_WIDTH_CHARS,
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
            Self::Folder
            | Self::Theme
            | Self::Scale
            | Self::TextWidth
            | Self::TextChars
            | Self::Order => None,
        }
    }
}

/// The option after or before `current` in `options`. It wraps at the ends.
fn next_option<T: Copy + PartialEq>(options: &[(T, &str)], current: T, forward: bool) -> T {
    let at = options.iter().position(|(option, _)| *option == current);
    let at = list_cursor::step(at, options.len(), forward, |_| true).unwrap_or(0);
    options[at].0
}

pub(crate) struct SettingsDialog {
    config: Entity<Config>,
    /// The place of the cursor in [`ROWS`].
    cursor: usize,
    focus: FocusHandle,
    /// The width field of the text width row.
    chars: Entity<InputState>,
    _subscriptions: [Subscription; 2],
}

impl SettingsDialog {
    /// The dialog with the cursor on the first row. It takes the focus.
    pub(crate) fn new(config: Entity<Config>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let chars = config.read(cx).settings().text_width.chars().to_string();
        let chars = cx.new(|cx| {
            InputState::new(window, cx)
                .validate(|text, _| {
                    text.len() <= 3 && text.bytes().all(|byte| byte.is_ascii_digit())
                })
                .default_value(chars)
        });
        let subscriptions = [
            cx.observe_in(&config, window, |dialog, _, window, cx| {
                dialog.leave_hidden_field(window, cx);
                cx.notify();
            }),
            cx.subscribe_in(&chars, window, Self::chars_event),
        ];
        Self {
            config,
            cursor: 0,
            focus,
            chars,
            _subscriptions: subscriptions,
        }
    }

    fn limited(&self, cx: &App) -> bool {
        self.config.read(cx).settings().text_width.limited
    }

    /// Whether the cursor can be on `row`.
    fn reaches(&self, row: Row, cx: &App) -> bool {
        row != Row::TextChars || self.limited(cx)
    }

    /// A typed width applies when it is in the range. When the field loses the focus, it shows
    /// the width of the settings again.
    fn chars_event(
        &mut self,
        field: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                if let Ok(chars) = field.read(cx).value().parse::<i64>() {
                    self.change(|settings| _ = settings.text_width.set_chars(chars), cx);
                }
            }
            InputEvent::Focus => {
                if let Some(at) = ROWS.iter().position(|row| *row == Row::TextChars) {
                    self.cursor = at;
                    cx.notify();
                }
            }
            InputEvent::Blur => {
                let chars = self
                    .config
                    .read(cx)
                    .settings()
                    .text_width
                    .chars()
                    .to_string();
                if field.read(cx).value() != chars {
                    field.update(cx, |field, cx| field.set_value(chars, window, cx));
                }
            }
            InputEvent::PressEnter { .. } => {}
        }
    }

    /// Moves the cursor off the width field when the field hides, like after a click on "Full".
    fn leave_hidden_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.row() == Row::TextChars && !self.limited(cx) {
            self.cursor -= 1;
            window.focus(&self.focus, cx);
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

    /// Moves the cursor to the next or the previous row it reaches. The focus goes into the
    /// width field with the cursor, and all its text is selected, so typing replaces it.
    fn move_cursor(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(at) = list_cursor::step(Some(self.cursor), ROWS.len(), forward, |at| {
            self.reaches(ROWS[at], cx)
        }) else {
            return;
        };
        self.cursor = at;
        if self.row() == Row::TextChars {
            self.chars.update(cx, |field, cx| {
                field.focus(window, cx);
                field.select_all(window, cx);
            });
        } else {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    /// Picks the next or the previous option of the row. The options wrap.
    fn pick_option(&self, forward: bool, cx: &mut App) {
        match self.row() {
            Row::Theme => self.change(
                |settings| settings.theme = next_option(&THEMES, settings.theme, forward),
                cx,
            ),
            Row::Scale => self.change(
                |settings| {
                    settings.scale = next_option(Scale::options(), settings.scale, forward);
                },
                cx,
            ),
            Row::TextWidth => self.change(
                |settings| {
                    let width = &mut settings.text_width;
                    width.limited = next_option(&WIDTHS, width.limited, forward);
                },
                cx,
            ),
            Row::Order => self.change(
                |settings| settings.note_order = next_option(&ORDERS, settings.note_order, forward),
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

    fn run(&mut self, Run(command): &Run, window: &mut Window, cx: &mut Context<Self>) {
        let row = self.row();
        match command {
            Command::Next => self.move_cursor(true, window, cx),
            Command::Previous => self.move_cursor(false, window, cx),
            Command::Switch => match row {
                Row::Folder => self.pick_folder(cx),
                Row::Theme | Row::Scale | Row::TextWidth | Row::Order => {
                    self.pick_option(true, cx);
                }
                Row::TextChars => {}
                Row::FormatKeys | Row::ContinueLists | Row::IndentLists => self.toggle(row, cx),
            },
            Command::SwitchBack => self.pick_option(false, cx),
            Command::ChangeFolder => {
                if row == Row::Folder {
                    self.pick_folder(cx);
                }
            }
            Command::Close => cx.emit(Close),
            _ => cx.propagate(),
        }
    }

    fn control(&self, row: Row, settings: &Settings, cx: &mut Context<Self>) -> impl IntoElement {
        match row {
            Row::Folder => {
                let folder = settings
                    .notes_folder
                    .as_deref()
                    .map(text::home_path)
                    .unwrap_or_default();
                let theme = cx.theme();
                let (muted, mono) = (theme.muted_foreground, theme.mono_font_family.clone());
                // The button shrinks below the width of its path, so a long path ends in "…".
                Button::new("notes-folder")
                    .outline()
                    .small()
                    .flex_shrink(1.)
                    .min_w_0()
                    .accessibility_label(folder.clone())
                    .child(Icon::Folder.view(muted))
                    .child(
                        div()
                            .min_w_0()
                            .text_ellipsis()
                            .font_family(mono)
                            .text_size(PATH_SIZE)
                            .child(folder),
                    )
                    .on_click(cx.listener(|dialog, _, _, cx| dialog.pick_folder(cx)))
                    .into_any_element()
            }
            Row::Theme => options(&THEMES, settings.theme, cx, |settings, theme| {
                settings.theme = theme;
            })
            .into_any_element(),
            Row::Scale => options(Scale::options(), settings.scale, cx, |settings, scale| {
                settings.scale = scale;
            })
            .into_any_element(),
            Row::TextWidth => {
                let limited = settings.text_width.limited;
                let theme = cx.theme();
                let muted = theme.muted_foreground;
                h_flex()
                    .flex_none()
                    .gap_2()
                    .child(options(&WIDTHS, limited, cx, |settings, limited| {
                        settings.text_width.limited = limited;
                    }))
                    .when(limited, |row| {
                        row.child(
                            Input::new(&self.chars)
                                .id("text-width-chars")
                                .aria_label(text::TEXT_WIDTH_CHARS)
                                .small()
                                .w(CHARS_FIELD_WIDTH),
                        )
                        .child(div().text_color(muted).child(text::CHARS_UNIT))
                    })
                    .into_any_element()
            }
            // The text width row shows the field.
            Row::TextChars => div().into_any_element(),
            Row::Order => options(&ORDERS, settings.note_order, cx, |settings, order| {
                settings.note_order = order;
            })
            .into_any_element(),
            Row::FormatKeys | Row::ContinueLists | Row::IndentLists => {
                let mut help = settings.writing_help;
                let on = row.switch(&mut help).is_some_and(|on| *on);
                Switch::new(row.label())
                    .checked(on)
                    .small()
                    .accessibility_label(row.label())
                    .on_click(cx.listener(move |dialog, _: &bool, _, cx| dialog.toggle(row, cx)))
                    .into_any_element()
            }
        }
    }
}

/// The format keys the settings show, from the editor key table: Bold, Italic, and the
/// headings in one chip, like `Ctrl+1–6`.
fn format_keys() -> Vec<String> {
    let keys = |command| {
        editor::shortcuts()
            .iter()
            .find(|shortcut| shortcut.command == command)
            .map(|shortcut| shortcut.keys)
    };
    let mut chips: Vec<_> = [editor::Command::Bold, editor::Command::Italic]
        .into_iter()
        .filter_map(keys)
        .map(keys_text)
        .collect();
    if let (Some(first), Some(last)) = (
        keys(editor::Command::Heading(1)),
        keys(editor::Command::Heading(6)),
    ) {
        let last = keys_text(Keys::plain(last.key));
        chips.push(format!("{}–{last}", keys_text(first)));
    }
    chips
}

/// The text of `keys`, like `Ctrl+B`, as the shortcut list shows it.
fn keys_text(keys: Keys) -> String {
    Kbd::format(&shortcuts::keystroke(keys))
}

/// The key chips of the format keys row, then a muted "…" for the other keys.
fn format_key_chips(cx: &App) -> impl IntoElement + use<> {
    let theme = cx.theme();
    let chips = format_keys().into_iter().enumerate().map(|(at, keys)| {
        div()
            .id(("format-key", at))
            .role(Role::Label)
            .aria_label(keys.clone())
            .test_support()
            .px(px(4.))
            .text_size(px(9.))
            .line_height(px(14.))
            .whitespace_nowrap()
            .border_1()
            .border_color(theme.border)
            .rounded(px(4.))
            .bg(theme.sidebar)
            .child(keys)
    });
    h_flex()
        .id("format-keys")
        .test_support()
        .mt(px(3.))
        .gap(px(3.))
        .children(chips)
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child("…"),
        )
}

/// The buttons of a row with options, in one frame. A click picks the option.
///
/// The frame draws the outer border, and a line lies between each two buttons. The buttons
/// have no border, so the chosen one is as wide as the others. Its fill stays inside the outer
/// border, and the lines next to it take its fill, so it reads as one block.
fn options<T: Copy + PartialEq + 'static>(
    options: &'static [(T, &'static str)],
    current: T,
    cx: &mut Context<SettingsDialog>,
    set: fn(&mut Settings, T),
) -> impl IntoElement {
    let theme = cx.theme();
    let (fill, on_fill, line, radius) = (
        theme.primary,
        theme.primary_foreground,
        theme.border,
        theme.radius,
    );
    let inner = radius - px(1.);
    let last = options.len() - 1;
    let chosen = options.iter().position(|(option, _)| *option == current);
    let mut frame = h_flex()
        .flex_none()
        .border_1()
        .border_color(line)
        .rounded(radius);
    for (at, (option, label)) in options.iter().enumerate() {
        if at > 0 {
            let touches = chosen.is_some_and(|chosen| chosen == at || chosen + 1 == at);
            frame =
                frame.child(
                    div()
                        .w(px(1.))
                        .h(BUTTON_HEIGHT)
                        .bg(if touches { fill } else { line }),
                );
        }
        let selected = chosen == Some(at);
        let option = *option;
        frame = frame.child(
            Button::new(*label)
                .label(*label)
                .ghost()
                .small()
                .h(BUTTON_HEIGHT)
                .rounded_none()
                .when(at == 0, |button| button.rounded_l(inner))
                .when(at == last, |button| button.rounded_r(inner))
                .selected(selected)
                .toggled(selected)
                .when(selected, |button| button.bg(fill).text_color(on_fill))
                .on_click(cx.listener(move |dialog, _, _, cx| {
                    dialog.change(|settings| set(settings, option), cx);
                })),
        );
    }
    frame
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
        let (bar, radius) = (theme.primary, theme.radius);
        // The cursor in the width field marks the text width row.
        let cursor_row = match self.row() {
            Row::TextChars => Row::TextWidth,
            row => row,
        };
        let rows: Vec<_> = ROWS
            .iter()
            .enumerate()
            .filter(|(_, row)| **row != Row::TextChars)
            .map(|(at, &row)| {
                let current = row == cursor_row;
                let detail = (row == Row::FormatKeys).then(|| format_key_chips(cx));
                h_flex()
                    .id(("setting", at))
                    .role(Role::ListItem)
                    .aria_label(row.label())
                    .aria_selected(current)
                    .test_support()
                    .justify_between()
                    .gap_4()
                    // The bar lies inside the left padding, so the label does not move.
                    .pl(px(5.))
                    .pr_2()
                    .py_1p5()
                    .rounded(radius)
                    .border_l(px(3.))
                    .border_color(if current { bar } else { transparent_black() })
                    .child(
                        v_flex()
                            .flex_none()
                            .child(
                                div()
                                    .when(current, |label| label.font_weight(FontWeight::SEMIBOLD))
                                    .child(row.label()),
                            )
                            .children(detail),
                    )
                    .child(self.control(row, &settings, cx))
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
            .key_context(Place::Settings.key_context())
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::run))
            .child(panel)
    }
}
