//! The notes view: the sidebar, the editor pane, and the file actions between them.
//!
//! Each file action goes in the write line of the core door. The write line writes the
//! unsaved text of the pane first. The sidebar shows each new snapshot by itself.
//!
//! Every change that counts notes or asks the user runs here. The sidebar sends an event for
//! it. A name field of the sidebar writes by itself, and the field of a tag writes through
//! this view.
//!
//! Holm makes no note by itself. When the open note is gone, the pane opens its neighbor in
//! the sidebar list. With no note left, it shows the empty pane.

use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, CursorStyle, Entity, FocusHandle, Focusable as _, InteractiveElement as _, IntoElement,
    MouseButton, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Render, Styled, Subscription, Window, div, px,
};
use holm_core::model::Note;
use holm_core::{FolderPath, NotePath, TagName};

use crate::app_menu::{AppMenu, Picked, Spot};
use crate::config::{Config, SidebarSettings};
use crate::core_door::{CoreDoor, Done, JobFailed, NotSaved, Then};
use crate::dialog::{self, Ask, Dialog};
use crate::error::AppError;
use crate::link_picker::{LinkPicker, LinkPickerEvent, LinkStart};
use crate::message_line::MessageLine;
use crate::pane::{Leave, NotePane, PaneEvent, Pick};
use crate::picker::{FolderPicker, PickerEvent};
use crate::quick_open::{QuickOpen, QuickOpenEvent};
use crate::settings::{self, SettingsDialog};
use crate::shortcut_list::{self, ShortcutList};
use crate::shortcuts::{Command, Place, Run};
use crate::sidebar::{Item, Sidebar, SidebarEvent, TagRename, Target};
use crate::tags::RenameTag;
use crate::text;
use crate::window_step::window_step;

const MIN_WIDTH: f32 = 160.;
const MAX_WIDTH: f32 = 600.;
/// The least width the drag leaves for the editor.
const MIN_EDITOR: f32 = 360.;

pub(crate) struct NotesView {
    core_door: Entity<CoreDoor>,
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
    settings: Option<Entity<SettingsDialog>>,
    shortcut_list: Option<Entity<ShortcutList>>,
    app_menu: Entity<AppMenu>,
    /// The part that had the focus before quick open, the link picker, the settings or the
    /// shortcut list. It gets the focus back on `Esc`.
    before_popup: Option<FocusHandle>,
    message_line: Entity<MessageLine>,
    subscriptions: Vec<Subscription>,
}

impl NotesView {
    /// The view of `core_door` with `note` open.
    pub(crate) fn new(
        core_door: Entity<CoreDoor>,
        note: Option<Note>,
        config: Entity<Config>,
        message_line: Entity<MessageLine>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = config.read(cx).settings().clone();
        let pane = cx.new(|cx| NotePane::new(core_door.clone(), &config, message_line.clone(), note, window, cx));
        let sidebar = cx.new(|cx| {
            Sidebar::new(
                core_door.clone(),
                pane.clone(),
                config.clone(),
                &settings.sidebar.view,
                window,
                cx,
            )
        });
        let app_menu = cx.new(|cx| {
            let button = sidebar.read(cx).menu_button();
            AppMenu::new(pane.focus_handle(cx), button, window, cx)
        });
        let subscriptions = vec![
            // A command of the whole window, like Quit, goes to its owner as a key would.
            cx.subscribe_in(&app_menu, window, |this, _, Picked(command), window, cx| {
                if !this.run(*command, window, cx) {
                    window.dispatch_action(Box::new(Run(*command)), cx);
                }
            }),
            cx.observe(&core_door, |this, core_door, cx| {
                let writing = core_door.read(cx).writing();
                this.message_line.update(cx, |line, cx| line.set_writing(writing, cx));
            }),
            cx.subscribe(&core_door, |this, _, JobFailed(answer), cx| {
                this.message_line.update(cx, |line, cx| line.show(*answer, cx));
            }),
            cx.subscribe_in(&sidebar, window, Self::sidebar_event),
            cx.subscribe_in(&pane, window, Self::pane_event),
            // The focused part can leave the screen, like the tag field that closes when a
            // popup takes the focus. With no focus, no key reaches this view, so the focus
            // goes home.
            cx.on_focus_lost(window, |this, window, cx| this.focus_home(window, cx)),
        ];
        let mut this = Self {
            core_door,
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
            settings: None,
            shortcut_list: None,
            app_menu,
            before_popup: None,
            message_line,
            subscriptions,
        };
        this.save_place(cx);
        this.focus_home(window, cx);
        this
    }

