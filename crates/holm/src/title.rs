//! The path of the open note at the right end of the tag row: its folders and its title, like
//! `Work › 2026 › Plans`. A long path cuts folders from the left, never the title. The folders
//! are only text.
//!
//! The title field takes the place of the title. It is a [`NameField`] with the write of the
//! owner. The title, the field and the name of a running write share one text element, so the
//! path does not move when the field opens or closes.
//! The field closes after [`NameEvent::Written`] and [`NameEvent::Closed`], and it sends each
//! [`NameEvent`] on to the owner. The owner decides what a blur does.

use std::path::Path;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::h_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement, Pixels, Render, Role, StatefulInteractiveElement as _,
    Styled, Subscription, TestSupportExt as _, Window, div, px,
};

use holm_core::NotePath;

use crate::name_field::{Look, NameEvent, NameField, NameWrite};
use crate::text;
use crate::text_field;

/// The widest the folders get. More folders are cut from the left.
const FOLDERS_WIDTH: Pixels = px(240.);

const TEXT_SIZE: Pixels = px(12.);

/// The line height of the path. It sets the place of the path in the middle of the tag row.
const LINE_HEIGHT: Pixels = px(14.);

pub(crate) struct TitleField<T> {
    /// The path of the open note.
    path: Option<NotePath>,
    write: NameWrite<T>,
    home: FocusHandle,
    field: Option<(Entity<NameField<T>>, Subscription)>,
}

impl<T: Clone + 'static> TitleField<T> {
    /// `write` gives the note a typed title. `home` gets the focus when the field closes.
    pub(crate) fn new(write: NameWrite<T>, home: FocusHandle) -> Self {
        Self {
            path: None,
            write,
            home,
            field: None,
        }
    }

    /// Shows the path of the open note.
    pub(crate) fn show(&mut self, path: Option<NotePath>, cx: &mut Context<Self>) {
        if self.path != path {
            self.path = path;
            cx.notify();
        }
    }

    fn title(&self) -> String {
        self.path
            .as_ref()
            .map(|path| path.title().into_owned())
            .unwrap_or_default()
    }

    /// Opens the field with the title, selected, and gives it the focus. An open field only
    /// gets the focus.
    pub(crate) fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((field, _)) = &self.field {
            field.update(cx, |field, cx| field.focus(window, cx));
            return;
        }
        let look = Look {
            id: "title-input",
            label: text::NOTE_TITLE.into(),
            placeholder: text::NOTE_TITLE.into(),
        };
        let (write, home) = (self.write.clone(), self.home.clone());
        let field = if self.path.is_some() {
            let title = self.title();
            cx.new(|cx| NameField::rename(look, &title, write, home, window, cx))
        } else {
            cx.new(|cx| NameField::new_name(look, write, home, window, cx))
        };
        let subscription = cx.subscribe(&field, |this, _, event: &NameEvent<T>, cx| {
            if !matches!(event, NameEvent::Blur) {
                this.close(cx);
            }
            cx.emit(event.clone());
        });
        self.field = Some((field, subscription));
        cx.notify();
    }

    /// Closes the field and shows the title again.
    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        if self.field.take().is_some() {
            cx.notify();
        }
    }
}

impl<T: 'static> EventEmitter<NameEvent<T>> for TitleField<T> {}

impl<T: Clone + 'static> Render for TitleField<T> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let folders: Vec<String> = self
            .path
            .as_ref()
            .map(|path| {
                let folder = path.folder();
                Path::new(folder.as_ref())
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
            Some((field, _)) => div()
                .flex_none()
                .text_color(theme.foreground)
                .child(field.clone())
                .into_any_element(),
            None => div()
                .id("note-title")
                .role(Role::Heading)
                .aria_label(title.clone())
                .test_support()
                .flex_none()
                .text_color(theme.foreground)
                .child(text_field::label(title))
                .into_any_element(),
        };
        h_flex()
            .id("note-path")
            .role(Role::Group)
            .aria_label(label)
            .test_support()
            .text_size(TEXT_SIZE)
            .line_height(LINE_HEIGHT)
            .when_some(folders, ParentElement::child)
            .child(end)
            // The tag row would open the tag popup.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
    }
}
