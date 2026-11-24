//! One test for each flow of the main window, with a real core on a temp folder.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AnyWindowHandle, AppContext as _, Role, TestAppContext, VisualTestContext, Window};
use holm_core::{Core, Feed};
use tempfile::TempDir;

use super::{HolmWindow, Startup};
use crate::config::Config;
use crate::log::Log;
use crate::{text, theme};

mod app_menu;
mod closing;
mod empty_pane;
mod focus;
mod folder_screen;
mod folders;
mod links;
mod messages;
mod note_title;
mod outside_changes;
mod popups;
mod saving;
mod search;
mod settings;
mod shortcut_list;
mod speed;
mod tags;

/// A write budget that no job outlasts, also on a loaded machine, so a test sees a late job only when it sets
/// [`Duration::ZERO`].
const QUICK_DISK: Duration = Duration::from_hours(1);

struct Fixture {
    dir: TempDir,
    /// The core opens with no file watching. Tests tell it about outside changes here.
    feed: Arc<Mutex<Option<Feed>>>,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("notes")).unwrap();
        Self {
            dir,
            feed: Arc::default(),
        }
    }

    fn write_outside(&self, cx: &mut TestAppContext, note: &str, text: &str) {
        fs::write(self.notes().join(note), text).unwrap();
        self.tell(cx, note);
    }

    fn delete_outside(&self, cx: &mut TestAppContext, note: &str) {
        fs::remove_file(self.notes().join(note)).unwrap();
        self.tell(cx, note);
    }

    fn tell(&self, cx: &mut TestAppContext, note: &str) {
        let feed = self.feed.lock().unwrap();
        feed.as_ref().expect("the core is open").changed([note]).unwrap();
        drop(feed);
        settle(cx);
    }

    fn notes(&self) -> PathBuf {
        self.dir.path().join("notes")
    }

    fn config(&self) -> PathBuf {
        self.dir.path().join("config.toml")
    }

    fn write_config(&self, text: &str) {
        fs::write(self.config(), text).unwrap();
    }

    fn with_folder(self) -> Self {
        self.write_config(&format!(
            "version = 1\nnotes_folder = {:?}\n",
            self.notes().to_str().unwrap()
        ));
        self
    }

    fn startup(&self, crash_log: Option<PathBuf>) -> Startup {
        let feed = self.feed.clone();
        Startup {
            open_core: Arc::new(move |folder, index_dir| {
                let (core, new_feed) = Core::open_with_feed(folder, index_dir)?;
                *feed.lock().unwrap() = Some(new_feed);
                Ok(core)
            }),
            config: Config::load(self.config()),
            index_dir: self.dir.path().join("cache"),
            default_folder: self.dir.path().join("Notes"),
            crashed: crash_log.is_some(),
            log: Log::at(crash_log.unwrap_or_else(|| PathBuf::from("/state/holm/holm.log"))),
            write_budget: QUICK_DISK,
        }
    }
}

/// An inactive window sends no focus events.
fn open(cx: &mut TestAppContext, startup: Startup) -> AnyWindowHandle {
    cx.executor().set_block_on_ticks(usize::MAX..=usize::MAX);
    let opened = cx.update(|cx| {
        gpui_kit::init(cx);
        holm_editor::init(cx);
        crate::shortcuts::bind(cx);
        theme::init(cx);
        gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| HolmWindow::new(startup, window, cx))
        })
    });
    let (handle, _) = opened.unwrap();
    cx.update_window(handle, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    handle
}

/// The autosave does not run yet.
fn settle(cx: &mut TestAppContext) {
    cx.run_until_parked();
}

fn autosave(cx: &mut TestAppContext) {
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
}

fn click(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) {
    cx.update_window(handle, |_, window, cx| window.click(id, cx)).unwrap();
    cx.run_until_parked();
}