    fn pane_event(&mut self, _: &Entity<NotePane>, event: &PaneEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            PaneEvent::Opening(_) => {}
            PaneEvent::Opened(_) => self.save_place(cx),
            PaneEvent::Closed => {
                self.save_place(cx);
                // A popup that closes later must not give the focus back to the editor.
                self.before_popup = None;
                let in_pane = self.pane.focus_handle(cx).contains_focused(window, cx);
                if in_pane || window.focused(cx).is_none() {
                    self.focus_home(window, cx);
                }
            }
            PaneEvent::Gone(path) => {
                let next = self.sidebar.read(cx).next_note(path, cx);
                self.show_next(next, window, cx);
            }
        }
    }

    /// `F2` on a tag line: shows the sidebar and starts the name field of the tag.
    fn rename_tag_from_line(&mut self, RenameTag(tag): &RenameTag, window: &mut Window, cx: &mut Context<Self>) {
        let tag = tag.clone();
        self.hidden = false;
        self.save_place(cx);
        self.focus_sidebar(window, cx);
        cx.notify();
        // A new tag may still wait in the write line.
        self.call(window, cx, CoreDoor::wait, move |this, (), window, cx| {
            this.rename_tag(tag, window, cx);
        });
    }

    /// Starts the name field of a tag in the sidebar, for a rename in all notes.
    fn rename_tag(&mut self, tag: TagName, window: &mut Window, cx: &mut Context<Self>) {
        let notes = cx.weak_entity();
        let rename: TagRename = Rc::new(move |from, to, done, window, cx| {
            notes
                .update(cx, |this, cx| {
                    this.ask_rename_tag(from, to, done, window, cx);
                })
                .ok();
        });
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.rename_tag(tag, rename, window, cx);
        });
    }

    /// The job of the tag name field: it counts the notes, asks, and renames.
    fn ask_rename_tag(
        &mut self,
        from: TagName,
        to: String,
        done: Then<Result<(), AppError>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if to.trim().is_empty() {
            done(Ok(()), cx);
            return;
        }
        let counted_from = from.clone();
        let counted = window_step(
            window,
            cx,
            move |this: &mut Self, count: Result<(TagName, usize), AppError>, window, cx| {
                let (to, notes) = match count {
                    Ok(count) => count,
                    Err(error) => return done(Err(error), cx),
                };
                let ask = Ask::RenameTag {
                    from: from.clone(),
                    to: to.clone(),
                    notes,
                };
                this.ask(ask, window, cx, move |this, confirmed, _, cx| {
                    if !confirmed {
                        done(Ok(()), cx);
                        return;
                    }
                    let renamed = Done::result(move |renamed, cx| done(renamed.map(drop), cx));
                    this.core_door.update(cx, |core_door, cx| {
                        core_door.rename_tag(from, to, renamed, cx);
                    });
                });
            },
        );
        self.core_door.update(cx, |core_door, cx| {
            core_door.tag_rename_count(counted_from, to, Done::result(counted), cx);
        });
    }

    /// Opens `next`, or shows the empty pane.
    fn show_next(&mut self, next: Option<NotePath>, _: &mut Window, cx: &mut Context<Self>) {
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
                self.pane.update(cx, |pane, cx| pane.open(path, Leave::Keep, cx));
            }
            SidebarEvent::FocusEditor => self.focus_home(window, cx),
            SidebarEvent::RenameNote => {
                self.pane.update(cx, |pane, cx| pane.edit_title(window, cx));
            }
            SidebarEvent::Delete(target) => self.ask_delete(target.clone(), window, cx),
            SidebarEvent::Move(target) => {
                let folders: Vec<_> = self.sidebar.read(cx).folders().cloned().collect();
                let target = target.clone();
                let picker = cx.new(|cx| FolderPicker::new(target, &folders, window, cx));
                self.subscriptions
                    .push(cx.subscribe_in(&picker, window, Self::picker_event));
                self.picker = Some(picker);
                cx.notify();
            }
            SidebarEvent::RenameTag(tag) => self.rename_tag(tag.clone(), window, cx),
            SidebarEvent::DeleteTag(tag) => {
                let tag = tag.clone();
                self.call(
                    window,
                    cx,
                    {
                        let tag = tag.clone();
                        |core_door, done, cx| core_door.tag_delete_count(tag, done, cx)
                    },
                    move |this, notes, window, cx| {
                        let ask = Ask::DeleteTag {
                            tag: tag.clone(),
                            notes,
                        };
                        this.ask(ask, window, cx, move |this, confirmed, window, cx| {
                            if confirmed {
                                this.call(
                                    window,
                                    cx,
                                    |core_door, done, cx| core_door.delete_tag(tag, done, cx),
                                    |_, _, _, _| {},
                                );
                            }
                        });
                    },
                );
            }
            SidebarEvent::ViewChanged => self.save_place(cx),
            SidebarEvent::AppMenu => self.toggle_app_menu(window, cx),
        }
    }

    /// Shows the dialog for `ask`, and gives `then` whether the user confirmed. After
    /// "Do not show again", a delete confirms with no dialog.
    fn ask(
        &mut self,
        ask: Ask,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, bool, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if ask.can_skip() && !self.config.read(cx).settings().ask_before_delete {
            then(self, true, window, cx);
            return;
        }
        let decided = window_step(
            window,
            cx,
            move |this: &mut Self, decision: dialog::Decision, window, cx| {
                this.dialog = None;
                this.focus_sidebar(window, cx);
                cx.notify();
                if decision.confirm && decision.do_not_ask {
                    this.config.update(cx, |config, cx| {
                        config.change(|settings| settings.ask_before_delete = false, cx);
                    });
                }
                then(this, decision.confirm, window, cx);
            },
        );
        self.dialog = Some(cx.new(|cx| Dialog::new(ask, decided, window, cx)));
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
            Target::Note(path) => self.call(
                window,
                cx,
                |core_door, done, cx| core_door.move_note(path, folder, done, cx),
                |this, _, _, cx| this.save_place(cx),
            ),
            Target::Folder(path) => self.call(
                window,
                cx,
                |core_door, done, cx| core_door.move_folder(path, folder, done, cx),
                |this, to, window, cx| {
                    this.show_folder(to, window, cx);
                    this.save_place(cx);
                },
            ),
        }
    }

    fn open_quick_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let notes = self.sidebar.read(cx).notes().cloned().collect();
        self.before_popup = window.focused(cx);
        let quick_open = cx.new(|cx| QuickOpen::new(notes, window, cx));
        self.subscriptions
            .push(cx.subscribe_in(&quick_open, window, Self::quick_open_event));
        self.quick_open = Some(quick_open);
        cx.notify();
    }

    /// Ctrl+Shift+S: opens the settings dialog.
    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.before_popup = window.focused(cx);
        let config = self.config.clone();
        let settings = cx.new(|cx| SettingsDialog::new(config, window, cx));
        self.subscriptions.push(
            cx.subscribe_in(&settings, window, |this, _, _: &settings::Close, window, cx| {
                this.settings = None;
                let before = this.before_popup.take();
                this.focus_before(before, window, cx);
                cx.notify();
            }),
        );
        self.settings = Some(settings);
        cx.notify();
    }

    /// Ctrl+?: opens the shortcut list.
    fn open_shortcut_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.before_popup = window.focused(cx);
        let list = cx.new(|cx| ShortcutList::new(window, cx));
        self.subscriptions.push(
            cx.subscribe_in(&list, window, |this, _, _: &shortcut_list::Close, window, cx| {
                this.shortcut_list = None;
                let before = this.before_popup.take();
                this.focus_before(before, window, cx);
                cx.notify();
            }),
        );
        self.shortcut_list = Some(list);
        cx.notify();
    }

    /// F10 or the button of the sidebar head: opens or closes the app menu. With the sidebar
    /// hidden, it opens at the window corner, and the sidebar stays hidden.
    fn toggle_app_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let spot = if self.hidden { Spot::Corner } else { Spot::Button };
        let note = self.pane.read(cx).path().is_some();
        self.app_menu.update(cx, |menu, cx| menu.toggle(spot, note, window, cx));
    }

    /// Ctrl+K: opens the link picker.
    fn open_link_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let done = window_step(window, cx, Self::show_link_picker);
        self.pane.update(cx, |pane, cx| pane.start_link(done, cx));
    }

    fn show_link_picker(&mut self, start: LinkStart, window: &mut Window, cx: &mut Context<Self>) {
        let core_door = self.core_door.clone();
        self.before_popup = window.focused(cx);
        let picker = cx.new(|cx| LinkPicker::new(core_door, start, window, cx));
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
                let done = window_step(window, cx, |this: &mut Self, (), window, cx| {
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
                self.pane.update(cx, |pane, cx| pane.open(path, Leave::Keep, cx));
                self.focus_editor(window, cx);
            }
            QuickOpenEvent::Close => self.focus_before(before, window, cx),
        }
        cx.notify();
    }

    /// Gives the focus back to `before`, or to the editor.
    fn focus_before(&self, before: Option<FocusHandle>, window: &mut Window, cx: &mut Context<Self>) {
        match before {
            Some(before) => window.focus(&before, cx),
            None => self.focus_home(window, cx),
        }
    }

    fn show_folder(&mut self, folder: FolderPath, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.show_in_tree(Item::Folder(folder), cx);
        });
    }

    fn ask_delete(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        let counted = target.clone();
        self.call(
            window,
            cx,
            |core_door, done, cx| match counted {
                Target::Note(path) => core_door.note_delete_count(path, done, cx),
                Target::Folder(path) => core_door.folder_delete_count(path, done, cx),
            },
            move |this, count, window, cx| {
                let name = match &target {
                    Target::Note(path) => this
                        .core_door
                        .read(cx)
                        .snapshot()
                        .note(path)
                        .map(|note| note.title.clone())
                        .unwrap_or_default(),
                    Target::Folder(path) => path.last_part().into_owned(),
                };
                this.ask(
                    Ask::Delete { name, count },
                    window,
                    cx,
                    move |this, confirmed, window, cx| {
                        if confirmed {
                            this.delete(target, window, cx);
                        }
                    },
                );
            },
        );
    }

    /// Deletes the target. The pane sends [`PaneEvent::Gone`] when the open note goes with it.
    fn delete(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        self.call(
            window,
            cx,
            |core_door, done, cx| match target {
                Target::Note(path) => core_door.delete_note(path, done, cx),
                Target::Folder(path) => core_door.delete_folder(path, done, cx),
            },
            |_, (), _, _| {},
        );
    }

    /// Ctrl+N: a new note in the folder of the sidebar view, with its tag.
    fn new_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (folder, tag) = self.sidebar.read(cx).new_note_place();
        self.pane.update(cx, |pane, cx| pane.new_note(folder, tag, window, cx));
    }

    /// Ctrl+Shift+N: an empty name field for a new folder in the folder of the sidebar view.
    fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.hidden = false;
        self.sidebar.update(cx, |sidebar, cx| sidebar.new_folder(window, cx));
        cx.notify();
    }

    /// A close of the window: gives whether the window may close now. See
    /// [`NotePane::may_close`].
    #[cfg(test)]
    pub(crate) fn pane(&self) -> &Entity<NotePane> {
        &self.pane
    }

    pub(crate) fn may_close(&self, window: &Window, cx: &mut Context<Self>) -> bool {
        self.pane.update(cx, |pane, cx| pane.may_close(window, cx))
    }

    /// Before the notes folder closes for another one: writes the unsaved text and waits for
    /// the jobs. `done` gets whether the text is on disk. A failed write keeps the text in the
    /// editor and shows "Not saved".
    pub(crate) fn leave(&self, done: Then<bool>, cx: &mut Context<Self>) {
        self.core_door.update(cx, |core_door, cx| {
            core_door.wait_saved(move |saved: Result<(), NotSaved>, cx| done(saved.is_ok(), cx), cx);
        });
    }

    /// Puts the job of `start` in the write line. `done` gets its value in the update where
    /// the job ends.
    fn call<T: 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        start: impl FnOnce(&mut CoreDoor, Done<T>, &mut Context<CoreDoor>),
        done: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let done = Done::value(window_step(window, cx, move |this, value, window, cx| {
            done(this, value, window, cx);
            cx.notify();
        }));
        self.core_door.update(cx, |core_door, cx| start(core_door, done, cx));
    }

    fn focus_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.pane.focus_handle(cx), cx);
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
        self.save_place(cx);
        cx.notify();
    }

    /// Ctrl+Shift+F: show the sidebar and give its search field the focus.
    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.hidden = false;
        self.sidebar.update(cx, |sidebar, cx| sidebar.focus_search(window, cx));
        self.save_place(cx);
        cx.notify();
    }

    /// Whether a dialog, a picker, the settings or the shortcut list is open. Then the keys of
    /// [`Place::Window`] do nothing.
    fn covered(&self) -> bool {
        self.dialog.is_some()
            || self.picker.is_some()
            || self.quick_open.is_some()
            || self.link_picker.is_some()
            || self.settings.is_some()
            || self.shortcut_list.is_some()
    }

    fn run_key(&mut self, Run(command): &Run, window: &mut Window, cx: &mut Context<Self>) {
        if self.covered() || !self.run(*command, window, cx) {
            cx.propagate();
        }
    }

    /// Runs `command` of a key or the app menu. Gives whether the view has an action for it.
    fn run(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match command {
            Command::EditTitle => self.pane.update(cx, |pane, cx| pane.edit_title(window, cx)),
            Command::ToggleSidebar => self.toggle_sidebar(window, cx),
            Command::Search => self.focus_search(window, cx),
            Command::QuickOpen => self.open_quick_open(window, cx),
            Command::Pin => self.pane.update(cx, NotePane::toggle_pin),
            Command::Tags => self.pane.update(cx, |pane, cx| pane.focus_tags(window, cx)),
            Command::Backlinks => self.pane.update(cx, |pane, cx| pane.show_backlinks(window, cx)),
            Command::NewNote => self.new_note(window, cx),
            Command::NewFolder => self.new_folder(window, cx),
            Command::LinkNote => self.open_link_picker(window, cx),
            Command::LinkFiles => {
                self.pane.update(cx, |pane, cx| pane.link_paths(Pick::Files, cx));
            }
            Command::LinkFolders => {
                self.pane.update(cx, |pane, cx| pane.link_paths(Pick::Folders, cx));
            }
            Command::Settings => self.open_settings(window, cx),
            Command::ShortcutList => self.open_shortcut_list(window, cx),
            Command::AppMenu => self.toggle_app_menu(window, cx),
            Command::ScaleUp => self.step_scale(true, cx),
            Command::ScaleDown => self.step_scale(false, cx),
            _ => return false,
        }
        true
    }

    /// Moves the scale one step up or down, and shows the new scale in the message line. At an
    /// end the scale stays.
    fn step_scale(&mut self, bigger: bool, cx: &mut Context<Self>) {
        let scale = self.config.read(cx).settings().scale;
        let next = scale.step(bigger).unwrap_or(scale);
        if next != scale {
            self.config.update(cx, |config, cx| {
                config.change(|settings| settings.scale = next, cx);
            });
        }
        self.message_line
            .update(cx, |line, cx| line.show(text::scale_answer(next), cx));
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
            self.save_place(cx);
        }
    }

    /// Keeps the sidebar view and the open note in the config.
    fn save_place(&mut self, cx: &mut Context<Self>) {
        let sidebar = SidebarSettings {
            width: whole(self.width),
            hidden: self.hidden,
            view: self.sidebar.read(cx).view(),
        };
        let last_note = self.pane.read(cx).path().cloned();
        self.config.update(cx, |config, cx| {
            config.change(
                |settings| {
                    settings.sidebar = sidebar;
                    settings.last_note = last_note;
                },
                cx,
            );
        });
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
            .when(!self.covered(), |view| view.key_context(Place::Window.key_context()))
            .on_action(cx.listener(Self::run_key))
            .on_action(cx.listener(Self::rename_tag_from_line))
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
            .children(self.settings.clone())
            .children(self.shortcut_list.clone())
            .child(self.app_menu.clone())
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
