//! The sidebar: one panel that shows the tree or the note list of one tree item.
//!
//! The tree has "All notes", then the folders, then the tags. In a list, `↑` and `↓` open each
//! note at once. A name field of a folder writes its name through the core door. The name
//! field of a tag writes through its owner. For other changes, the sidebar sends a
//! [`SidebarEvent`].
//!
//! The search field sits at the top. While it holds text, the note list shows the notes that
//! match it, and `Esc` clears it and shows the last sidebar view again.

mod list;
mod tree;

use std::cell::Cell;
use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;
use std::time::SystemTime;

use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, CursorStyle, Div, ElementId,
    Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement as _, IntoElement,
    KeyContext, KeyDownEvent, MouseButton, ParentElement, Pixels, Render, Role, ScrollStrategy,
    SharedString, StatefulInteractiveElement as _, Styled, Subscription, Task, TestSupportExt as _,
    UniformListScrollHandle, Window, div, px, uniform_list,
};
use holm_core::model::{Change, NoteInfo, Order};
use holm_core::{FolderPath, NotePath, TagName};

use crate::config::{Config, SavedSidebarView};
use crate::core_door::{CoreDoor, Done, DoorEvent, Then};
use crate::error::AppError;
use crate::icons::Icon;
use crate::name_field::{NameEvent, NameField, NameWrite, Wording};
use crate::pane::{NotePane, PaneEvent};
use crate::shortcuts::{Command, Place, Run};
use crate::text;

use list::NoteList;
pub(crate) use tree::Item;
use tree::{Entry, Label, Lists, SidebarView, Tree};

const TREE_ROW: f32 = 28.;
const NOTE_ROW: f32 = 52.;

/// A note or a folder, for a delete or a move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Note(NotePath),
    Folder(FolderPath),
}

/// What the sidebar asks of the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SidebarEvent {
    /// Open this note in the editor.
    Open(NotePath),
    FocusEditor,
    /// Open the title field of the open note.
    RenameNote,
    /// Delete, with the dialog.
    Delete(Target),
    /// Move, with the folder picker.
    Move(Target),
    /// Rename a tag in all notes, with the dialog.
    RenameTag(TagName),
    /// Remove a tag from all notes, with the dialog.
    DeleteTag(TagName),
    /// The sidebar view changed. The app keeps it in the config.
    ViewChanged,
    /// Open or close the app menu.
    AppMenu,
}

/// Renames the tag `from` to a typed name, and ends with the result. It shows the answer of
/// a fail.
pub(crate) type TagRename =
    Rc<dyn Fn(TagName, String, Then<Result<(), AppError>>, &mut Window, &mut App)>;

/// A name field in the tree: for a folder or a tag after `F2`, or for a new folder.
struct Naming {
    named: Named,
    field: Entity<NameField<()>>,
    _subscription: Subscription,
}

/// What a name field names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Named {
    Folder(FolderPath),
    Tag(TagName),
    /// A new folder in this parent folder. It exists only after `Enter`.
    NewFolder(FolderPath),
}

impl Named {
    /// What a rename of `item` names. "All notes" has no name.
    fn item(item: &Item) -> Option<Self> {
        match item {
            Item::All => None,
            Item::Folder(path) => Some(Self::Folder(path.clone())),
            Item::Tag(tag) => Some(Self::Tag(tag.clone())),
        }
    }

    fn moved(&self, change: &Change) -> Option<Self> {
        match self {
            Self::Folder(path) => change.folder_after(path).map(Self::Folder),
            Self::Tag(tag) => change.tag_after(tag).map(Self::Tag),
            Self::NewFolder(parent) => change.folder_after(parent).map(Self::NewFolder),
        }
    }
}

pub(crate) struct Sidebar {
    core_door: Entity<CoreDoor>,
    config: Entity<Config>,
    lists: Lists,
    view: SidebarView,
    open: HashSet<Item>,
    tree: Tree,
    list: NoteList,
    /// The pane, for the open note. The open note is the selected row of a list.
    pane: Entity<NotePane>,
    /// The open note after it is gone from the notes folder, with its neighbor in the list
    /// before. The app asks for the next note only after the list lost the gone note.
    left: Option<(NotePath, NotePath)>,
    field: Option<Naming>,
    search: Entity<InputState>,
    /// The trimmed text of the search field. With text, the list shows `found`.
    query: String,
    found: NoteList,
    finding: Option<Task<()>>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    /// The window bounds of the app menu button in the last frame.
    menu_button: Rc<Cell<Bounds<Pixels>>>,
    _subscriptions: Vec<Subscription>,
}

