//! The settings dialog over the notes view. Each change goes to [`Config::change`], which applies it at once.
//!
//! `Tab` and `Shift+Tab` move the cursor between rows and wrap at the ends. On a choice row, `Space` picks the next
//! option and `Shift+Space` the previous one, and the options wrap. `Space` switches a switch. `Space` or `Enter` on
//! the notes folder opens the folder picker. `Esc` closes the dialog.
//!
//! With a limited text width, the width shows after the options of its row, with a caret while the cursor is on the
//! row. Digits and `Backspace` there change the width; the first digit after the cursor comes to the row takes the
//! place of the whole width. A width in the range applies at once. When the cursor leaves the row, it shows the width
//! of the settings again.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement, Pixels, Render, Role, SharedString, StatefulInteractiveElement as _, Styled,
    Subscription, TestSupportExt as _, Window, div, px, rems, transparent_black,
};
use holm_core::model::Order;
use holm_editor::WritingHelp;
use holm_editor::shortcuts::{self as editor, Keys};

use crate::config::{AppearanceSetting, Config, Settings};
use crate::icons::Icon;
use crate::list_cursor::{self, Toward};
use crate::scale::Scale;
use crate::shortcuts::{self, Command, Place, Run};
use crate::{popup, text, text_field, window};

/// The dialog asks its owner to close it.
pub(crate) struct Close;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Folder,
    Theme,
    Scale,
    TextWidth,
    Order,
    FormatKeys,
    ContinueLists,
    IndentLists,
}

const ROWS: [Row; 8] = [
    Row::Folder,
    Row::Theme,
    Row::Scale,
    Row::TextWidth,
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

/// The height of a small button.
const BUTTON_HEIGHT: Pixels = px(22.);

/// The text size of the path in the folder button: the size of a path chip.
const PATH_SIZE: Pixels = px(12.5);

/// Whether the text width is limited.
const WIDTHS: [(bool, &str); 2] = [(false, text::WIDTH_FULL), (true, text::WIDTH_LIMITED)];

const MAX_CHARS_DIGITS: usize = 3;

const CHARS_BOX_PAD: Pixels = px(5.);

/// The height of a row of options with its frame. The text width row keeps it with or without the width box, so the
/// dialog does not change its height at any scale.
const CONTROL_HEIGHT: Pixels = px(24.);

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
            Self::Order => text::SETTING_ORDER,
            Self::FormatKeys => text::SETTING_FORMAT_KEYS,
            Self::ContinueLists => text::SETTING_CONTINUE_LISTS,
            Self::IndentLists => text::SETTING_INDENT_LISTS,
        }
    }

    fn switch(self, help: &mut WritingHelp) -> Option<&mut bool> {
        match self {
            Self::FormatKeys => Some(&mut help.format_keys),
            Self::ContinueLists => Some(&mut help.continue_lists),
            Self::IndentLists => Some(&mut help.indent_lists),
            Self::Folder | Self::Theme | Self::Scale | Self::TextWidth | Self::Order => None,
        }
    }
}

/// It wraps at the ends.
fn next_option<T: Copy + PartialEq>(options: &[(T, &str)], current: T, toward: Toward) -> T {
    let at = options.iter().position(|(option, _)| *option == current);
    let at = list_cursor::step(at, options.len(), toward, |_| true).unwrap_or(0);
    options[at].0
}

/// The width as the user types it on the text width row.
#[derive(Clone, Debug, PartialEq, Eq)]
struct CharsDraft {
    text: String,
    /// The next digit takes the place of the whole text.
    fresh: bool,
}

impl CharsDraft {
    /// The next digit replaces the whole width.
    fn new(chars: u16) -> Self {
        Self {
            text: chars.to_string(),
            fresh: true,
        }
    }

    /// Puts `digit` after the text, or in its place when it is fresh. A text with all its digits takes no more.
    fn type_digit(&mut self, digit: char) {
        if self.fresh {
            self.text.clear();
            self.fresh = false;
        }
        if self.text.len() < MAX_CHARS_DIGITS {
            self.text.push(digit);
        }
    }

    fn backspace(&mut self) {
        self.fresh = false;
        self.text.pop();
    }

    fn chars(&self) -> Option<i64> {
        self.text.parse().ok()
    }
}

pub(crate) struct SettingsDialog {
    config: Entity<Config>,
    /// An index into [`ROWS`].
    cursor: usize,
    focus: FocusHandle,
    chars: CharsDraft,
    _config: Subscription,
}

