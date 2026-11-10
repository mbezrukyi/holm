//! The sidebar: one panel that shows the folder tree or the note list of one tree item.
//!
//! The tree has "All notes", then the folders, then the tags. In a list, `↑` and `↓` open each
//! note at once. The sidebar changes no files itself. It sends a [`SidebarEvent`], and the app
//! saves the open note before each change.

mod tree;

use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement, Render, Role, ScrollStrategy, SharedString, StatefulInteractiveElement as _,
    Styled, Subscription, Task, TestSupportExt as _, UniformListScrollHandle, Window, div, px,
    uniform_list,
};
use holm_core::Core;
use holm_core::model::{NoteInfo, Order};

use crate::error::AppError;
use crate::icons::Icon;
use crate::{changes, text};

pub(crate) use tree::{Item, View};
use tree::{Row, Snapshot};

const TREE_ROW: f32 = 28.;
const NOTE_ROW: f32 = 52.;

/// One line of the tree: a group label or a row.
#[derive(Clone, Copy, Debug)]
enum Entry {
    Label(&'static str),
    Row(usize),
}

/// A note or a folder, for a delete or a move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Note(PathBuf),
    Folder(PathBuf),
}

/// What the sidebar asks of the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SidebarEvent {
    /// Open this note in the editor.
    Open(PathBuf),
    FocusEditor,
    /// Put the cursor on the title of the open note.
    EditTitle,
    /// Delete, with the dialog.
    Delete(Target),
    /// Move, with the folder picker.
    Move(Target),
    RenameFolder {
        folder: PathBuf,
        name: String,
    },
    /// Rename a tag in all notes, with the dialog.
    RenameTag {
        from: String,
        to: String,
    },
    /// Remove a tag from all notes, with the dialog.
    DeleteTag(String),
    /// The view changed. The app keeps it in the config.
    ViewChanged,
}

/// A name field for a folder or a tag in the tree, after `F2`.
struct Rename {
    item: Item,
    input: Entity<InputState>,
    _subscription: Subscription,
}

pub(crate) struct Sidebar {
    core: Core,
    order: Order,
    snapshot: Snapshot,
    view: View,
    open: HashSet<Item>,
    rows: Vec<Row>,
    /// The rows of the tree with the group labels between them.
    entries: Vec<Entry>,
    list: Vec<NoteInfo>,
    /// The note in the editor. It is the selected row of a list.
    current: Option<PathBuf>,
    rename: Option<Rename>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    error: Option<&'static str>,
    loading: Option<Task<()>>,
    _changes: Task<()>,
}

impl Sidebar {
    pub(crate) fn new(
        core: Core,
        order: Order,
        view: View,
        current: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) -> Self {
        let changes = changes::watch_revision(&core, cx, Self::refresh);
        let mut sidebar = Self {
            core,
            order,
            snapshot: Snapshot::default(),
            view,
            open: HashSet::new(),
            rows: Vec::new(),
            entries: Vec::new(),
            list: Vec::new(),
            current,
            rename: None,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            error: None,
            loading: None,
            _changes: changes,
        };
        sidebar.refresh(cx);
        sidebar
    }

    pub(crate) fn view(&self) -> &View {
        &self.view
    }

