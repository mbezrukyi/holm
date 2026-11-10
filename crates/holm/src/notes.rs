//! The open notes folder: the sidebar, the editor pane, and the file actions between them.
//!
//! Each file action saves the open note first and runs the core call off the UI thread.
//! The sidebar then reads the notes again. Holm's own changes don't come back as changes
//! from the core.

use std::path::{Path, PathBuf};

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AppContext as _, Context, CursorStyle, Entity, Focusable as _, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    Render, Styled, Subscription, Task, Window, div, px,
};
use holm_core::Core;
use holm_core::model::{Note, Order};

use crate::bar::{self, Bar};
use crate::config::{Config, SidebarSettings};
use crate::dialog::{DeleteDialog, DialogEvent};
use crate::error::AppError;
use crate::pane::{Leave, NotePane, PaneEvent};
use crate::picker::{FolderPicker, PickerEvent};
use crate::sidebar::{Item, Sidebar, SidebarEvent, Target};
use crate::text;

const MIN_WIDTH: f32 = 160.;
const MAX_WIDTH: f32 = 600.;
/// The least width the drag leaves for the editor.
const MIN_EDITOR: f32 = 360.;

pub(crate) struct NotesView {
    core: Core,
    config: Entity<Config>,
    sidebar: Entity<Sidebar>,
    pane: Entity<NotePane>,
    hidden: bool,
    width: Pixels,
    resizing: bool,
    dialog: Option<Entity<DeleteDialog>>,
    picker: Option<Entity<FolderPicker>>,
    error: Option<&'static str>,
    action: Option<Task<()>>,
    subscriptions: Vec<Subscription>,
}

impl NotesView {
    pub(crate) fn new(
        core: Core,
        notes_folder: PathBuf,
        note: Note,
        config: Entity<Config>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = config.read(cx).settings().clone();
        let current = note.info.path.clone();
        let pane = cx.new(|cx| NotePane::new(core.clone(), notes_folder, note, window, cx));
        let sidebar = cx.new(|cx| {
            Sidebar::new(
                core.clone(),
                settings.note_order,
                settings.sidebar.view.clone(),
                Some(current),
                cx,
            )
        });
        let subscriptions = vec![
            cx.subscribe_in(&sidebar, window, Self::sidebar_event),
            cx.subscribe(&pane, |this, _, event: &PaneEvent, cx| match event {
                PaneEvent::Opened(path) => {
                    let path = path.clone();
                    this.sidebar
                        .update(cx, |sidebar, cx| sidebar.set_current(path, cx));
                    this.keep_state(cx);
                }
                PaneEvent::Wrote => this.sidebar.update(cx, Sidebar::refresh),
            }),
        ];
        let mut this = Self {
            core,
            config,
            sidebar,
            pane,
            hidden: settings.sidebar.hidden,
            width: px(f32::from(settings.sidebar.width)),
            resizing: false,
            dialog: None,
            picker: None,
            error: None,
            action: None,
            subscriptions,
        };
        this.keep_state(cx);
        this
    }