impl Sidebar {
    pub(crate) fn new(
        core_door: Entity<CoreDoor>,
        pane: Entity<NotePane>,
        config: Entity<Config>,
        view: &SavedSidebarView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let changed = cx.subscribe(&core_door, |sidebar, _, event: &DoorEvent, cx| {
            sidebar.changed(event, cx);
        });
        let opened = cx.subscribe(&pane, |sidebar, _, event: &PaneEvent, cx| {
            if !matches!(event, PaneEvent::Gone(_)) {
                sidebar.open_changed(cx);
            }
        });
        let config_changed = cx.observe(&config, |sidebar, _, cx| {
            if sidebar.lists.order != sidebar.order(cx) {
                sidebar.refresh(cx);
            }
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(text::SEARCH));
        let search_changed = cx.subscribe(&search, |sidebar, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                let query = input.read(cx).value().trim().to_owned();
                if query != sidebar.query {
                    sidebar.query = query;
                    sidebar.find(cx);
                }
            }
        });
        let mut sidebar = Self {
            core_door,
            config,
            lists: Lists::default(),
            view: SidebarView::from(view),
            open: HashSet::new(),
            tree: Tree::default(),
            list: NoteList::default(),
            pane,
            left: None,
            field: None,
            search,
            query: String::new(),
            found: NoteList::default(),
            finding: None,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            menu_button: Rc::default(),
            _subscriptions: vec![changed, opened, config_changed, search_changed],
        };
        sidebar.refresh(cx);
        sidebar
    }

    /// The window bounds of the app menu button in the last frame. The sidebar updates them
    /// at each frame.
    pub(crate) fn menu_button(&self) -> Rc<Cell<Bounds<Pixels>>> {
        self.menu_button.clone()
    }

    /// The note order of the settings.
    fn order(&self, cx: &App) -> Order {
        self.config.read(cx).settings().note_order
    }

    pub(crate) fn view(&self) -> SavedSidebarView {
        SavedSidebarView::from(&self.view)
    }

    /// Takes the end of a job or of outside changes: it follows the moves, and then shows
    /// the snapshot.
    fn changed(&mut self, event: &DoorEvent, cx: &mut Context<Self>) {
        if event.is_empty() {
            return;
        }
        let view = self.view.clone();
        for change in event.changes() {
            self.follow(change, cx);
        }
        self.refresh(cx);
        if self.view != view {
            self.scroll_to_selection(cx);
        }
    }

    /// Moves the sidebar view, the open items and the name field with `change`, when it is a move.
    fn follow(&mut self, change: &Change, cx: &mut Context<Self>) {
        let view = match &self.view {
            SidebarView::Tree(item) => item.moved(change).map(SidebarView::Tree),
            SidebarView::List(item) => item.moved(change).map(SidebarView::List),
        };
        if let Some(view) = view {
            self.view = view;
            cx.emit(SidebarEvent::ViewChanged);
        }
        self.open = self
            .open
            .drain()
            .map(|item| item.moved(change).unwrap_or(item))
            .collect();
        if let Some(naming) = &mut self.field
            && let Some(named) = naming.named.moved(change)
        {
            naming.named = named;
        }
    }

    /// Builds the lists from the snapshot of the core door. An item that is gone leaves the
    /// sidebar view and the open items.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let snapshot = self.core_door.read(cx).snapshot();
        self.lists = Lists::new(snapshot, self.order(cx));
        let item = self.lists.nearest(self.view.item());
        if &item != self.view.item() {
            self.show_in_tree(item, cx);
        }
        self.open.retain(|item| self.lists.has(item));
        self.rebuild(cx);
        cx.notify();
        if self.searching() {
            self.find(cx);
        }
    }

    fn searching(&self) -> bool {
        !self.query.is_empty()
    }

    /// Whether the sidebar shows a note list: the list of a tree item, or the search results.
    fn shows_list(&self) -> bool {
        self.searching() || matches!(self.view, SidebarView::List(_))
    }

    /// Searches the notes for the query. A result for an old query is dropped.
    fn find(&mut self, cx: &mut Context<Self>) {
        if !self.searching() {
            self.finding = None;
            self.found = NoteList::default();
            self.rebuild_list(cx);
            cx.notify();
            return;
        }
        let (query, order) = (self.query.clone(), self.order(cx));
        let search = self.core_door.update(cx, |core_door, cx| {
            core_door.search(query.clone(), order, cx)
        });
        self.finding = Some(cx.spawn(async move |sidebar, cx| {
            let found = search.await.map(NoteList::new);
            sidebar
                .update(cx, |sidebar, cx| {
                    if query != sidebar.query {
                        return;
                    }
                    sidebar.finding = None;
                    if let Some(found) = found {
                        sidebar.found = found;
                    }
                    sidebar.rebuild_list(cx);
                    sidebar.scroll_to_selection(cx);
                    cx.notify();
                })
                .ok();
        }));
    }

    /// Ctrl+Shift+F: gives the search field the focus, with its text selected.
    pub(crate) fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
    }

    /// Empties the search field, so the last sidebar view shows again. The list gets the focus.
    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query.clear();
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.find(cx);
        window.focus(&self.focus, cx);
        self.scroll_to_selection(cx);
    }

    /// `Enter` or `↓` in the search field: the first result opens, and the list gets the focus.
    fn enter_results(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        if let Some(path) = self.list.path(0) {
            let path = path.to_owned();
            cx.emit(SidebarEvent::Open(path));
        }
    }

    fn rebuild(&mut self, cx: &App) {
        self.rebuild_tree();
        self.rebuild_list(cx);
    }

    fn rebuild_tree(&mut self) {
        let new_folder = match self.field.as_ref().map(|naming| &naming.named) {
            Some(Named::NewFolder(parent)) => Some(parent),
            _ => None,
        };
        self.tree = self.lists.tree(&self.open, new_folder);
    }

    fn rebuild_list(&mut self, cx: &App) {
        let list = if self.searching() {
            self.found.clone()
        } else {
            self.lists.list(self.view.item())
        };
        if let Some(open) = self.open_note(cx)
            && self.lists.snapshot.note(open).is_none()
            && let Some(next) = self.neighbor(open)
        {
            self.left = Some((open.clone(), next));
        }
        self.list = list;
    }

    fn open_note<'a>(&self, cx: &'a App) -> Option<&'a NotePath> {
        self.pane.read(cx).path()
    }

    /// Selects the new open note.
    fn open_changed(&mut self, cx: &mut Context<Self>) {
        self.left = None;
        self.scroll_to_selection(cx);
        cx.notify();
    }

    /// The folder for a new note, and the tag it gets, from the item of the sidebar view. A note
    /// from the search results goes in the root folder.
    pub(crate) fn new_note_place(&self) -> (FolderPath, Option<TagName>) {
        if self.searching() {
            return (FolderPath::root(), None);
        }
        let item = self.view.item();
        let tag = match item {
            Item::Tag(name) => Some(name.clone()),
            Item::All | Item::Folder(_) => None,
        };
        (item.folder(), tag)
    }

    /// The note to open after the open note `gone` is gone: its neighbor in the list when
    /// it still exists, else the first note. With no notes, it is `None`.
    pub(crate) fn next_note(&self, gone: &NotePath, cx: &App) -> Option<NotePath> {
        self.core_door
            .read(cx)
            .snapshot()
            .note_or_first(self.neighbor(gone).as_ref(), self.order(cx))
            .map(|note| note.path.clone())
    }

    /// The note after `path` in the list, or else the one before it. For the open note after
    /// it is gone, it is its neighbor before.
    fn neighbor(&self, path: &NotePath) -> Option<NotePath> {
        let Some(at) = self.list.position(path) else {
            return self
                .left
                .as_ref()
                .filter(|(gone, _)| gone == path)
                .map(|(_, next)| next.clone());
        };
        self.list
            .path(at + 1)
            .or_else(|| at.checked_sub(1).and_then(|before| self.list.path(before)))
            .cloned()
    }

    /// Every note, in the order of the lists.
    pub(crate) fn notes(&self) -> impl Iterator<Item = &NoteInfo> {
        self.lists.notes.notes()
    }

    /// Every folder, sorted by path, for the folder picker.
    pub(crate) fn folders(&self) -> impl Iterator<Item = &FolderPath> {
        self.lists.folders.iter().map(|folder| &folder.path)
    }

    /// Shows the tree with `item` selected, and opens its parents.
    pub(crate) fn show_in_tree(&mut self, item: Item, cx: &mut Context<Self>) {
        let mut parent = item.parent();
        while let Some(item) = parent {
            parent = item.parent();
            self.open.insert(item);
        }
        self.set_view(SidebarView::Tree(item), cx);
    }

    fn set_view(&mut self, view: SidebarView, cx: &mut Context<Self>) {
        if self.view != view {
            self.view = view;
            cx.emit(SidebarEvent::ViewChanged);
        }
        self.rebuild(cx);
        self.scroll_to_selection(cx);
        cx.notify();
    }

    fn scroll_to_selection(&self, cx: &App) {
        let at = match &self.view {
            _ if self.shows_list() => self.selected_note(cx),
            SidebarView::Tree(item) => self.tree.entry_of(item),
            SidebarView::List(_) => self.selected_note(cx),
        };
        if let Some(at) = at {
            self.scroll.scroll_to_item(at, ScrollStrategy::Nearest);
        }
    }

    fn selected_note(&self, cx: &App) -> Option<usize> {
        self.list.position(self.open_note(cx)?)
    }

    /// Starts the name field of a tag in the tree, with its full name, so a rename can also move
    /// the tag under another parent. `rename` writes the typed name. It does nothing while the
    /// job of a name field runs.
    pub(crate) fn rename_tag(
        &mut self,
        tag: TagName,
        rename: TagRename,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sidebar = cx.weak_entity();
        // The name field follows moves, so the job takes the tag it names at `Enter`.
        let write: NameWrite<()> = Rc::new(move |name, done, window, cx| {
            match sidebar.read_with(cx, |sidebar, _| sidebar.named()) {
                Ok(Some(Named::Tag(from))) => rename(from, name, done, window, cx),
                _ => done(Ok(()), cx),
            }
        });
        let text = tag.to_string();
        let item = Item::Tag(tag.clone());
        self.open_name_field(Named::Tag(tag), Some(&text), item, write, window, cx);
    }

    fn rename_folder(&mut self, folder: FolderPath, window: &mut Window, cx: &mut Context<Self>) {
        let item = Item::Folder(folder.clone());
        let text = item.name();
        let write = Self::folder_write(cx);
        self.open_name_field(Named::Folder(folder), Some(&text), item, write, window, cx);
    }

    /// Starts an empty name field for a new folder in the folder of the sidebar view, as its first
    /// child row. Nothing is on disk before `Enter`.
    pub(crate) fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent = self.view.item().folder();
        let item = if parent.is_root() {
            Item::All
        } else {
            let item = Item::Folder(parent.clone());
            self.open.insert(item.clone());
            item
        };
        let write = Self::folder_write(cx);
        self.open_name_field(Named::NewFolder(parent), None, item, write, window, cx);
    }

    /// What the open name field names.
    fn named(&self) -> Option<Named> {
        self.field.as_ref().map(|naming| naming.named.clone())
    }

    /// Opens the name field for `named`, with `text` for a rename, and shows `item` selected in
    /// the tree.
    fn open_name_field(
        &mut self,
        named: Named,
        text: Option<&str>,
        item: Item,
        write: NameWrite<()>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.writing(cx) {
            return;
        }
        if self.searching() {
            self.clear_search(window, cx);
        }
        let wording = Wording {
            id: "name-input",
            label: text::NAME_FIELD.into(),
            placeholder: SharedString::default(),
        };
        let home = self.focus.clone();
        let field = match text {
            Some(text) => cx.new(|cx| NameField::rename(wording, text, write, home, window, cx)),
            None => cx.new(|cx| NameField::new_name(wording, write, home, window, cx)),
        };
        let subscription = cx.subscribe(&field, |sidebar, _, _: &NameEvent<()>, cx| {
            sidebar.field = None;
            sidebar.rebuild(cx);
            cx.notify();
        });
        self.field = Some(Naming {
            named,
            field,
            _subscription: subscription,
        });
        self.show_in_tree(item, cx);
    }

    /// The job of a folder name field. The name field follows moves, so the job takes the
    /// folder it names at `Enter`.
    fn folder_write(cx: &Context<Self>) -> NameWrite<()> {
        let sidebar = cx.weak_entity();
        Rc::new(move |name, done, _, cx| {
            sidebar
                .update(cx, |sidebar, cx| match sidebar.named() {
                    Some(Named::Folder(folder)) => {
                        sidebar.write_folder_name(folder, name, done, cx);
                    }
                    Some(Named::NewFolder(parent)) => sidebar.create_folder(parent, name, done, cx),
                    Some(Named::Tag(_)) | None => done(Ok(()), cx),
                })
                .ok();
        })
    }

    fn write_folder_name(
        &mut self,
        folder: FolderPath,
        name: String,
        done: Then<Result<(), AppError>>,
        cx: &mut Context<Self>,
    ) {
        let sidebar = cx.weak_entity();
        let renamed = Done::result(move |renamed: Result<FolderPath, AppError>, cx: &mut App| {
            if renamed.is_ok() {
                sidebar
                    .update(cx, |_, cx| cx.emit(SidebarEvent::ViewChanged))
                    .ok();
            }
            done(renamed.map(drop), cx);
        });
        self.core_door.update(cx, |core_door, cx| {
            core_door.rename_folder(folder, name, renamed, cx);
        });
    }

    fn create_folder(
        &mut self,
        parent: FolderPath,
        name: String,
        done: Then<Result<(), AppError>>,
        cx: &mut Context<Self>,
    ) {
        let sidebar = cx.weak_entity();
        let created = Done::result(move |created: Result<FolderPath, AppError>, cx: &mut App| {
            let shown = created.map(|folder| {
                sidebar
                    .update(cx, |sidebar, cx| {
                        sidebar.show_in_tree(Item::Folder(folder), cx);
                    })
                    .ok();
            });
            done(shown, cx);
        });
        self.core_door.update(cx, |core_door, cx| {
            core_door.create_folder(parent, name, created, cx);
        });
    }

    /// Whether the job of a name field runs.
    fn writing(&self, cx: &App) -> bool {
        self.field
            .as_ref()
            .is_some_and(|naming| naming.field.read(cx).writing())
    }

    /// The key context of the sidebar while it has the focus itself: [`Place::Sidebar`] with
    /// [`Place::Tree`] or [`Place::NoteList`]. The search field and the name field get no
    /// sidebar keys.
    fn key_context(&self) -> KeyContext {
        let mut context = KeyContext::default();
        context.add(Place::Sidebar.key_context());
        let place = if self.shows_list() {
            Place::NoteList
        } else {
            Place::Tree
        };
        context.add(place.key_context());
        context
    }

    /// A shortcut of the tree or the note list.
    fn run(&mut self, Run(command): &Run, window: &mut Window, cx: &mut Context<Self>) {
        let handled = match command {
            Command::Move => {
                if let Some(target) = self.target(cx) {
                    cx.emit(SidebarEvent::Move(target));
                }
                true
            }
            _ if self.shows_list() => self.list_command(*command, window, cx),
            _ => self.tree_command(*command, window, cx),
        };
        if !handled {
            cx.propagate();
        }
    }

    /// Up, Down, Left and Right in the tree or the note list.
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if self.field.is_some()
            || keystroke.modifiers.modified()
            || self.search.focus_handle(cx).is_focused(window)
        {
            return;
        }
        let key = keystroke.key.as_str();
        let handled = if self.shows_list() {
            self.list_move(key, cx)
        } else {
            self.tree_move(key, cx)
        };
        if handled {
            cx.stop_propagation();
        }
    }

    /// The note or folder that `Delete` and Ctrl+Shift+M act on.
    fn target(&self, cx: &App) -> Option<Target> {
        match &self.view {
            _ if self.shows_list() => self
                .selected_note(cx)
                .and_then(|at| self.list.path(at))
                .map(|path| Target::Note(path.to_owned())),
            SidebarView::Tree(Item::Folder(folder)) => Some(Target::Folder(folder.clone())),
            SidebarView::Tree(_) | SidebarView::List(_) => None,
        }
    }

    /// A shortcut of the tree.
    fn tree_command(
        &mut self,
        command: Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let SidebarView::Tree(item) = &self.view else {
            return false;
        };
        let item = item.clone();
        match command {
            Command::ShowNotes => self.set_view(SidebarView::List(item), cx),
            Command::FocusEditor => cx.emit(SidebarEvent::FocusEditor),
            Command::Rename => match item {
                Item::Tag(tag) => cx.emit(SidebarEvent::RenameTag(tag)),
                Item::Folder(folder) => self.rename_folder(folder, window, cx),
                Item::All => {}
            },
            Command::Delete => match item {
                Item::Tag(tag) => cx.emit(SidebarEvent::DeleteTag(tag)),
                Item::Folder(folder) => cx.emit(SidebarEvent::Delete(Target::Folder(folder))),
                Item::All => {}
            },
            _ => return false,
        }
        true
    }

    /// Up, Down, Left or Right in the tree.
    fn tree_move(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let SidebarView::Tree(item) = &self.view else {
            return false;
        };
        let item = item.clone();
        let rows = &self.tree.rows;
        let at = self.tree.row_of(&item);
        match key {
            "up" | "down" => {
                let next = match (at, key) {
                    (Some(at), "up") => at.saturating_sub(1),
                    (Some(at), _) => (at + 1).min(rows.len().saturating_sub(1)),
                    (None, _) => 0,
                };
                if let Some(row) = rows.get(next) {
                    let item = row.item.clone();
                    self.set_view(SidebarView::Tree(item), cx);
                }
            }
            "right" => {
                if at.is_some_and(|at| rows[at].has_children) {
                    self.open.insert(item);
                    self.rebuild(cx);
                    cx.notify();
                }
            }
            "left" => {
                if at.is_some_and(|at| rows[at].open) {
                    self.open.remove(&item);
                    self.rebuild(cx);
                    cx.notify();
                } else if let Some(parent) = item.parent() {
                    self.set_view(SidebarView::Tree(parent), cx);
                }
            }
            _ => return false,
        }
        true
    }

    /// A shortcut of the note list.
    fn list_command(
        &mut self,
        command: Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match command {
            Command::FocusEditor => cx.emit(SidebarEvent::FocusEditor),
            Command::Back if self.searching() => self.clear_search(window, cx),
            Command::Back => {
                let item = self.view.item().clone();
                self.show_in_tree(item, cx);
            }
            Command::Rename => {
                if self.selected_note(cx).is_some() {
                    cx.emit(SidebarEvent::RenameNote);
                }
            }
            Command::Delete => {
                if let Some(target) = self.target(cx) {
                    cx.emit(SidebarEvent::Delete(target));
                }
            }
            _ => return false,
        }
        true
    }

    /// Up or Down in the note list opens the note above or below.
    fn list_move(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        match key {
            "up" | "down" => {
                let next = match (self.selected_note(cx), key) {
                    (Some(at), "up") => at.checked_sub(1),
                    (Some(at), _) => Some(at + 1),
                    (None, _) => Some(0),
                };
                if let Some(path) = next.and_then(|next| self.list.path(next)) {
                    let path = path.to_owned();
                    cx.emit(SidebarEvent::Open(path));
                }
            }
            _ => return false,
        }
        true
    }

    fn search_field(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let button = self.menu_button.clone();
        h_flex()
            .flex_none()
            .gap_1()
            .px_2()
            .py_2()
            .on_children_prepainted(move |children, _, _| {
                if let Some(bounds) = children.get(1) {
                    button.set(*bounds);
                }
            })
            .capture_action(cx.listener(|sidebar, _: &Escape, window, cx| {
                sidebar.clear_search(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|sidebar, _: &Enter, window, cx| {
                sidebar.enter_results(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|sidebar, _: &MoveDown, window, cx| {
                sidebar.enter_results(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|_, _: &MoveUp, _, cx| cx.stop_propagation()))
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.search)
                        .id("search")
                        .aria_label(text::SEARCH)
                        .prefix(Icon::view(Icon::Search, theme.muted_foreground))
                        .small()
                        .appearance(false)
                        .rounded(theme.radius)
                        .border_1()
                        .map(|field| {
                            if self.search.focus_handle(cx).is_focused(window) {
                                field.bg(theme.background).border_color(theme.border)
                            } else {
                                field
                                    .bg(theme.accent)
                                    .border_color(gpui_kit::transparent_black())
                            }
                        }),
                ),
            )
            .child(
                div()
                    .id("app-menu-button")
                    .role(Role::Button)
                    .aria_label(text::APP_MENU)
                    .test_support()
                    .flex_none()
                    .p_1()
                    .rounded(theme.radius)
                    .cursor(CursorStyle::PointingHand)
                    .hover(|button| button.bg(theme.secondary_hover))
                    .child(Icon::view(Icon::Menu, theme.muted_foreground))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, window, cx| {
                            // The sidebar would take the focus from the part that has it.
                            window.prevent_default();
                            cx.stop_propagation();
                            cx.emit(SidebarEvent::AppMenu);
                        }),
                    ),
            )
    }

    fn tree_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let entries: Vec<Entry> = range
            .filter_map(|at| self.tree.entries.get(at).copied())
            .collect();
        entries
            .into_iter()
            .map(|entry| match entry {
                Entry::Label(label) => div()
                    .w_full()
                    .h(px(TREE_ROW))
                    .px_3()
                    .pb_1()
                    .flex()
                    .items_end()
                    .text_size(px(11.))
                    .text_color(cx.theme().muted_foreground)
                    .child(match label {
                        Label::Folders => text::FOLDERS,
                        Label::Tags => text::TAGS,
                    })
                    .into_any_element(),
                Entry::Row(at) => self.tree_row(at, cx),
                Entry::NewFolder(depth) => self.new_folder_row(depth, cx),
            })
            .collect()
    }

    fn tree_row(&mut self, at: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let row = &self.tree.rows[at];
        let selected = matches!(&self.view, SidebarView::Tree(item) if item == &row.item);
        let icon = match &row.item {
            Item::All => Icon::All,
            Item::Folder(_) if row.open => Icon::FolderOpen,
            Item::Folder(_) => Icon::Folder,
            Item::Tag(_) if !row.has_children => Icon::Tag,
            Item::Tag(_) if row.open => Icon::TagsOpen,
            Item::Tag(_) => Icon::Tags,
        };
        let name = SharedString::from(row.name.clone());
        let aria = item_label(&row.item);
        let named = Named::item(&row.item);
        let field = self
            .field
            .as_ref()
            .filter(|naming| Some(&naming.named) == named.as_ref());
        let editing = field.is_some();
        let label = match field {
            Some(naming) => field_label(&naming.field),
            None => div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(name)
                .into_any_element(),
        };
        let item = row.item.clone();
        let has_children = row.has_children;
        let toggle = div()
            .id(ElementId::NamedInteger("toggle".into(), at as u64))
            .flex_none()
            .child(Icon::view(icon, theme.muted_foreground))
            .on_click(cx.listener(move |sidebar, _: &ClickEvent, _, cx| {
                if has_children {
                    if !sidebar.open.remove(&item) {
                        sidebar.open.insert(item.clone());
                    }
                    sidebar.rebuild(cx);
                    cx.notify();
                    cx.stop_propagation();
                }
            }));
        let item = row.item.clone();
        row_frame(row.depth, editing, cx)
            .id(ElementId::NamedInteger("tree-row".into(), at as u64))
            .role(Role::TreeItem)
            .aria_label(aria)
            .aria_selected(selected)
            .when(row.has_children, |row_element| {
                row_element.aria_expanded(row.open)
            })
            .test_support()
            .when(selected, |row| row.bg(theme.selection))
            .child(toggle)
            .child(label)
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.muted_foreground)
                    .child(row.count.to_string()),
            )
            .on_click(cx.listener(move |sidebar, _: &ClickEvent, window, cx| {
                window.focus(&sidebar.focus, cx);
                sidebar.set_view(SidebarView::List(item.clone()), cx);
            }))
            .into_any_element()
    }

    /// The row of the name field for a new folder.
    fn new_folder_row(&self, depth: usize, cx: &App) -> AnyElement {
        let Some(naming) = &self.field else {
            return div().into_any_element();
        };
        row_frame(depth, true, cx)
            .id("new-folder")
            .role(Role::TreeItem)
            .aria_label(text::NEW_FOLDER)
            .test_support()
            .child(Icon::view(Icon::Folder, cx.theme().muted_foreground))
            .child(field_label(&naming.field))
            .into_any_element()
    }

    /// The head of a note list: `‹`, the name of the list, and its count. A click leaves the
    /// list.
    fn back(
        &self,
        label: impl Into<SharedString>,
        name: impl Into<SharedString>,
        leave: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        h_flex()
            .id("back")
            .role(Role::Button)
            .aria_label(label)
            .test_support()
            .w_full()
            .flex_none()
            .px_3()
            .py_2()
            .gap_1p5()
            .border_b_1()
            .border_color(theme.border)
            .text_color(theme.muted_foreground)
            .child("‹")
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.foreground)
                    .child(name.into()),
            )
            .child(div().text_size(px(12.)).child(self.list.len().to_string()))
            .on_click(leave)
            .into_any_element()
    }

    fn note_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme();
        let selected = self.selected_note(cx);
        range
            .filter_map(|at| self.list.note(at).map(|note| (at, note)))
            .map(|(at, note)| {
                let path = note.path.clone();
                let title = h_flex()
                    .gap_1p5()
                    .children(note.pinned.then(|| {
                        div()
                            .text_size(px(10.))
                            .text_color(theme.primary)
                            .child(text::PIN_MARK)
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(note.title.clone()),
                    );
                let meta = if note.preview.is_empty() {
                    when(note.modified)
                } else {
                    format!("{} · {}", when(note.modified), note.preview)
                };
                v_flex()
                    .id(ElementId::NamedInteger("note-row".into(), at as u64))
                    .role(Role::ListItem)
                    .aria_label(note.title.clone())
                    .aria_selected(selected == Some(at))
                    .test_support()
                    .w_full()
                    .h(px(NOTE_ROW))
                    .px_3()
                    .gap_0p5()
                    .justify_center()
                    .when(selected == Some(at), |row| row.bg(theme.selection))
                    .child(title)
                    .child(
                        div()
                            .text_size(px(12.))
                            .truncate()
                            .text_color(theme.muted_foreground)
                            .child(meta),
                    )
                    .on_click(cx.listener(move |sidebar, _: &ClickEvent, window, cx| {
                        window.focus(&sidebar.focus, cx);
                        cx.emit(SidebarEvent::Open(path.clone()));
                    }))
                    .into_any_element()
            })
            .collect()
    }
}