    /// Reads the notes, folders and tags again. The sidebar calls it itself when the core
    /// revision grows.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let order = self.order;
        let job = cx.background_spawn(async move {
            let revision = core.revision();
            let snapshot = Snapshot {
                notes: core.notes(order)?,
                folders: core.folders()?,
                tags: core.tags()?,
            };
            Ok::<_, holm_core::Error>((revision, snapshot))
        });
        self.loading = Some(cx.spawn(async move |sidebar, cx| {
            let result = job.await;
            sidebar
                .update(cx, |sidebar, cx| sidebar.loaded(result, cx))
                .ok();
        }));
    }

    fn loaded(&mut self, result: holm_core::Result<(u64, Snapshot)>, cx: &mut Context<Self>) {
        self.loading = None;
        match result {
            Ok((revision, snapshot)) => {
                self.snapshot = snapshot;
                self.error = None;
                // An old copy may miss an item that a later write made. The next copy decides.
                let current = revision == self.core.revision();
                if current && !self.snapshot.has(self.view.item()) {
                    self.view = View::List(Item::All);
                    cx.emit(SidebarEvent::ViewChanged);
                }
                self.open.retain(|item| self.snapshot.has(item));
            }
            Err(error) => {
                let error = AppError::from(error);
                tracing::warn!(error = %crate::chain(&error), "cannot read the notes for the sidebar");
                self.error = Some(error.message());
            }
        }
        self.rebuild();
        cx.notify();
    }

    fn rebuild(&mut self) {
        self.rows = self.snapshot.rows(&self.open);
        self.entries.clear();
        for (at, row) in self.rows.iter().enumerate() {
            let first = |kind: fn(&Item) -> bool| {
                kind(&row.item) && !self.rows[..at].iter().any(|row| kind(&row.item))
            };
            if first(|item| matches!(item, Item::Folder(_))) {
                self.entries.push(Entry::Label(text::FOLDERS));
            }
            if first(|item| matches!(item, Item::Tag(_))) {
                self.entries.push(Entry::Label(text::TAGS));
            }
            self.entries.push(Entry::Row(at));
        }
        self.list = self
            .snapshot
            .list(self.view.item())
            .into_iter()
            .cloned()
            .collect();
    }

    /// Marks the note in the editor.
    pub(crate) fn set_current(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.current = Some(path);
        self.scroll_to_selection();
        cx.notify();
    }

    /// The folder for a new note, and the tag it gets, from the item of the view.
    pub(crate) fn new_note_place(&self) -> (PathBuf, Option<String>) {
        let item = self.view.item();
        let tag = match item {
            Item::Tag(name) => Some(name.clone()),
            Item::All | Item::Folder(_) => None,
        };
        (item.folder().to_owned(), tag)
    }

    /// The note after `path` in the list, or else the one before it.
    pub(crate) fn neighbor(&self, path: &Path) -> Option<PathBuf> {
        let at = self.list.iter().position(|note| note.path == path)?;
        self.list
            .get(at + 1)
            .or_else(|| at.checked_sub(1).and_then(|before| self.list.get(before)))
            .map(|note| note.path.clone())
    }

    /// Every folder, sorted by path, for the folder picker.
    pub(crate) fn folders(&self) -> &[PathBuf] {
        &self.snapshot.folders
    }

    /// Shows the tree with `item` selected, and opens its parents.
    pub(crate) fn show_in_tree(&mut self, item: Item, cx: &mut Context<Self>) {
        let mut parent = item.parent();
        while let Some(item) = parent {
            parent = item.parent();
            self.open.insert(item);
        }
        self.set_view(View::Tree(item), cx);
    }

    fn set_view(&mut self, view: View, cx: &mut Context<Self>) {
        if self.view != view {
            self.view = view;
            cx.emit(SidebarEvent::ViewChanged);
        }
        self.rebuild();
        self.scroll_to_selection();
        cx.notify();
    }

    fn scroll_to_selection(&self) {
        let at = match &self.view {
            View::Tree(item) => self
                .entries
                .iter()
                .position(|entry| matches!(entry, Entry::Row(at) if &self.rows[*at].item == item)),
            View::List(_) => self.selected_note(),
        };
        if let Some(at) = at {
            self.scroll.scroll_to_item(at, ScrollStrategy::Nearest);
        }
    }

    fn selected_note(&self) -> Option<usize> {
        let current = self.current.as_ref()?;
        self.list.iter().position(|note| &note.path == current)
    }

    /// Starts the name field for a folder or a tag in the tree. A tag field holds the full
    /// name, so a rename can also move the tag under another parent.
    pub(crate) fn rename(&mut self, item: Item, window: &mut Window, cx: &mut Context<Self>) {
        let name = match &item {
            Item::All => return,
            Item::Folder(folder) => folder
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            Item::Tag(tag) => tag.clone(),
        };
        self.show_in_tree(item.clone(), cx);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |sidebar, input, event, window, cx| match event {
                    InputEvent::PressEnter { .. } => {
                        let name = input.read(cx).value().trim().to_owned();
                        if let Some(rename) = sidebar.rename.take()
                            && !name.is_empty()
                        {
                            match rename.item {
                                Item::Folder(folder) => {
                                    cx.emit(SidebarEvent::RenameFolder { folder, name });
                                }
                                Item::Tag(from) if from != name => {
                                    cx.emit(SidebarEvent::RenameTag { from, to: name });
                                }
                                Item::Tag(_) | Item::All => {}
                            }
                        }
                        window.focus(&sidebar.focus, cx);
                        cx.notify();
                    }
                    InputEvent::Blur => {
                        sidebar.rename = None;
                        cx.notify();
                    }
                    InputEvent::Change | InputEvent::Focus => {}
                },
            );
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        self.rename = Some(Rename {
            item,
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if self.rename.is_some() {
            if keystroke.key == "escape" {
                self.rename = None;
                window.focus(&self.focus, cx);
                cx.notify();
                cx.stop_propagation();
            }
            return;
        }
        if modifiers.alt || modifiers.platform {
            return;
        }
        let key = keystroke.key.as_str();
        let handled = match (modifiers.control, modifiers.shift, key) {
            (true, true, "m") => {
                if let Some(target) = self.target() {
                    cx.emit(SidebarEvent::Move(target));
                }
                true
            }
            (false, false, _) => match &self.view {
                View::Tree(_) => self.tree_key(key, window, cx),
                View::List(_) => self.list_key(key, cx),
            },
            _ => false,
        };
        if handled {
            cx.stop_propagation();
        }
    }

    /// The note or folder that `Delete` and Ctrl+Shift+M act on.
    fn target(&self) -> Option<Target> {
        match &self.view {
            View::Tree(Item::Folder(folder)) => Some(Target::Folder(folder.clone())),
            View::Tree(_) => None,
            View::List(_) => self
                .selected_note()
                .map(|at| Target::Note(self.list[at].path.clone())),
        }
    }

    fn tree_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let View::Tree(item) = &self.view else {
            return false;
        };
        let item = item.clone();
        let at = self.rows.iter().position(|row| row.item == item);
        match key {
            "up" | "down" => {
                let next = match (at, key) {
                    (Some(at), "up") => at.saturating_sub(1),
                    (Some(at), _) => (at + 1).min(self.rows.len().saturating_sub(1)),
                    (None, _) => 0,
                };
                if let Some(row) = self.rows.get(next) {
                    let item = row.item.clone();
                    self.set_view(View::Tree(item), cx);
                }
            }
            "right" => {
                if at.is_some_and(|at| self.rows[at].has_children) {
                    self.open.insert(item);
                    self.rebuild();
                    cx.notify();
                }
            }
            "left" => {
                if self.open.remove(&item) {
                    self.rebuild();
                    cx.notify();
                } else if let Some(parent) = item.parent() {
                    self.set_view(View::Tree(parent), cx);
                }
            }
            "enter" => self.set_view(View::List(item), cx),
            "escape" => cx.emit(SidebarEvent::FocusEditor),
            "f2" => self.rename(item, window, cx),
            "delete" => match item {
                Item::Tag(tag) => cx.emit(SidebarEvent::DeleteTag(tag)),
                Item::Folder(folder) => cx.emit(SidebarEvent::Delete(Target::Folder(folder))),
                Item::All => {}
            },
            _ => return false,
        }
        true
    }

    fn list_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        match key {
            "up" | "down" => {
                let next = match (self.selected_note(), key) {
                    (Some(at), "up") => at.checked_sub(1),
                    (Some(at), _) => Some(at + 1),
                    (None, _) => Some(0),
                };
                if let Some(note) = next.and_then(|next| self.list.get(next)) {
                    let path = note.path.clone();
                    self.choose(path, cx);
                }
            }
            "enter" => cx.emit(SidebarEvent::FocusEditor),
            "escape" => {
                let item = self.view.item().clone();
                self.show_in_tree(item, cx);
            }
            "f2" => {
                if self.selected_note().is_some() {
                    cx.emit(SidebarEvent::EditTitle);
                }
            }
            "delete" => {
                if let Some(target) = self.target() {
                    cx.emit(SidebarEvent::Delete(target));
                }
            }
            _ => return false,
        }
        true
    }

    fn choose(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.set_current(path.clone(), cx);
        cx.emit(SidebarEvent::Open(path));
    }

    fn tree_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let entries: Vec<Entry> = range
            .filter_map(|at| self.entries.get(at).copied())
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
                    .child(label)
                    .into_any_element(),
                Entry::Row(at) => self.tree_row(at, cx),
            })
            .collect()
    }

    fn tree_row(&mut self, at: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let row = &self.rows[at];
        let selected = matches!(&self.view, View::Tree(item) if item == &row.item);
        let icon = match &row.item {
            Item::All => Icon::All,
            Item::Folder(_) if row.open => Icon::FolderOpen,
            Item::Folder(_) => Icon::Folder,
            Item::Tag(_) if !row.has_children => Icon::Tag,
            Item::Tag(_) if row.open => Icon::TagsOpen,
            Item::Tag(_) => Icon::Tags,
        };
        let name: SharedString = match &row.item {
            Item::All => text::ALL_NOTES.into(),
            Item::Folder(_) | Item::Tag(_) => row.name.clone().into(),
        };
        let renaming = self
            .rename
            .as_ref()
            .filter(|rename| rename.item == row.item);
        let label = match renaming {
            Some(rename) => Input::new(&rename.input).into_any_element(),
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
                    sidebar.rebuild();
                    cx.notify();
                    cx.stop_propagation();
                }
            }));
        let depth = f32::from(u16::try_from(row.depth).unwrap_or(u16::MAX));
        let item = row.item.clone();
        h_flex()
            .id(ElementId::NamedInteger("tree-row".into(), at as u64))
            .role(Role::TreeItem)
            .aria_label(item_label(&row.item))
            .aria_selected(selected)
            .when(row.has_children, |row_element| {
                row_element.aria_expanded(row.open)
            })
            .test_support()
            .w_full()
            .h(px(TREE_ROW))
            .pr_3()
            .pl(px(12. + 16. * depth))
            .gap_2()
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
                sidebar.set_view(View::List(item.clone()), cx);
            }))
            .into_any_element()
    }

    fn note_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme();
        let selected = self.selected_note();
        range
            .filter_map(|at| self.list.get(at).map(|note| (at, note)))
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
                        sidebar.choose(path.clone(), cx);
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
        Item::All => text::ALL_NOTES.to_owned(),
        Item::Folder(path) => path.display().to_string(),
        Item::Tag(name) => format!("#{name}"),
    }
}

