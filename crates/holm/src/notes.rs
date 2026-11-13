//! The open notes folder: the sidebar, the editor pane, and the file actions between them.
//!
//! Each file action runs through the pane, which saves the open note first. The sidebar
//! reads the notes again by itself after each change.
//!
//! Holm makes no note by itself. When the open note is gone, the pane opens its neighbor in
//! the sidebar list. With no note left, it shows the empty pane.

use std::path::{Path, PathBuf};

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AppContext as _, Context, CursorStyle, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, MouseMoveEvent, MouseUpEvent,
    ParentElement, Pixels, Render, Styled, Subscription, Task, Window, div, px,
};
use holm_core::Core;
use holm_core::model::{Note, Order};

use crate::bar::{self, Bar};
use crate::config::{Config, SidebarSettings};
use crate::dialog::{Ask, Dialog, DialogEvent};
use crate::error::AppError;
use crate::link_picker::{LinkPicker, LinkPickerEvent, LinkStart};
use crate::pane::{Leave, NotePane, PaneEvent};
use crate::picker::{FolderPicker, PickerEvent};
use crate::quick_open::{QuickOpen, QuickOpenEvent};
use crate::sidebar::{Item, Sidebar, SidebarEvent, Target};
use crate::tags::RenameTag;
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
    dialog: Option<Entity<Dialog>>,
    picker: Option<Entity<FolderPicker>>,
    quick_open: Option<Entity<QuickOpen>>,
    link_picker: Option<Entity<LinkPicker>>,
    /// The part that had the focus before quick open or the link picker. It gets the focus
    /// back on `Esc`.
    before_popup: Option<FocusHandle>,
    error: Option<&'static str>,
    action: Option<Task<()>>,
    subscriptions: Vec<Subscription>,
}

impl NotesView {
    pub(crate) fn new(
        core: Core,
        notes_folder: PathBuf,
        note: Option<Note>,
        config: Entity<Config>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = config.read(cx).settings().clone();
        let current = note.as_ref().map(|note| note.info.path.clone());
        let pane = cx.new(|cx| NotePane::new(core.clone(), notes_folder, note, window, cx));
        let sidebar = cx.new(|cx| {
            Sidebar::new(
                core.clone(),
                settings.note_order,
                settings.sidebar.view.clone(),
                current,
                window,
                cx,
            )
        });
        let subscriptions = vec![
            cx.subscribe_in(&sidebar, window, Self::sidebar_event),
            cx.subscribe_in(&pane, window, Self::pane_event),
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
            quick_open: None,
            link_picker: None,
            before_popup: None,
            error: None,
            action: None,
            subscriptions,
        };
        this.keep_state(cx);
        this.focus_home(window, cx);
        this
    }

