//! The path of the open note at the right end of the tag row: its folders and its title, like
//! `Work › 2026 › Plans`. A long path cuts folders from the left, never the title. The folders
//! are only text.
//!
//! The title field takes the place of the title. It changes no files itself.
//!
//! The field has no frame, and it is as wide as its text. So the path does not move when the
//! field opens.
//!
//! - `Enter` sends [`TitleEvent::Submit`] with the typed title, and locks the field until the
//!   pane ends the write.
//! - `Esc` sends [`TitleEvent::Cancel`].
//! - When the field loses the focus, it sends [`TitleEvent::Blur`].

use std::path::{Path, PathBuf};

use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement, Pixels, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, TestSupportExt as _, Window, div, px,
    transparent_black,
};

use crate::text;

/// The widest the folders get. More folders are cut from the left.
const FOLDERS_WIDTH: Pixels = px(240.);

const TEXT_SIZE: Pixels = px(12.);

/// The line height of the path and of the input. It is about the height of the font, so the
/// selection fits the text.
const LINE_HEIGHT: Pixels = px(14.);

/// The room after the text of the field. It lies outside the layout. The input grows one
/// frame after each key, so it needs room for one more char, or it scrolls.
const CARET_ROOM: Pixels = px(16.);

/// What the title field tells the pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TitleEvent {
    /// `Enter` with this title, trimmed.
    Submit(String),
    Cancel,
    Blur,
}

pub(crate) struct TitleField {
    /// The path of the open note.
    path: Option<PathBuf>,
    field: Option<Field>,
}

struct Field {
    input: Entity<InputState>,
    /// After `Enter`, the field stays locked until the pane ends the write.
    locked: bool,
    _subscription: Subscription,
}

impl TitleField {
    pub(crate) fn new() -> Self {
        Self {
            path: None,
            field: None,
        }
    }

    /// Shows the path of the open note.
    pub(crate) fn show(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.path != path {
            self.path = path;
            cx.notify();
        }
    }

    /// The title of the open note: its file name without `.md`.
    pub(crate) fn title(&self) -> String {
        self.path.as_deref().map(title_of).unwrap_or_default()
    }

    /// Opens the field with `text`, selected, and gives it the focus. An open field only gets
    /// the focus.
    pub(crate) fn open(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(field) = &self.field {
            if !field.locked {
                field.input.update(cx, |input, cx| input.focus(window, cx));
            }
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).default_value(text.to_owned()));
        let subscription = cx.subscribe(&input, |this, input, event, cx| {
            let Some(field) = this.field.as_mut().filter(|field| !field.locked) else {
                return;
            };
            match event {
                InputEvent::PressEnter { .. } => {
                    field.locked = true;
                    let title = input.read(cx).value().trim().to_owned();
                    cx.emit(TitleEvent::Submit(title));
                }
                InputEvent::Blur => cx.emit(TitleEvent::Blur),
                // The width follows the text.
                InputEvent::Change => cx.notify(),
                InputEvent::Focus => {}
            }
        });
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        self.field = Some(Field {
            input,
            locked: false,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// Opens the locked field again with its text, after the pane refused the title.
    pub(crate) fn unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(field) = self.field.as_mut().filter(|field| field.locked) {
            field.locked = false;
            field.input.update(cx, |input, cx| input.focus(window, cx));
        }
    }

    /// Closes the field and shows the title again.
    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        if self.field.take().is_some() {
            cx.notify();
        }
    }

    fn cancel(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.field.as_ref().is_some_and(|field| !field.locked) {
            cx.emit(TitleEvent::Cancel);
        }
    }
}

/// The file name of a note without `.md`.
pub(crate) fn title_of(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl EventEmitter<TitleEvent> for TitleField {}

impl Render for TitleField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let folders: Vec<String> = self
            .path
            .as_deref()
            .and_then(Path::parent)
            .map(|parent| {
                parent
                    .iter()
                    .map(|part| part.to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        let title = self.title();
        let label = folders
            .iter()
            .chain([&title])
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(text::PATH_GAP);
        let folders = (!folders.is_empty()).then(|| {
            div()
                .min_w_0()
                .max_w(FOLDERS_WIDTH)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis_start()
                .text_color(theme.muted_foreground)
                .child(format!(
                    "{}{}",
                    folders.join(text::PATH_GAP),
                    text::PATH_GAP
                ))
        });
        let end = match &self.field {
            Some(field) => field_element(field, window, cx),
            None => div()
                .id("note-title")
                .role(Role::Heading)
                .aria_label(title.clone())
                .test_support()
                .flex_none()
                .w(text_width(&title.clone().into(), window, cx))
                .whitespace_nowrap()
                .text_color(theme.foreground)
                .child(title)
                .into_any_element(),
        };
        h_flex()
            .id("note-path")
            .role(Role::Group)
            .aria_label(label)
            .test_support()
            .key_context("TitleField")
            .capture_action(cx.listener(Self::cancel))
            .text_size(TEXT_SIZE)
            .line_height(LINE_HEIGHT)
            .when_some(folders, ParentElement::child)
            .child(end)
            // The tag row would open the tag popup.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
    }
}

/// The open title field: as wide as its text.
///
/// The field draws its text like the title, in the same place. The input lies on top with
/// clear text, so it gives only the caret, the selection and the keys. Its own text would sit
/// a pixel apart, by other rounding.
fn field_element(field: &Field, window: &mut Window, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (shown, color) = match field.input.read(cx).value() {
        value if value.is_empty() => (text::NOTE_TITLE.into(), theme.muted_foreground),
        value => (value.clone(), theme.foreground),
    };
    let width = text_width(&shown, window, cx);
    div()
        .relative()
        .flex_none()
        .w(width + CARET_ROOM)
        // The caret room lies over the gap, so the parts after the field stay.
        .mr(-CARET_ROOM)
        .child(div().whitespace_nowrap().text_color(color).child(shown))
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                // The input is higher than its line. Centered, its line lies on the text.
                .flex()
                .items_center()
                .text_color(transparent_black())
                .child(
                    Input::new(&field.input)
                        .id("title-input")
                        .aria_label(text::NOTE_TITLE)
                        .appearance(false)
                        .small()
                        .px_0()
                        .py_0()
                        .line_height(LINE_HEIGHT)
                        .text_size(TEXT_SIZE),
                ),
        )
        .into_any_element()
}

/// The width of `text` in the path. The title and the field both take it, so they lay out
/// the same at any scale.
fn text_width(text: &SharedString, window: &Window, cx: &App) -> Pixels {
    let mut style = window.text_style();
    style.font_family = cx.theme().font_family.clone();
    let run = style.to_run(text.len());
    window
        .text_system()
        .shape_line(text.clone(), TEXT_SIZE, &[run], None)
        .width()
}
