//! The open notes folder: the sidebar, the editor pane, and the file actions between them.
//!
//! Each file action goes in the write line of the notes folder, after the unsaved text of the
//! pane. The sidebar shows each new snapshot by itself.
//!
//! Holm makes no note by itself. When the open note is gone, the pane opens its neighbor in
//! the sidebar list. With no note left, it shows the empty pane.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AppContext as _, Context, CursorStyle, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, MouseMoveEvent, MouseUpEvent,
    ParentElement, Pixels, Render, Styled, Subscription, Window, div, px,
};
use holm_core::model::{Note, Order};
use holm_core::{Core, Snapshot};

use crate::config::{Config, SidebarSettings};
use crate::dialog::{Ask, Dialog, DialogEvent};
use crate::error::AppError;
use crate::link_picker::{LinkPicker, LinkPickerEvent, LinkStart};
use crate::message_line::MessageLine;
use crate::name_field::NameDone;
use crate::notes_folder::{NotesFolder, Step, window_step};
use crate::pane::{Leave, NotePane, PaneEvent};
use crate::picker::{FolderPicker, PickerEvent};
use crate::quick_open::{QuickOpen, QuickOpenEvent};
use crate::sidebar::{Item, Named, Sidebar, SidebarEvent, SidebarWrite, Target};
use crate::tags::RenameTag;

const MIN_WIDTH: f32 = 160.;
const MAX_WIDTH: f32 = 600.;
/// The least width the drag leaves for the editor.
const MIN_EDITOR: f32 = 360.;

pub(crate) struct NotesView {
    notes_folder: Entity<NotesFolder>,
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
    /// The end of the tag rename in the name field, while its dialog shows.
    tag_rename: Option<NameDone>,
    line: Entity<MessageLine>,
    subscriptions: Vec<Subscription>,
}