impl SettingsDialog {
    /// It takes the focus.
    pub(crate) fn new(config: Entity<Config>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let chars = CharsDraft::new(config.read(cx).settings().text_width.chars());
        let subscription = cx.observe(&config, |_, _, cx| cx.notify());
        Self {
            config,
            cursor: 0,
            focus,
            chars,
            _config: subscription,
        }
    }

    fn reset_chars(&mut self, cx: &App) {
        self.chars = CharsDraft::new(self.config.read(cx).settings().text_width.chars());
    }

    /// Digits and Backspace on the text width row with a limited width change the width. A width in the range applies
    /// at once.
    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if self.row() != Row::TextWidth
            || !self.config.read(cx).settings().text_width.limited
            || modifiers.control
            || modifiers.alt
            || modifiers.platform
        {
            return;
        }
        let mut chars = keystroke.key.chars();
        match (chars.next(), chars.next()) {
            (Some(digit), None) if digit.is_ascii_digit() => self.chars.type_digit(digit),
            _ if keystroke.key == "backspace" => self.chars.backspace(),
            _ => return,
        }
        cx.stop_propagation();
        if let Some(chars) = self.chars.chars() {
            self.change(|settings| _ = settings.text_width.set_chars(chars), cx);
        }
        cx.notify();
    }

    fn change(&self, change: impl FnOnce(&mut Settings), cx: &mut App) {
        self.config.update(cx, |config, cx| config.change(change, cx));
    }

    /// The picked folder goes to the settings, also after the dialog closes.
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

    /// The width shows the settings again.
    fn move_cursor(&mut self, toward: Toward, cx: &mut Context<Self>) {
        if let Some(at) = list_cursor::step(Some(self.cursor), ROWS.len(), toward, |_| true) {
            self.cursor = at;
            self.reset_chars(cx);
            cx.notify();
        }
    }

    /// The options wrap.
    fn pick_option(&mut self, toward: Toward, cx: &mut App) {
        match self.row() {
            Row::Theme => self.change(
                |settings| settings.theme = next_option(&THEMES, settings.theme, toward),
                cx,
            ),
            Row::Scale => self.change(
                |settings| {
                    settings.scale = next_option(Scale::options(), settings.scale, toward);
                },
                cx,
            ),
            Row::TextWidth => {
                self.change(
                    |settings| {
                        let width = &mut settings.text_width;
                        width.limited = next_option(&WIDTHS, width.limited, toward);
                    },
                    cx,
                );
                self.reset_chars(cx);
            }
            Row::Order => self.change(
                |settings| settings.note_order = next_option(&ORDERS, settings.note_order, toward),
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

    fn run(&mut self, Run(command): &Run, _window: &mut Window, cx: &mut Context<Self>) {
        let row = self.row();
        match command {
            Command::Next => self.move_cursor(Toward::Next, cx),
            Command::Previous => self.move_cursor(Toward::Previous, cx),
            Command::Switch => match row {
                Row::Folder => self.pick_folder(cx),
                Row::Theme | Row::Scale | Row::TextWidth | Row::Order => {
                    self.pick_option(Toward::Next, cx);
                }
                Row::FormatKeys | Row::ContinueLists | Row::IndentLists => self.toggle(row, cx),
            },
            Command::SwitchBack => self.pick_option(Toward::Previous, cx),
            Command::ChangeFolder => {
                if row == Row::Folder {
                    self.pick_folder(cx);
                }
            }
            Command::Close => cx.emit(Close),
            _ => cx.propagate(),
        }
    }

    fn control(&self, row: Row, settings: &Settings, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                let editing = self.row() == Row::TextWidth;
                let box_width = chars_box_width(window);
                let theme = cx.theme();
                let (muted, border, caret, radius) = (theme.muted_foreground, theme.border, theme.caret, theme.radius);
                // A plain frame and no focus ring: the dialog keeps the focus. Its width fits 3 digits, so the options
                // before it stay in place while the user types.
                let width = h_flex()
                    .id("text-width-chars")
                    .role(Role::TextInput)
                    .aria_label(text::TEXT_WIDTH_CHARS)
                    .aria_value(SharedString::from(self.chars.text.clone()))
                    .test_support()
                    .flex_none()
                    .h_full()
                    .w(box_width)
                    .px(CHARS_BOX_PAD)
                    .justify_end()
                    .border_1()
                    .border_color(border)
                    .rounded(radius)
                    .text_sm()
                    .child(text_field::label(self.chars.text.clone()))
                    .when(editing, |field| field.child(div().w(px(1.)).h(px(14.)).bg(caret)));
                h_flex()
                    .flex_none()
                    .h(CONTROL_HEIGHT)
                    .gap_2()
                    .child(options(&WIDTHS, limited, cx, |settings, limited| {
                        settings.text_width.limited = limited;
                    }))
                    .when(limited, |row| {
                        row.child(width)
                            .child(div().text_sm().text_color(muted).child(text::CHARS_UNIT))
                    })
                    .into_any_element()
            }
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

/// 3 digits of the small text, the caret, the padding and the border.
fn chars_box_width(window: &Window) -> Pixels {
    let style = window.text_style();
    let font_size = rems(0.875).to_pixels(window.rem_size());
    let digits = "0".repeat(MAX_CHARS_DIGITS);
    let run = style.to_run(digits.len());
    let digits = window
        .text_system()
        .shape_line(digits.into(), font_size, &[run], None)
        .width();
    // The label keeps room for a caret, and the caret of the box comes after it.
    (digits + px(2.) + CHARS_BOX_PAD * 2. + px(2.)).ceil()
}

/// From the editor key table: Bold, Italic, and the headings in one chip, like `Ctrl+1–6`.
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
    if let (Some(first), Some(last)) = (keys(editor::Command::Heading(1)), keys(editor::Command::Heading(6))) {
        let last = keys_text(Keys::plain(last.key));
        chips.push(format!("{}–{last}", keys_text(first)));
    }
    chips
}

/// Like `Ctrl+B`, as the shortcut list shows it.
fn keys_text(keys: Keys) -> String {
    Kbd::format(&shortcuts::keystroke(keys))
}

/// The format key chips, then a muted "…" for the other keys.
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
        .child(div().text_xs().text_color(theme.muted_foreground).child("…"))
}

/// A click picks the option.
///
/// The frame draws the outer border, and a line lies between each two buttons. The buttons have no border, so the
/// chosen one is as wide as the others. Its fill stays inside the outer border, and the lines next to it take its fill,
/// so it reads as one block.
fn options<T: Copy + PartialEq + 'static>(
    options: &'static [(T, &'static str)],
    current: T,
    cx: &mut Context<SettingsDialog>,
    set: fn(&mut Settings, T),
) -> impl IntoElement {
    let theme = cx.theme();
    let (fill, on_fill, line, radius) = (theme.primary, theme.primary_foreground, theme.border, theme.radius);
    let inner = radius - px(1.);
    let last = options.len() - 1;
    let chosen = options.iter().position(|(option, _)| *option == current);
    let mut frame = h_flex().flex_none().border_1().border_color(line).rounded(radius);
    for (at, (option, label)) in options.iter().enumerate() {
        if at > 0 {
            let touches = chosen.is_some_and(|chosen| chosen == at || chosen + 1 == at);
            frame = frame.child(div().w(px(1.)).h(BUTTON_HEIGHT).bg(if touches { fill } else { line }));
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.config.read(cx).settings().clone();
        let theme = cx.theme();
        // The cursor row has a bar on its left and no fill, so the outlines of its controls stay visible.
        let (bar, radius) = (theme.primary, theme.radius);
        let rows: Vec<_> = ROWS
            .iter()
            .enumerate()
            .map(|(at, &row)| {
                let current = at == self.cursor;
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
                    .child(self.control(row, &settings, window, cx))
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
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(Self::run))
            .child(panel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_digit_replaces_the_width_and_the_next_ones_add_to_it() {
        let mut draft = CharsDraft::new(80);
        draft.type_digit('1');
        assert_eq!(draft.chars(), Some(1));
        for digit in ['2', '0', '5'] {
            draft.type_digit(digit);
        }
        assert_eq!(draft.text, "120", "3 digits at most");
        draft.backspace();
        draft.backspace();
        draft.backspace();
        assert_eq!(draft.chars(), None);
    }

    #[test]
    fn backspace_on_a_fresh_width_edits_it() {
        let mut draft = CharsDraft::new(80);
        draft.backspace();
        draft.type_digit('5');
        assert_eq!(draft.text, "85");
    }
}
