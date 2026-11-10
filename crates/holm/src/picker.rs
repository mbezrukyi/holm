//! The folder picker for a move. `↑` and `↓` pick a folder, `Enter` moves there, `Esc` cancels.

use std::path::{Path, PathBuf};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement, Render, Role,
    ScrollStrategy, SharedString, StatefulInteractiveElement as _, Styled, TestSupportExt as _,
    UniformListScrollHandle, Window, div, px, uniform_list,
};

use crate::sidebar::Target;
use crate::{popup, text};

/// The answer of the picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PickerEvent {
    Move { target: Target, folder: PathBuf },
    Cancel,
}

pub(crate) struct FolderPicker {
    target: Target,
    /// The root folder first, as the empty path.
    folders: Vec<PathBuf>,
    selected: usize,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
}

impl FolderPicker {
    /// A picker for `target`, over `folders`. It leaves out the folder of the target.
    /// For a folder move, it also leaves out that folder and its children.
    pub(crate) fn new(
        target: Target,
        folders: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (path, inside) = match &target {
            Target::Note(path) => (path.as_path(), None),
            Target::Folder(path) => (path.as_path(), Some(path.as_path())),
        };
        let home = path.parent().unwrap_or(Path::new(""));
        let folders = std::iter::once(PathBuf::new())
            .chain(folders.iter().cloned())
            .filter(|folder| {
                folder != home && inside.is_none_or(|inside| !folder.starts_with(inside))
            })
            .collect();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            target,
            folders,
            selected: 0,
            focus,
            scroll: UniformListScrollHandle::new(),
        }
    }

    fn pick(&mut self, at: usize, cx: &mut Context<Self>) {
        if let Some(folder) = self.folders.get(at) {
            cx.emit(PickerEvent::Move {
                target: self.target.clone(),
                folder: folder.clone(),
            });
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        match event.keystroke.key.as_str() {
            "up" => self.selected = self.selected.saturating_sub(1),
            "down" => {
                self.selected = (self.selected + 1).min(self.folders.len().saturating_sub(1));
            }
            "enter" => self.pick(self.selected, cx),
            "escape" => cx.emit(PickerEvent::Cancel),
            _ => return,
        }
        self.scroll
            .scroll_to_item(self.selected, ScrollStrategy::Nearest);
        cx.notify();
        cx.stop_propagation();
    }
}

impl EventEmitter<PickerEvent> for FolderPicker {}

impl Focusable for FolderPicker {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for FolderPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let rows = uniform_list(
            "folders",
            self.folders.len(),
            cx.processor(|picker, range: std::ops::Range<usize>, _, cx| {
                let theme = cx.theme();
                range
                    .filter_map(|at| picker.folders.get(at).map(|folder| (at, folder)))
                    .map(|(at, folder)| {
                        let name: SharedString = if folder.as_os_str().is_empty() {
                            text::ROOT_FOLDER.into()
                        } else {
                            folder.display().to_string().into()
                        };
                        let selected = at == picker.selected;
                        div()
                            .id(ElementId::NamedInteger("folder-row".into(), at as u64))
                            .role(Role::ListItem)
                            .aria_label(name.clone())
                            .aria_selected(selected)
                            .test_support()
                            .w_full()
                            .h(px(28.))
                            .px_3()
                            .flex()
                            .items_center()
                            .truncate()
                            .child(name)
                            .on_click(cx.listener(move |picker, _: &ClickEvent, _, cx| {
                                picker.pick(at, cx);
                            }))
                            .when(selected, |row| row.bg(theme.selection))
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .w_full()
        .h(px(300.));
        popup::layer(cx)
            .id("folder-picker")
            .role(Role::Dialog)
            .aria_label(text::MOVE_TO)
            .test_support()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .child(
                popup::panel(px(360.), cx)
                    .py_2()
                    .child(
                        div()
                            .px_3()
                            .pb_2()
                            .text_color(theme.muted_foreground)
                            .child(text::MOVE_TO),
                    )
                    .child(rows),
            )
    }
}