fn screen<R>(cx: &mut TestAppContext, handle: AnyWindowHandle, read: impl FnOnce(&mut Window) -> R) -> R {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        read(window)
    })
    .unwrap()
}

/// `None` when the notes are open.
fn folder_screen(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    screen(cx, handle, |window| {
        window
            .try_find("folder-screen")
            .and_then(|screen| screen.label().map(str::to_owned))
    })
}

fn empty_pane(cx: &mut TestAppContext, handle: AnyWindowHandle) -> bool {
    screen(cx, handle, |window| {
        window
            .try_find("empty-pane")
            .is_some_and(|pane| pane.label() == Some(text::NO_NOTE_OPEN))
    })
}

fn files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn editor_text(cx: &mut TestAppContext, handle: AnyWindowHandle) -> String {
    screen(cx, handle, |window| {
        window
            .find("editor")
            .value()
            .expect("the editor shows its text")
            .to_owned()
    })
}

/// On the left of the message line.
fn message(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    label(cx, handle, "message")
}

/// On the right of the message line.
fn state(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    label(cx, handle, "message-state")
}

/// The selected choice has `*`.
fn state_menu(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    labels(cx, handle, "state-menu", Role::ListBoxOption)
}

fn state_reason(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    label(cx, handle, "state-reason")
}

fn label(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) -> Option<String> {
    screen(cx, handle, |window| {
        window.try_find(id).and_then(|part| part.label().map(str::to_owned))
    })
}

/// Each new file in `dir` then fails. `false` undoes it.
fn read_only(dir: &Path, read_only: bool) {
    let mut permissions = fs::metadata(dir).unwrap().permissions();
    permissions.set_readonly(read_only);
    fs::set_permissions(dir, permissions).unwrap();
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

/// As the title bar does. Gives whether the window is still open.
fn close(cx: &mut TestAppContext, handle: AnyWindowHandle) -> bool {
    if VisualTestContext::from_window(handle, cx).simulate_close() {
        cx.update_window(handle, |_, window, _| window.remove_window()).unwrap();
    }
    cx.run_until_parked();
    cx.update_window(handle, |_, _, _| ()).is_ok()
}

/// The keys after it run no autosave, so the text is not saved yet.
fn type_unsaved(cx: &mut TestAppContext, handle: AnyWindowHandle, text: &str) {
    assert_eq!(focus(cx, handle), "editor");
    cx.simulate_input(handle, text);
}

fn set_time(path: &Path, seconds: u64) {
    let time = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(seconds);
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(time)
        .unwrap();
}

fn keys(cx: &mut TestAppContext, handle: AnyWindowHandle, keys: &str) {
    cx.simulate_keystrokes(handle, keys);
    settle(cx);
}

/// What the sidebar shows: "tree <selected item>", "list <item>", or "hidden".
fn sidebar(cx: &mut TestAppContext, handle: AnyWindowHandle) -> String {
    screen(cx, handle, |window| {
        if window.try_find("sidebar").is_none() {
            return "hidden".to_owned();
        }
        if let Some(back) = window.try_find("back") {
            return format!("list {}", back.label().unwrap_or_default());
        }
        let selected = window
            .find_all_by_role(Role::TreeItem)
            .into_iter()
            .find(|row| row.selected() == Some(true));
        format!(
            "tree {}",
            selected
                .and_then(|row| row.label().map(str::to_owned))
                .unwrap_or_default()
        )
    })
}

/// Which part has the focus: "sidebar", "editor", "tag field", "title field", "search", or "other". The tag row lies
/// inside the editor, so it comes first.
fn focus(cx: &mut TestAppContext, handle: AnyWindowHandle) -> &'static str {
    screen(cx, handle, |window| {
        let focused = |id| window.try_find(id).and_then(|part| part.focused()) == Some(true);
        if focused("search") {
            "search"
        } else if focused("sidebar") {
            "sidebar"
        } else if focused("tag-input") {
            "tag field"
        } else if focused("title-input") {
            "title field"
        } else if focused("editor") {
            "editor"
        } else {
            "other"
        }
    })
}