impl NotesView {
    pub(crate) fn new(
        core: Core,
        root: PathBuf,
        note: Option<Note>,
        config: Entity<Config>,
        line: Entity<MessageLine>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = config.read(cx).settings().clone();
        let current = note.as_ref().map(|note| note.info.path.clone());
        let notes_folder = cx.new(|cx| NotesFolder::new(core, cx));
        let pane =
            cx.new(|cx| NotePane::new(notes_folder.clone(), line.clone(), root, note, window, cx));
        let this = cx.weak_entity();
        let write: SidebarWrite = Rc::new(move |named, name, done, window, cx| {
            this.update(cx, |this, cx| {
                this.name_write(named, name, done, window, cx);
            })
            .ok();
        });
        let sidebar = cx.new(|cx| {
            Sidebar::new(
                notes_folder.clone(),
                settings.note_order,
                settings.sidebar.view.clone(),
                current,
                write,
                window,
                cx,
            )
        });
        let subscriptions = vec![
            cx.subscribe_in(&sidebar, window, Self::sidebar_event),
            cx.subscribe_in(&pane, window, Self::pane_event),
        ];
        let mut this = Self {
            notes_folder,
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
            tag_rename: None,
            line,
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
                let neighbor = self.sidebar.read(cx).neighbor(path);
                let order = self.config.read(cx).settings().note_order;
                let next = next_note(self.notes_folder.read(cx).snapshot(), neighbor, order);
                self.show_next(next, window, cx);
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
        self.hidden = false;
        self.keep_state(cx);
        self.focus_sidebar(window, cx);
        cx.notify();
        // A new tag may still wait in the write line.
        self.write(
            window,
            cx,
            |notes_folder, done, cx| notes_folder.wait(done, cx),
            move |this, (), window, cx| {
                this.sidebar
                    .update(cx, |sidebar, cx| sidebar.rename(item, window, cx));
            },
        );
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
            SidebarEvent::RenameNote => {
                self.pane.update(cx, |pane, cx| pane.edit_title(window, cx));
            }
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
            SidebarEvent::DeleteTag(tag) => {
                let tag = tag.clone();
                self.write(
                    window,
                    cx,
                    {
                        let tag = tag.clone();
                        |notes_folder, done, cx| notes_folder.tag_delete_count(tag, done, cx)
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
            Target::Note(path) => self.write(
                window,
                cx,
                |notes_folder, done, cx| notes_folder.move_note(path, folder, done, cx),
                |this, _, _, cx| this.keep_state(cx),
            ),
            Target::Folder(path) => self.write(
                window,
                cx,
                |notes_folder, done, cx| notes_folder.move_folder(path, folder, done, cx),
                |this, to, window, cx| {
                    this.show_folder(to, window, cx);
                    this.keep_state(cx);
                },
            ),
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
        let done = window_step(window, cx, Self::show_link_picker);
        self.pane.update(cx, |pane, cx| pane.start_link(done, cx));
    }

    fn show_link_picker(&mut self, start: LinkStart, window: &mut Window, cx: &mut Context<Self>) {
        let notes_folder = self.notes_folder.clone();
        self.before_popup = window.focused(cx);
        let picker = cx.new(|cx| LinkPicker::new(notes_folder, start, window, cx));
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
                let done = Self::then(window, cx, |this, (), window, cx| {
                    this.focus_editor(window, cx);
                });
                self.pane.update(cx, |pane, cx| pane.link(choice, done, cx));
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
        let counted = target.clone();
        self.write(
            window,
            cx,
            |notes_folder, done, cx| match counted {
                Target::Note(path) => notes_folder.note_delete_count(path, done, cx),
                Target::Folder(path) => notes_folder.folder_delete_count(path, done, cx),
            },
            move |this, count, window, cx| {
                let name = match &target {
                    Target::Note(path) => this
                        .notes_folder
                        .read(cx)
                        .snapshot()
                        .note(path)
                        .map(|note| note.title.clone())
                        .unwrap_or_default(),
                    Target::Folder(path) => path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                };
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
        let (ask, do_not_ask) = match event.clone() {
            DialogEvent::Confirm { ask, do_not_ask } => (ask, do_not_ask),
            DialogEvent::Cancel(ask) => {
                if matches!(ask, Ask::RenameTag { .. })
                    && let Some(done) = self.tag_rename.take()
                {
                    done(Ok(()), cx);
                }
                return;
            }
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
                let done = self.tag_rename.take();
                self.pane.update(cx, NotePane::save);
                self.notes_folder.update(cx, |notes_folder, cx| {
                    notes_folder.rename_tag(
                        from,
                        to,
                        move |renamed, cx| {
                            if let Some(done) = done {
                                done(renamed.map(drop), cx);
                            }
                        },
                        cx,
                    );
                });
            }
            Ask::DeleteTag { tag, .. } => {
                self.write(
                    window,
                    cx,
                    |notes_folder, done, cx| notes_folder.delete_tag(tag, done, cx),
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
        let neighbor = open.and_then(|open| self.sidebar.read(cx).neighbor(&open));
        let order = self.config.read(cx).settings().note_order;
        self.write(
            window,
            cx,
            |notes_folder, done, cx| match target {
                Target::Note(path) => notes_folder.delete_note(path, done, cx),
                Target::Folder(path) => notes_folder.delete_folder(path, done, cx),
            },
            move |this, (), window, cx| {
                if open_gone {
                    let next = next_note(this.notes_folder.read(cx).snapshot(), neighbor, order);
                    this.show_next(next, window, cx);
                }
            },
        );
    }

    /// Ctrl+N: a new note in the folder of the view, with its tag.
    fn new_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (folder, tag) = self.sidebar.read(cx).new_note_place();
        self.pane
            .update(cx, |pane, cx| pane.new_note(folder, tag, window, cx));
    }

    /// Ctrl+Shift+N: an empty name field for a new folder in the folder of the view.
    fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent = match self.sidebar.read(cx).view().item() {
            Item::Folder(folder) => folder.clone(),
            Item::All | Item::Tag(_) => PathBuf::new(),
        };
        self.hidden = false;
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.new_folder(parent, window, cx));
        cx.notify();
    }

    /// Puts the unsaved text of the pane in the write line, and then the write of `start`.
    /// `done` gets its value in the update where the write ends. An error shows in the message line.
    fn write<T: 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        start: impl FnOnce(&mut NotesFolder, Step<holm_core::Result<T>>, &mut Context<NotesFolder>),
        done: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    ) {
        self.pane.update(cx, NotePane::save);
        let done = Self::then(window, cx, done);
        self.notes_folder
            .update(cx, |notes_folder, cx| start(notes_folder, done, cx));
    }

    /// Writes `name` from the name field of `named` in the sidebar, after the unsaved text of
    /// the pane. A tag rename first asks with the dialog.
    fn name_write(
        &mut self,
        named: Named,
        name: String,
        done: NameDone,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pane.update(cx, NotePane::save);
        match named {
            Named::Item(Item::Folder(folder)) => {
                let kept = window_step(
                    window,
                    cx,
                    |this: &mut Self, renamed: holm_core::Result<PathBuf>, _, cx| {
                        if renamed.is_ok() {
                            this.keep_state(cx);
                        }
                        done(renamed.map(drop), cx);
                    },
                );
                self.notes_folder.update(cx, |notes_folder, cx| {
                    notes_folder.rename_folder(folder, name, kept, cx);
                });
            }
            Named::NewFolder(parent) => {
                let shown = window_step(
                    window,
                    cx,
                    |this: &mut Self, created: holm_core::Result<PathBuf>, window, cx| {
                        let shown = created.map(|folder| {
                            this.show_folder(folder, window, cx);
                            this.keep_state(cx);
                        });
                        done(shown, cx);
                    },
                );
                self.notes_folder.update(cx, |notes_folder, cx| {
                    notes_folder.create_folder(parent, name, shown, cx);
                });
            }
            Named::Item(Item::Tag(from)) if !name.is_empty() => {
                let (counted_from, counted_to) = (from.clone(), name.clone());
                let asked =
                    window_step(
                        window,
                        cx,
                        move |this: &mut Self, count, window, cx| match count {
                            Ok(notes) => {
                                this.tag_rename = Some(done);
                                this.ask(
                                    Ask::RenameTag {
                                        from,
                                        to: name,
                                        notes,
                                    },
                                    window,
                                    cx,
                                );
                            }
                            Err(error) => done(Err(error), cx),
                        },
                    );
                self.notes_folder.update(cx, |notes_folder, cx| {
                    notes_folder.tag_rename_count(counted_from, counted_to, asked, cx);
                });
            }
            Named::Item(Item::Tag(_) | Item::All) => done(Ok(()), cx),
        }
    }

    /// A step for the result of a file action. An error shows in the message line.
    fn then<T: 'static>(
        window: &Window,
        cx: &Context<Self>,
        done: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    ) -> Step<holm_core::Result<T>> {
        window_step(window, cx, move |this, result, window, cx| {
            match result {
                Ok(value) => done(this, value, window, cx),
                Err(error) => this.fail(error, cx),
            }
            cx.notify();
        })
    }

    /// Shows the error of a file action as an answer.
    fn fail(&mut self, error: holm_core::Error, cx: &mut Context<Self>) {
        let error = AppError::from(error);
        tracing::warn!(error = %crate::chain(&error), "a file action failed");
        self.line
            .update(cx, |line, cx| line.show(error.message(), cx));
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
        let alt_only =
            modifiers.alt && !(modifiers.control || modifiers.shift || modifiers.platform);
        if alt_only && keystroke.key == "d" {
            self.pane.update(cx, |pane, cx| pane.open_state(window, cx));
            cx.stop_propagation();
            return;
        }
        if !modifiers.modified() && keystroke.key == "f2" && self.editor_focused(window, cx) {
            self.pane.update(cx, |pane, cx| pane.edit_title(window, cx));
            cx.stop_propagation();
            return;
        }
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
            self.line
                .update(cx, |line, cx| line.show(error.message(), cx));
        }
    }
}

impl Render for NotesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
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

/// The note to open after the open one is gone: `neighbor` when it still exists, else the
/// first note in `order`. With no notes, it is `None`.
fn next_note(snapshot: &Snapshot, neighbor: Option<PathBuf>, order: Order) -> Option<PathBuf> {
    if let Some(next) = neighbor.filter(|next| snapshot.note(next).is_some()) {
        return Some(next);
    }
    snapshot
        .notes(order)
        .into_iter()
        .next()
        .map(|note| note.path)
}