/// The time of the last change: the hour today, else the day.
fn when(time: SystemTime) -> String {
    let zone = jiff::tz::TimeZone::system();
    let Ok(stamp) = jiff::Timestamp::try_from(time) else {
        return String::new();
    };
    let time = stamp.to_zoned(zone.clone());
    let now = jiff::Timestamp::now().to_zoned(zone);
    let format = if time.date() == now.date() {
        "%H:%M"
    } else if time.year() == now.year() {
        "%-d %b"
    } else {
        "%-d %b %Y"
    };
    time.strftime(format).to_string()
}

/// The accessible name of a tree item: "All notes", the folder path, or `#` and the tag.
fn item_label(item: &Item) -> String {
    match item {
        Item::All => item.name(),
        Item::Folder(path) => text::folder_label(path),
        Item::Tag(name) => text::tag_label(name),
    }
}

/// The frame of a tree row: its height, its indent, and the bar of an open name field.
fn row_frame(depth: usize, editing: bool, cx: &App) -> Div {
    let theme = cx.theme();
    let depth = f32::from(u16::try_from(depth).unwrap_or(u16::MAX));
    h_flex()
        .w_full()
        .h(px(TREE_ROW))
        .pr_3()
        .pl(px(12. + 16. * depth))
        .gap_2()
        .when(editing, |row| {
            row.relative().child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(px(2.))
                    .bg(theme.ring),
            )
        })
}