/// The chip or row in focus has `*`.
fn labels(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &str, role: Role) -> Vec<String> {
    let id = id.to_owned();
    screen(cx, handle, |window| {
        if window.try_find(id.clone()).is_none() {
            return Vec::new();
        }
        window
            .within(id)
            .find_all_by_role(role)
            .iter()
            .map(|item| {
                let label = item.label().unwrap_or_default();
                if item.selected() == Some(true) {
                    format!("{label}*")
                } else {
                    label.to_owned()
                }
            })
            .collect()
    })
}

fn chips(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    labels(cx, handle, "tag-row", Role::ListItem)
}

/// A tag of the note has `✓`, the cursor has `*`.
fn options(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    screen(cx, handle, |window| {
        if window.try_find("tag-options").is_none() {
            return Vec::new();
        }
        window
            .within("tag-options")
            .find_all_by_role(Role::ListBoxOption)
            .iter()
            .map(|line| {
                let mut label = line.label().unwrap_or_default().to_owned();
                if line.checked() == Some(true) {
                    label.push_str(" ✓");
                }
                if line.selected() == Some(true) {
                    label.push('*');
                }
                label
            })
            .collect()
    })
}

fn popup_open(cx: &mut TestAppContext, handle: AnyWindowHandle) -> bool {
    screen(cx, handle, |window| window.try_find("tag-popup").is_some())
}

fn fields(path: &Path) -> (Vec<String>, bool) {
    let note = holm_markdown::NoteFile::parse(&read(path));
    (note.tags(), note.pinned())
}

fn modified(path: &Path) -> std::time::SystemTime {
    fs::metadata(path).unwrap().modified().unwrap()
}

fn question(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    screen(cx, handle, |window| {
        window
            .find_all_by_role(Role::Dialog)
            .first()
            .and_then(|dialog| dialog.label().map(str::to_owned))
    })
}

fn click_chip(cx: &mut TestAppContext, handle: AnyWindowHandle, at: u64) {
    cx.update_window(handle, |_, window, cx| {
        window.click(gpui_kit::ElementId::NamedInteger("tag-chip".into(), at), cx);
    })
    .unwrap();
    settle(cx);
}

/// `None` when it is not on the screen.
fn bounds_of(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    id: &'static str,
) -> Option<gpui_kit::Bounds<gpui_kit::Pixels>> {
    screen(cx, handle, |window| window.try_find(id).map(|part| part.bounds()))
}

/// Runs the tasks before the sidebar reads the notes again.
fn keys_now(cx: &mut TestAppContext, handle: AnyWindowHandle, keys: &str) {
    cx.simulate_keystrokes(handle, keys);
    cx.run_until_parked();
}

/// The selected row has `*`.
fn tree(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    labels(cx, handle, "sidebar", Role::TreeItem)
}

fn type_new_name(cx: &mut TestAppContext, handle: AnyWindowHandle, name: &str) {
    keys(cx, handle, "f2");
    keys(cx, handle, "ctrl-a");
    cx.simulate_input(handle, name);
}

/// A write that the key starts has not run yet. Presses `key` and runs no task, so a write that it starts has not run
/// yet.
fn press(cx: &mut TestAppContext, handle: AnyWindowHandle, key: &'static str) {
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_keystroke(gpui_kit::Keystroke::parse(key).unwrap(), cx);
    })
    .unwrap();
}

/// With or without the focus.
fn tree_field(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    screen(cx, handle, |window| {
        window.try_find("sidebar")?;
        window
            .within("sidebar")
            .find_all_by_role(Role::TextInput)
            .into_iter()
            .find(|field| field.label() != Some(text::SEARCH))
            .and_then(|field| field.value().map(str::to_owned))
    })
}