    fn sidebar_event(
        &mut self,
        _: &Entity<Sidebar>,
        event: &SidebarEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SidebarEvent::Open(path) => {
                let path = path.clone();
                self.pane
                    .update(cx, |pane, cx| pane.open(path, Leave::Keep, cx));
            }
            SidebarEvent::FocusEditor => self.focus_editor(window, cx),
            SidebarEvent::EditTitle => self
                .pane
                .update(cx, |pane, cx| pane.select_title(window, cx)),
            SidebarEvent::Delete(target) => self.ask_delete(target.clone(), window, cx),
            SidebarEvent::Move(target) => {
                let folders = self.sidebar.read(cx).folders().to_vec();
                let target = target.clone();
                let picker = cx.new(|cx| FolderPicker::new(target, &folders, window, cx));
                self.subscriptions
                    .push(cx.subscribe_in(&picker, window, Self::picker_event));
                self.picker = Some(picker);
                cx.notify();
            }
            SidebarEvent::RenameFolder { folder, name } => {
                let (folder, name) = (folder.clone(), name.clone());
                self.run(
                    window,
                    cx,
                    {
                        let folder = folder.clone();
                        move |core| core.rename_folder(&folder, &name)
                    },
                    move |this, new, _, cx| this.folder_moved(&folder, new, cx),
                );
            }
            SidebarEvent::ViewChanged => self.keep_state(cx),
        }
    }

    fn picker_event(
        &mut self,
        _: &Entity<FolderPicker>,
        event: &PickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.picker = None;
        self.focus_sidebar(window, cx);
        cx.notify();
        let PickerEvent::Move { target, folder } = event.clone() else {
            return;
        };
        match target {
            Target::Note(path) => self.run(
                window,
                cx,
                {
                    let path = path.clone();
                    move |core| core.move_note(&path, &folder).map(|info| info.path)
                },
                move |this, new, _, cx| {
                    this.pane.update(cx, |pane, cx| pane.moved(&path, &new, cx));
                },
            ),
            Target::Folder(path) => self.run(
                window,
                cx,
                {
                    let path = path.clone();
                    move |core| core.move_folder(&path, &folder)
                },
                move |this, new, _, cx| this.folder_moved(&path, new, cx),
            ),
        }
    }

    fn folder_moved(&mut self, from: &Path, to: PathBuf, cx: &mut Context<Self>) {
        self.pane.update(cx, |pane, cx| pane.moved(from, &to, cx));
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.show_in_tree(Item::Folder(to), cx));
    }

    /// Shows the delete dialog, or deletes at once after "Do not show again".
    fn ask_delete(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        if !self.config.read(cx).settings().ask_before_delete {
            self.delete(target, window, cx);
            return;
        }
        self.run(
            window,
            cx,
            {
                let target = target.clone();
                move |core| match &target {
                    Target::Note(path) => core.note_delete_count(path),
                    Target::Folder(path) => core.folder_delete_count(path),
                }
            },
            move |this, count, window, cx| {
                let path = match &target {
                    Target::Note(path) | Target::Folder(path) => path,
                };
                let name = path
                    .file_stem()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let question =
                    text::delete_question(&format!("“{name}”"), count.notes, count.attachments);
                let dialog = cx.new(|cx| DeleteDialog::new(target, question, window, cx));
                this.subscriptions
                    .push(cx.subscribe_in(&dialog, window, Self::dialog_event));
                this.dialog = Some(dialog);
                cx.notify();
            },
        );
    }

    fn dialog_event(
        &mut self,
        _: &Entity<DeleteDialog>,
        event: &DialogEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dialog = None;
        self.focus_sidebar(window, cx);
        cx.notify();
        if let DialogEvent::Delete { target, do_not_ask } = event.clone() {
            if do_not_ask {
                let mut settings = self.config.read(cx).settings().clone();
                settings.ask_before_delete = false;
                self.save_settings(settings, cx);
            }
            self.delete(target, window, cx);
        }
    }

    fn delete(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        let open = self.pane.read(cx).path().to_owned();
        let open_gone = match &target {
            Target::Note(path) => path == &open,
            Target::Folder(folder) => open.starts_with(folder),
        };
        let next = self.sidebar.read(cx).neighbor(&open);
        let order = self.config.read(cx).settings().note_order;
        self.run(
            window,
            cx,
            move |core| {
                match &target {
                    Target::Note(path) => core.delete_note(path)?,
                    Target::Folder(path) => core.delete_folder(path)?,
                }
                if !open_gone {
                    return Ok(None);
                }
                let next = next.filter(|next| core.read(next).is_ok());
                let next = match next {
                    Some(next) => next,
                    None => match core.notes(order)?.into_iter().next() {
                        Some(note) => note.path,
                        None => core.create("")?.info.path,
                    },
                };
                Ok(Some(next))
            },
            |this, next, _, cx| {
                if let Some(next) = next {
                    this.pane
                        .update(cx, |pane, cx| pane.open(next, Leave::Drop, cx));
                }
            },
        );
    }

    fn new_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (folder, tag) = self.sidebar.read(cx).new_note_place();
        self.run(
            window,
            cx,
            move |core| {
                let note = core.create(&folder)?;
                match tag {
                    Some(tag) => core.set_tags(&note.info.path, &[tag]).map(|info| info.path),
                    None => Ok(note.info.path),
                }
            },
            |this, path, window, cx| {
                this.pane
                    .update(cx, |pane, cx| pane.open(path, Leave::Keep, cx));
                this.focus_editor(window, cx);
            },
        );
    }

    fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent = match self.sidebar.read(cx).view().item() {
            Item::Folder(folder) => folder.clone(),
            Item::All | Item::Tag(_) => PathBuf::new(),
        };
        self.run(
            window,
            cx,
            move |core| core.create_folder(&parent, text::NEW_FOLDER),
            |this, folder, window, cx| {
                this.hidden = false;
                this.sidebar
                    .update(cx, |sidebar, cx| sidebar.rename_folder(folder, window, cx));
            },
        );
    }

    /// Saves the open note, runs `job` off the UI thread, and gives its result to `done`.
    /// An error shows in a quiet bar. The sidebar reads the notes again after each action.
    fn run<T: Send + 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        job: impl FnOnce(&Core) -> holm_core::Result<T> + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let flush = self.pane.update(cx, NotePane::flush);
        let core = self.core.clone();
        self.action = Some(cx.spawn_in(window, async move |this, cx| {
            flush.await;
            let result = cx.background_spawn(async move { job(&core) }).await;
            this.update_in(cx, |this, window, cx| {
                this.action = None;
                match result {
                    Ok(value) => done(this, value, window, cx),
                    Err(error) => {
                        let error = AppError::from(error);
                        tracing::warn!(error = %crate::chain(&error), "a file action failed");
                        this.error = Some(error.message());
                    }
                }
                this.sidebar.update(cx, Sidebar::refresh);
                cx.notify();
            })
            .ok();
        }));
    }

    fn focus_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.pane.focus_handle(cx), cx);
    }

    fn focus_sidebar(&self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.hidden {
            window.focus(&self.sidebar.focus_handle(cx), cx);
        }
    }

    /// Ctrl+Shift+L: show the sidebar and focus it, or hide it when it has the focus.
    fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focused = self.sidebar.focus_handle(cx).contains_focused(window, cx);
        if self.hidden {
            self.hidden = false;
            self.focus_sidebar(window, cx);
        } else if focused {
            self.hidden = true;
            self.focus_editor(window, cx);
        } else {
            self.focus_sidebar(window, cx);
        }
        self.keep_state(cx);
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_some() || self.picker.is_some() {
            return;
        }
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if !modifiers.control || modifiers.alt || modifiers.platform {
            return;
        }
        match (modifiers.shift, keystroke.key.as_str()) {
            (true, "l") => self.toggle_sidebar(window, cx),
            (false, "n") => self.new_note(window, cx),
            (true, "n") => self.new_folder(window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.resizing {
            let room = window.viewport_size().width - px(MIN_EDITOR);
            let max = room.min(px(MAX_WIDTH)).max(px(MIN_WIDTH));
            self.width = event.position.x.clamp(px(MIN_WIDTH), max);
            cx.notify();
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.resizing {
            self.resizing = false;
            self.keep_state(cx);
        }
    }

    /// Keeps the sidebar state and the open note in the config.
    fn keep_state(&mut self, cx: &mut Context<Self>) {
        let mut settings = self.config.read(cx).settings().clone();
        settings.sidebar = SidebarSettings {
            width: whole(self.width),
            hidden: self.hidden,
            view: self.sidebar.read(cx).view().clone(),
        };
        settings.last_note = Some(self.pane.read(cx).path().to_owned());
        if &settings != self.config.read(cx).settings() {
            self.save_settings(settings, cx);
        }
    }

    fn save_settings(&mut self, settings: crate::config::Settings, cx: &mut Context<Self>) {
        if let Err(error) = self.config.update(cx, |config, _| config.save(settings)) {
            tracing::warn!(error = %crate::chain(&error), "cannot save the settings");
            self.error = Some(error.message());
            cx.notify();
        }
    }
}

#[cfg(test)]
impl NotesView {
    pub(crate) fn pane(&self) -> &Entity<NotePane> {
        &self.pane
    }

    pub(crate) fn sidebar(&self) -> &Entity<Sidebar> {
        &self.sidebar
    }

    pub(crate) fn is_hidden(&self) -> bool {
        self.hidden
    }

    pub(crate) fn dialog(&self) -> Option<&Entity<DeleteDialog>> {
        self.dialog.as_ref()
    }
}

impl Render for NotesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let error = self.error.map(|message| {
            Bar::new("action-error", message).button(
                bar::button("action-error-close", text::CLOSE).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.error = None;
                        cx.notify();
                    },
                )),
            )
        });
        // The drag strip lies over the sidebar line, so the rows reach the line.
        let sidebar = (!self.hidden).then(|| {
            div()
                .relative()
                .h_full()
                .flex_none()
                .w(self.width)
                .border_r_1()
                .border_color(theme.sidebar_border)
                .child(self.sidebar.clone())
                .child(
                    div()
                        .id("sidebar-edge")
                        .absolute()
                        .top_0()
                        .right(px(-3.))
                        .w(px(6.))
                        .h_full()
                        .cursor(CursorStyle::ResizeLeftRight)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.resizing = true;
                                cx.stop_propagation();
                            }),
                        ),
                )
        });
        v_flex()
            .id("notes")
            .size_full()
            .relative()
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .children(error)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .children(sidebar)
                    .child(div().flex_1().h_full().min_w_0().child(self.pane.clone())),
            )
            .children(self.dialog.clone())
            .children(self.picker.clone())
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the width stays between MIN_WIDTH and MAX_WIDTH"
)]
fn whole(width: Pixels) -> u16 {
    f32::from(width).round() as u16
}

/// The note to open first: the last open note when it still exists, else the last changed
/// note. With no notes, it makes one.
pub(crate) fn first_note(core: &Core, last: Option<&Path>) -> holm_core::Result<Note> {
    if let Some(note) = last.and_then(|last| core.read(last).ok()) {
        return Ok(note);
    }
    match core.notes(Order::LastChange)?.first() {
        Some(info) => core.read(&info.path),
        None => core.create(""),
    }
}