/// The name field in the place of the name of a tree row.
fn field_label(field: &Entity<NameField<()>>) -> AnyElement {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .child(field.clone())
        .into_any_element()
}

impl EventEmitter<SidebarEvent> for Sidebar {}

impl Focusable for Sidebar {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Sidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let header = match &self.view {
            _ if self.searching() => Some(self.back(
                text::search_header(&self.query),
                text::search_header(&self.query),
                cx.listener(|sidebar, _: &ClickEvent, window, cx| {
                    sidebar.clear_search(window, cx);
                }),
                cx,
            )),
            SidebarView::Tree(_) => None,
            SidebarView::List(item) => {
                let name = match item {
                    Item::All | Item::Folder(_) => item.name(),
                    Item::Tag(name) => text::tag_label(name),
                };
                let label = item_label(item);
                let item = item.clone();
                Some(self.back(
                    label,
                    name,
                    cx.listener(move |sidebar, _: &ClickEvent, window, cx| {
                        window.focus(&sidebar.focus, cx);
                        sidebar.show_in_tree(item.clone(), cx);
                    }),
                    cx,
                ))
            }
        };
        let body = if self.shows_list() {
            uniform_list(
                "notes",
                self.list.len(),
                cx.processor(|sidebar, range, _, cx| sidebar.note_rows(range, cx)),
            )
        } else {
            uniform_list(
                "tree",
                self.tree.entries.len(),
                cx.processor(|sidebar, range, _, cx| sidebar.tree_rows(range, cx)),
            )
        };
        let empty = self.shows_list() && self.list.is_empty() && self.finding.is_none();
        v_flex()
            .id("sidebar")
            .test_support()
            .when(self.focus.is_focused(window), |sidebar| {
                sidebar.key_context(self.key_context())
            })
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(Self::run))
            .size_full()
            .bg(theme.sidebar)
            .child(self.search_field(window, cx))
            .children(header)
            .children(empty.then(|| {
                div()
                    .px_3()
                    .py_2()
                    .text_color(theme.muted_foreground)
                    .child(text::NO_NOTES)
            }))
            .child(body.track_scroll(&self.scroll).flex_1().w_full())
    }
}