impl EventEmitter<SidebarEvent> for Sidebar {}

impl Focusable for Sidebar {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let header = match &self.view {
            View::Tree(_) => None,
            View::List(item) => {
                let name = match item {
                    Item::All => text::ALL_NOTES.to_owned(),
                    Item::Folder(path) => path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    Item::Tag(name) => format!("#{name}"),
                };
                let item = item.clone();
                Some(
                    h_flex()
                        .id("back")
                        .role(Role::Button)
                        .aria_label(item_label(&item))
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
                                .child(name),
                        )
                        .child(div().text_size(px(12.)).child(self.list.len().to_string()))
                        .on_click(cx.listener(move |sidebar, _: &ClickEvent, window, cx| {
                            window.focus(&sidebar.focus, cx);
                            sidebar.show_in_tree(item.clone(), cx);
                        })),
                )
            }
        };
        let body = match &self.view {
            View::Tree(_) => uniform_list(
                "tree",
                self.entries.len(),
                cx.processor(|sidebar, range, _, cx| sidebar.tree_rows(range, cx)),
            ),
            View::List(_) => uniform_list(
                "notes",
                self.list.len(),
                cx.processor(|sidebar, range, _, cx| sidebar.note_rows(range, cx)),
            ),
        };
        let empty = matches!(self.view, View::List(_)) && self.list.is_empty();
        v_flex()
            .id("sidebar")
            .test_support()
            .key_context("Sidebar")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .size_full()
            .bg(theme.sidebar)
            .children(header)
            .children(self.error.map(|error| {
                div()
                    .px_3()
                    .py_2()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(error)
            }))
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