    fn pane_event(
        &mut self,
        _: &Entity<NotePane>,
        event: &PaneEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            PaneEvent::Opened(path) => {
                let path = path.clone();
                self.sidebar
                    .update(cx, |sidebar, cx| sidebar.set_current(Some(path), cx));
                self.keep_state(cx);
            }
            PaneEvent::Closed => {
                self.sidebar
                    .update(cx, |sidebar, cx| sidebar.set_current(None, cx));
                self.keep_state(cx);
                // A popup that closes later must not give the focus back to the editor.
                self.before_popup = None;
                let in_pane = self.pane.focus_handle(cx).contains_focused(window, cx);
                if in_pane || window.focused(cx).is_none() {
                    self.focus_home(window, cx);
                }
            }
            PaneEvent::Gone(path) => {
                // A read changes no file, so it does not wait for the pane to save.
                let next = self.sidebar.read(cx).neighbor(path);
                let order = self.config.read(cx).settings().note_order;
                let core = self.core.clone();
                let job = cx.background_spawn(async move {
                    Ok::<_, holm_core::Error>(next_note(&core, next, order))
                });
                self.finish(job, Self::show_next, window, cx);
            }
        }
    }

    /// Starts the rename of a tag in the sidebar, for all notes.
    fn rename_tag(
        &mut self,
        RenameTag(tag): &RenameTag,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let item = Item::Tag(tag.clone());
        // A hidden sidebar still has its tree, so the field can start before it shows.
        if self
            .sidebar
            .update(cx, |sidebar, cx| sidebar.rename(item, window, cx))
        {
            self.hidden = false;
            self.keep_state(cx);
            cx.notify();
        }
    }

    /// Opens `next`, or shows the empty pane.
    fn show_next(&mut self, next: Option<PathBuf>, _: &mut Window, cx: &mut Context<Self>) {
        self.pane.update(cx, |pane, cx| match next {
            Some(next) => pane.open(next, Leave::Drop, cx),
            None => pane.close(cx),
        });
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
            SidebarEvent::FocusEditor => self.focus_home(window, cx),
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
                let from = Item::Folder(folder.clone());
                let to = Item::Folder(folder.with_file_name(&name));
                let job = self.pane.update(cx, |pane, cx| {
                    pane.run_move(
                        folder.clone(),
                        move |core| core.rename_folder(&folder, &name).result,
                        cx,
                    )
                });
                self.finish_rename(from, to, job, |to| Item::Folder(to.clone()), window, cx);
            }
            SidebarEvent::RenameTag { from, to } => {
                let (from, to) = (from.clone(), to.clone());
                self.run(
                    window,
                    cx,
                    {
                        let (from, to) = (from.clone(), to.clone());
                        move |core| core.tag_rename_count(&from, &to)
                    },
                    move |this, notes, window, cx| {
                        this.ask(Ask::RenameTag { from, to, notes }, window, cx);
                    },
                );
            }
            SidebarEvent::DeleteTag(tag) => {
                let tag = tag.clone();
                self.run(
                    window,
                    cx,
                    {
                        let tag = tag.clone();
                        move |core| core.tag_delete_count(&tag)
                    },
                    move |this, notes, window, cx| {
                        this.ask(Ask::DeleteTag { tag, notes }, window, cx);
                    },
                );
            }
            SidebarEvent::ViewChanged => self.keep_state(cx),
        }
    }

    /// Shows the dialog for `ask`, or deletes at once after "Do not show again".
    fn ask(&mut self, ask: Ask, window: &mut Window, cx: &mut Context<Self>) {
        if ask.can_skip() && !self.config.read(cx).settings().ask_before_delete {
            self.confirmed(ask, window, cx);
            return;
        }
        let dialog = cx.new(|cx| Dialog::new(ask, window, cx));
        self.subscriptions
            .push(cx.subscribe_in(&dialog, window, Self::dialog_event));
        self.dialog = Some(dialog);
        cx.notify();
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
            Target::Note(path) => {
                let job = self.pane.update(cx, |pane, cx| {
                    pane.run_move(
                        path.clone(),
                        move |core| core.move_note(&path, &folder).result.map(|info| info.path),
                        cx,
                    )
                });
                self.finish(job, |_, _, _, _| {}, window, cx);
            }
            Target::Folder(path) => {
                let job = self.pane.update(cx, |pane, cx| {
                    pane.run_move(
                        path.clone(),
                        move |core| core.move_folder(&path, &folder).result,
                        cx,
                    )
                });
                self.finish(job, Self::show_folder, window, cx);
            }
        }
    }

    fn open_quick_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let notes = self.sidebar.read(cx).notes().to_vec();
        self.before_popup = window.focused(cx);
        let quick_open = cx.new(|cx| QuickOpen::new(notes, window, cx));
        self.subscriptions
            .push(cx.subscribe_in(&quick_open, window, Self::quick_open_event));
        self.quick_open = Some(quick_open);
        cx.notify();
    }

    /// Ctrl+K: opens the link picker.
    fn open_link_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(job) = self.pane.update(cx, NotePane::start_link) {
            self.finish(job, Self::show_link_picker, window, cx);
        }
    }

    fn show_link_picker(&mut self, start: LinkStart, window: &mut Window, cx: &mut Context<Self>) {
        let core = self.core.clone();
        self.before_popup = window.focused(cx);
        let picker = cx.new(|cx| LinkPicker::new(core, start, window, cx));
        self.subscriptions
            .push(cx.subscribe_in(&picker, window, Self::link_picker_event));
        self.link_picker = Some(picker);
        cx.notify();
    }

    fn link_picker_event(
        &mut self,
        _: &Entity<LinkPicker>,
        event: &LinkPickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.link_picker = None;
        let before = self.before_popup.take();
        match event {
            LinkPickerEvent::Pick(choice) => {
                self.focus_editor(window, cx);
                let choice = choice.clone();
                let job = self.pane.update(cx, |pane, cx| pane.link(choice, cx));
                self.finish(
                    job,
                    |this, (), window, cx| this.focus_editor(window, cx),
                    window,
                    cx,
                );
            }
            LinkPickerEvent::Close => self.focus_before(before, window, cx),
        }
        cx.notify();
    }

    fn quick_open_event(
        &mut self,
        _: &Entity<QuickOpen>,
        event: &QuickOpenEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.quick_open = None;
        let before = self.before_popup.take();
        match event {
            QuickOpenEvent::Open(path) => {
                let path = path.clone();
                self.pane
                    .update(cx, |pane, cx| pane.open(path, Leave::Keep, cx));
                self.focus_editor(window, cx);
            }
            QuickOpenEvent::Close => self.focus_before(before, window, cx),
        }
        cx.notify();
    }

    /// Gives the focus back to `before`, or to the editor.
    fn focus_before(
        &self,
        before: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match before {
            Some(before) => window.focus(&before, cx),
            None => self.focus_home(window, cx),
        }
    }

    fn show_folder(&mut self, folder: PathBuf, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.show_in_tree(Item::Folder(folder), cx);
        });
    }

    fn ask_delete(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        self.run(
            window,
            cx,
            {
                let target = target.clone();
                move |core| match &target {
                    Target::Note(path) => {
                        let title = core.read(path)?.info.title;
                        Ok((title, core.note_delete_count(path)?))
                    }
                    Target::Folder(path) => {
                        let name = path.file_name().unwrap_or_default();
                        Ok((
                            name.to_string_lossy().into_owned(),
                            core.folder_delete_count(path)?,
                        ))
                    }
                }
            },
            move |this, (name, count), window, cx| {
                this.ask(
                    Ask::Delete {
                        target,
                        name,
                        count,
                    },
                    window,
                    cx,
                );
            },
        );
    }

    fn dialog_event(
        &mut self,
        _: &Entity<Dialog>,
        event: &DialogEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dialog = None;
        self.focus_sidebar(window, cx);
        cx.notify();
        let DialogEvent::Confirm { ask, do_not_ask } = event.clone() else {
            return;
        };
        if do_not_ask {
            let mut settings = self.config.read(cx).settings().clone();
            settings.ask_before_delete = false;
            self.save_settings(settings, cx);
        }
        self.confirmed(ask, window, cx);
    }

    fn confirmed(&mut self, ask: Ask, window: &mut Window, cx: &mut Context<Self>) {
        match ask {
            Ask::Delete { target, .. } => self.delete(target, window, cx),
            Ask::RenameTag { from, to, .. } => {
                let shown = holm_markdown::normalize_tag(&to);
                let item = Item::Tag(from.clone());
                let job = self.pane.update(cx, |pane, cx| {
                    pane.run(move |core| core.rename_tag(&from, &to).result, cx)
                });
                match shown {
                    Some(tag) => {
                        let tag = Item::Tag(tag);
                        self.finish_rename(item, tag.clone(), job, move |_| tag, window, cx);
                    }
                    None => self.finish(job, |_, _, _, _| {}, window, cx),
                }
            }
            Ask::DeleteTag { tag, .. } => {
                self.run(
                    window,
                    cx,
                    move |core| core.delete_tag(&tag).result,
                    |_, _, _, _| {},
                );
            }
        }
    }

    fn delete(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        let open = self.pane.read(cx).path().map(Path::to_owned);
        let open_gone = open.as_ref().is_some_and(|open| match &target {
            Target::Note(path) => path == open,
            Target::Folder(folder) => open.starts_with(folder),
        });
        let next = open.and_then(|open| self.sidebar.read(cx).neighbor(&open));
        let order = self.config.read(cx).settings().note_order;
        self.run(
            window,
            cx,
            move |core| {
                match &target {
                    Target::Note(path) => core.delete_note(path).result?,
                    Target::Folder(path) => core.delete_folder(path).result?,
                }
                if !open_gone {
                    return Ok(None);
                }
                Ok(Some(next_note(core, next, order)))
            },
            |this, next, window, cx| {
                if let Some(next) = next {
                    this.show_next(next, window, cx);
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
                let note = core.create(&folder).result?;
                match tag {
                    Some(tag) => core
                        .set_tags(&note.info.path, &[tag])
                        .result
                        .map(|info| info.path),
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
            move |core| core.create_folder(&parent, text::NEW_FOLDER).result,
            |this, folder, window, cx| {
                this.hidden = false;
                this.sidebar.update(cx, |sidebar, cx| {
                    sidebar.rename(Item::Folder(folder), window, cx);
                });
            },
        );
    }

    /// Runs `job` through the pane, which saves the open note first, and gives its result to
    /// `done`.
    fn run<T: Send + 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        job: impl FnOnce(&Core) -> holm_core::Result<T> + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let job = self.pane.update(cx, |pane, cx| pane.run(job, cx));
        self.finish(job, done, window, cx);
    }

    /// Shows the rename of `from` to `to` in the tree at once, and waits for `job`. `item`
    /// gives the item on disk from the result of `job`.
    fn finish_rename<T: 'static>(
        &mut self,
        from: Item,
        to: Item,
        job: Task<holm_core::Result<T>>,
        item: impl FnOnce(&T) -> Item + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.show_rename(from, to, cx));
        let sidebar = self.sidebar.downgrade();
        let job = cx.spawn(async move |_, cx| {
            let result = job.await;
            let to = result.as_ref().ok().map(item);
            sidebar
                .update(cx, |sidebar, cx| sidebar.end_rename(to, cx))
                .ok();
            result
        });
        self.finish(job, |_, _, _, _| {}, window, cx);
    }

    /// Waits for a file action and gives its result to `done`. An error shows in a quiet bar.
    fn finish<T: 'static>(
        &mut self,
        job: Task<holm_core::Result<T>>,
        done: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.action = Some(cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
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
                cx.notify();
            })
            .ok();
        }));
    }

    fn focus_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.pane.focus_handle(cx), cx);
    }

    fn editor_focused(&self, window: &Window, cx: &Context<Self>) -> bool {
        self.pane.focus_handle(cx).is_focused(window)
    }

    /// Gives the focus to the editor. On the empty pane, the sidebar gets it when it shows.
    fn focus_home(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pane.read(cx).path().is_none() && !self.hidden {
            self.focus_sidebar(window, cx);
        } else {
            self.focus_editor(window, cx);
        }
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

    /// Ctrl+Shift+F: show the sidebar and give its search field the focus.
    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.hidden = false;
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.focus_search(window, cx));
        self.keep_state(cx);
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_some()
            || self.picker.is_some()
            || self.quick_open.is_some()
            || self.link_picker.is_some()
        {
            return;
        }
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if !modifiers.control || modifiers.alt || modifiers.platform {
            return;
        }
        let in_editor = self.editor_focused(window, cx);
        match (modifiers.shift, keystroke.key.as_str()) {
            (true, "l") => self.toggle_sidebar(window, cx),
            (true, "f") => self.focus_search(window, cx),
            (true, "o") => self.open_quick_open(window, cx),
            (true, "p") => self.pane.update(cx, NotePane::toggle_pin),
            (true, "t") => self.pane.update(cx, |pane, cx| pane.focus_tags(window, cx)),
            (true, "b") => self
                .pane
                .update(cx, |pane, cx| pane.show_backlinks(window, cx)),
            (false, "n") => self.new_note(window, cx),
            (true, "n") => self.new_folder(window, cx),
            (false, "k") if in_editor => self.open_link_picker(window, cx),
            (false, "o") if in_editor => {
                self.pane.update(cx, NotePane::link_files);
            }
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
        settings.last_note = self.pane.read(cx).path().map(Path::to_owned);
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
            .on_action(cx.listener(Self::rename_tag))
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
            .children(self.quick_open.clone())
            .children(self.link_picker.clone())
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
/// note. With no notes, it is `None`.
pub(crate) fn first_note(core: &Core, last: Option<&Path>) -> holm_core::Result<Option<Note>> {
    if let Some(note) = last.and_then(|last| core.read(last).ok()) {
        return Ok(Some(note));
    }
    match core.snapshot().notes(Order::LastChange).first() {
        Some(info) => core.read(&info.path).map(Some),
        None => Ok(None),
    }
}

/// The note to open after the open one is gone: `neighbor` when it still exists, else the
/// first note in `order`. With no notes, it is `None`.
fn next_note(core: &Core, neighbor: Option<PathBuf>, order: Order) -> Option<PathBuf> {
    if let Some(next) = neighbor.filter(|next| core.read(next).is_ok()) {
        return Some(next);
    }
    core.snapshot()
        .notes(order)
        .into_iter()
        .next()
        .map(|note| note.path)
}