/// A frame can show each of these steps.
fn tree_steps(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<Vec<String>> {
    let mut steps = vec![tree(cx, handle)];
    while cx.executor().tick() {
        let now = tree(cx, handle);
        if steps.last() != Some(&now) {
            steps.push(now);
        }
    }
    steps
}

/// A nested folder `area/work` with an open child `sub` and the note `A.md`, which is open.
fn nested_folders(fixture: &Fixture) {
    let notes_dir = fixture.notes();
    fs::create_dir_all(notes_dir.join("area/work/sub")).unwrap();
    fs::create_dir(notes_dir.join("area/bebra")).unwrap();
    fs::write(notes_dir.join("area/work/A.md"), "# A\n").unwrap();
}

/// While it has the focus.
fn name_field(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    screen(cx, handle, |window| {
        window.try_find("sidebar")?;
        window
            .within("sidebar")
            .find_all_by_role(Role::TextInput)
            .into_iter()
            .find(|field| field.focused() == Some(true))
            .and_then(|field| field.value().map(str::to_owned))
    })
}

/// `None` means it has no child rows.
fn expanded(cx: &mut TestAppContext, handle: AnyWindowHandle, label: &str) -> Option<bool> {
    screen(cx, handle, |window| {
        window
            .within("sidebar")
            .find_all_by_role(Role::TreeItem)
            .into_iter()
            .find(|row| row.label() == Some(label))
            .and_then(|row| row.expanded())
    })
}

/// Three notes: Plan, then Goals with the tag `work`, then Top, in the order of their last change.
fn three_notes(notes_dir: &Path) {
    fs::write(notes_dir.join("Plan.md"), "# Plan\nbuy milk\n").unwrap();
    fs::write(
        notes_dir.join("Goals.md"),
        "---\ntags:\n  - work\n---\n# Goals\nread more\n",
    )
    .unwrap();
    fs::write(notes_dir.join("Top.md"), "# Top\n").unwrap();
    set_time(&notes_dir.join("Plan.md"), 300);
    set_time(&notes_dir.join("Goals.md"), 200);
    set_time(&notes_dir.join("Top.md"), 100);
}

fn notes_list(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    labels(cx, handle, "sidebar", Role::ListItem)
}

/// `None` for an option with no folder.
fn folders(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &str) -> Vec<Option<String>> {
    let id = id.to_owned();
    screen(cx, handle, |window| {
        let popup = window.within(id);
        let labels = popup.find_all_by_role(Role::Label);
        popup
            .find_all_by_role(Role::ListBoxOption)
            .iter()
            .map(|option| {
                labels
                    .iter()
                    .find(|label| label.path().starts_with(option.path()))
                    .and_then(|label| label.label().map(str::to_owned))
            })
            .collect()
    })
}

/// `None` with no button.
fn backlink_count(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    screen(cx, handle, |window| {
        window
            .try_find("backlinks-button")
            .and_then(|button| button.label().map(str::to_owned))
    })
}

/// The cursor has `*`.
fn backlinks(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    labels(cx, handle, "backlinks", Role::ListBoxOption)
}

fn new_note(cx: &mut TestAppContext, handle: AnyWindowHandle, title: &str) {
    keys(cx, handle, "ctrl-n");
    cx.simulate_input(handle, title);
    keys(cx, handle, "enter");
}

/// At the end of the tag row, like `work › Plans`.
fn note_path(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    label(cx, handle, "note-path")
}

/// The new note has no file yet.
fn new_note_open(cx: &mut TestAppContext, handle: AnyWindowHandle) -> bool {
    screen(cx, handle, |window| window.try_find("new-note").is_some())
}

/// The row under the cursor has `*`.
fn app_menu(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    labels(cx, handle, "app-menu", Role::ListBoxOption)
}

fn window_open(cx: &mut TestAppContext, handle: AnyWindowHandle) -> bool {
    cx.update_window(handle, |_, _, _| ()).is_ok()
}
