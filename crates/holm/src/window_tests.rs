//! One test for each flow of the main window, with a real core on a temp folder.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests")]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AnyWindowHandle, AppContext as _, Role, TestAppContext, VisualTestContext, Window};
use holm_core::model::Order;
use holm_core::{Core, Feed};
use holm_editor::WritingHelp;
use tempfile::TempDir;

use super::{HolmWindow, Startup};
use crate::config::{AppearanceSetting, Config, SavedSidebarView, Settings, SidebarItem};
use crate::log::Log;
use crate::{text, theme};

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

    /// Writes `text` to a note, like another tool, and tells the core.
    fn write_outside(&self, cx: &mut TestAppContext, note: &str, text: &str) {
        fs::write(self.notes().join(note), text).unwrap();
        self.tell(cx, note);
    }

    /// Deletes a note, like another tool, and tells the core.
    fn delete_outside(&self, cx: &mut TestAppContext, note: &str) {
        fs::remove_file(self.notes().join(note)).unwrap();
        self.tell(cx, note);
    }

    fn tell(&self, cx: &mut TestAppContext, note: &str) {
        let feed = self.feed.lock().unwrap();
        feed.as_ref()
            .expect("the core is open")
            .changed([note])
            .unwrap();
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

    /// A config file with the notes folder in it.
    fn with_folder(self) -> Self {
        self.write_config(&format!(
            "version = 1\nnotes_folder = {:?}\n",
            self.notes().to_str().unwrap()
        ));
        self
    }

    /// A start with a log file. `crash_log` is that file after a crash.
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
            write_budget: Duration::from_millis(50),
        }
    }
}

/// Opens an active window, as the user sees it. An inactive window sends no focus events.
/// Each write ends within its budget, as a quick disk does.
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

/// Runs all tasks, so the UI sees the outside changes. The autosave does not run yet.
fn settle(cx: &mut TestAppContext) {
    cx.run_until_parked();
}

/// Runs the autosave.
fn autosave(cx: &mut TestAppContext) {
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
}

fn click(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) {
    cx.update_window(handle, |_, window, cx| window.click(id, cx))
        .unwrap();
    cx.run_until_parked();
}

/// Draws a frame and reads the screen with `read`.
fn screen<R>(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    read: impl FnOnce(&mut Window) -> R,
) -> R {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        read(window)
    })
    .unwrap()
}

/// The message of the folder screen, or `None` when the notes are open.
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

/// The files in the notes folder, sorted.
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

/// The text on the left of the message line.
fn message(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    label(cx, handle, "message")
}

/// The state on the right of the message line.
fn state(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    label(cx, handle, "message-state")
}

/// The gray mark with the count of the problems.
fn problem_mark(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    label(cx, handle, "problem-mark")
}

/// The rows of the open problem list. The selected one has `*`.
fn problem_list(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    labels(cx, handle, "problem-list", Role::ListBoxOption)
}

/// The choices of the open state menu. The selected one has `*`.
fn state_menu(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    labels(cx, handle, "state-menu", Role::ListBoxOption)
}

/// The reason line of the open state menu.
fn state_reason(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    label(cx, handle, "state-reason")
}

fn label(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) -> Option<String> {
    screen(cx, handle, |window| {
        window
            .try_find(id)
            .and_then(|part| part.label().map(str::to_owned))
    })
}

/// Makes the notes folder read-only, so each new file fails. `false` undoes it.
fn read_only(dir: &Path, read_only: bool) {
    let mut permissions = fs::metadata(dir).unwrap().permissions();
    permissions.set_readonly(read_only);
    fs::set_permissions(dir, permissions).unwrap();
}

/// The sorted names in the folder `dir`.
fn folder_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

#[gpui_kit::test]
fn the_first_start_asks_for_the_folder_and_keeps_the_pick(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    fs::write(fixture.notes().join("Plans.md"), "# Plans\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    assert_eq!(folder_screen(cx, handle).as_deref(), Some(text::ASK_FOLDER));

    click(cx, handle, "pick-folder");
    assert!(cx.did_prompt_for_paths());
    let notes = fixture.notes();
    cx.simulate_path_prompt_response(|_| Some(vec![notes]));
    cx.run_until_parked();

    assert_eq!(editor_text(cx, handle), "# Plans\n");
    let config = Config::load(fixture.config());
    assert_eq!(config.settings().notes_folder, Some(fixture.notes()));
}

#[gpui_kit::test]
fn the_first_start_can_make_the_default_folder(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let handle = open(cx, fixture.startup(None));

    click(cx, handle, "use-default");

    assert_eq!(folder_screen(cx, handle), None);
    assert!(empty_pane(cx, handle));
    let default = fixture.dir.path().join("Notes");
    assert_eq!(files(&default), Vec::<String>::new());
    let config = Config::load(fixture.config());
    assert_eq!(config.settings().notes_folder, Some(default));
}

#[gpui_kit::test]
fn a_missing_folder_offers_to_pick_another(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::remove_dir(fixture.notes()).unwrap();
    let handle = open(cx, fixture.startup(None));
    assert_eq!(
        folder_screen(cx, handle),
        Some(text::folder_missing(&fixture.notes()).to_string())
    );

    let other = fixture.dir.path().join("other");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("A.md"), "a\n").unwrap();
    click(cx, handle, "pick-folder");
    cx.simulate_path_prompt_response(|_| Some(vec![other]));
    cx.run_until_parked();

    assert_eq!(editor_text(cx, handle), "a\n");
}

#[gpui_kit::test]
fn typing_saves_the_note_with_its_line_breaks(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "---\nid: 01A\n---\r\none\r\ntwo\r\n").unwrap();
    let fixture = fixture.with_folder();
    let handle = open(cx, fixture.startup(None));
    assert_eq!(editor_text(cx, handle), "one\ntwo\n");

    cx.simulate_input(handle, "zero ");
    settle(cx);
    assert!(
        !read(&note).contains("zero"),
        "the autosave waits for a pause"
    );
    autosave(cx);

    assert_eq!(read(&note), "---\nid: 01A\n---\r\nzero one\r\ntwo\r\n");
}

/// A save that waits behind a late job does not write over a change that another tool made
/// meanwhile. It asks the disk question instead.
#[gpui_kit::test]
fn a_save_behind_a_late_job_keeps_an_outside_change(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "old\n").unwrap();
    let startup = Startup {
        write_budget: Duration::ZERO,
        ..fixture.startup(None)
    };
    let handle = open(cx, startup);

    press(cx, handle, "ctrl-shift-p");
    press(cx, handle, "x");
    press(cx, handle, "ctrl-shift-p");
    fs::write(&note, "theirs\n").unwrap();
    let feed = fixture.feed.lock().unwrap();
    feed.as_ref().unwrap().changed(["Note.md"]).unwrap();
    drop(feed);
    cx.run_until_parked();

    assert!(read(&note).ends_with("\n---\ntheirs\n"), "{}", read(&note));
    assert_eq!(state(cx, handle).as_deref(), Some(text::CHANGED_ON_DISK));
    assert_eq!(editor_text(cx, handle), "xold\n");

    keys(cx, handle, "ctrl-shift-d enter");
    assert_eq!(state(cx, handle), None);
    assert!(read(&note).ends_with("\n---\nxold\n"), "{}", read(&note));
}

#[gpui_kit::test]
fn a_late_pin_of_the_open_note_asks_nothing_about_the_disk(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "one\n").unwrap();
    let startup = Startup {
        write_budget: Duration::ZERO,
        ..fixture.startup(None)
    };
    let handle = open(cx, startup);

    press(cx, handle, "ctrl-shift-p");
    press(cx, handle, "x");
    cx.run_until_parked();

    assert!(read(&note).contains("pinned: true"));
    assert_eq!(editor_text(cx, handle), "xone\n");
    assert_eq!(state(cx, handle), None);
}

#[gpui_kit::test]
fn an_outside_change_reloads_quietly_with_no_unsaved_text(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    fixture.write_outside(cx, "Note.md", "new\n");

    assert_eq!(editor_text(cx, handle), "new\n");
    assert_eq!(message(cx, handle), None);
    assert_eq!(state(cx, handle), None);
}

#[gpui_kit::test]
fn an_outside_change_with_unsaved_text_asks_to_reload_or_keep_mine(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    cx.simulate_input(handle, "mine ");
    fixture.write_outside(cx, "Note.md", "theirs\n");
    assert_eq!(state(cx, handle).as_deref(), Some(text::CHANGED_ON_DISK));
    assert_eq!(message(cx, handle), None);
    autosave(cx);
    assert_eq!(read(&note), "theirs\n", "no autosave during the question");

    // The open menu takes every key, and Esc closes only the menu.
    keys(cx, handle, "ctrl-shift-d");
    assert_eq!(state_menu(cx, handle), ["Keep mine*", "Reload"]);
    assert_eq!(
        state_reason(cx, handle).as_deref(),
        Some(text::CHANGED_REASON)
    );
    let text = editor_text(cx, handle);
    keys(cx, handle, "x backspace");
    assert_eq!(editor_text(cx, handle), text);
    keys(cx, handle, "escape");
    assert_eq!(state_menu(cx, handle), Vec::<String>::new());
    assert_eq!(state(cx, handle).as_deref(), Some(text::CHANGED_ON_DISK));
    assert_eq!(focus(cx, handle), "editor");

    // A click outside the menu closes it, and so does a second click on the state.
    keys(cx, handle, "ctrl-shift-d");
    click(cx, handle, "editor");
    assert_eq!(state_menu(cx, handle), Vec::<String>::new());
    assert_eq!(state(cx, handle).as_deref(), Some(text::CHANGED_ON_DISK));
    click(cx, handle, "message-state");
    assert_eq!(state_menu(cx, handle).len(), 2);
    click(cx, handle, "message-state");
    assert_eq!(state_menu(cx, handle), Vec::<String>::new());
    assert_eq!(focus(cx, handle), "editor");

    // The menu is right above the state, and its right edge meets the right edge of the state.
    keys(cx, handle, "ctrl-shift-d");
    let menu = bounds_of(cx, handle, "state-menu").unwrap();
    let opener = bounds_of(cx, handle, "message-state").unwrap();
    // The panel snaps to whole pixels.
    assert!((menu.right() - opener.right()).abs() <= gpui_kit::px(0.5));
    assert!(menu.bottom() <= opener.top());
    keys(cx, handle, "escape");

    keys(cx, handle, "ctrl-shift-d enter");
    assert!(read(&note).ends_with("\n---\nmine old\n"));
    assert_eq!(state(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");

    cx.simulate_input(handle, "x");
    fixture.write_outside(cx, "Note.md", "theirs again\n");
    click(cx, handle, "message-state");
    assert_eq!(state_menu(cx, handle), ["Keep mine*", "Reload"]);
    keys(cx, handle, "down enter");
    assert_eq!(editor_text(cx, handle), "theirs again\n");
    assert_eq!(state(cx, handle), None);
}

#[gpui_kit::test]
fn a_pin_or_a_tag_change_during_a_disk_question_keeps_the_outside_text(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    cx.simulate_input(handle, "mine ");
    fixture.write_outside(cx, "Note.md", "theirs\n");

    keys(cx, handle, "ctrl-shift-p");
    assert!(read(&note).contains("pinned: true"));
    assert!(read(&note).ends_with("\n---\ntheirs\n"));
    assert_eq!(state(cx, handle).as_deref(), Some(text::CHANGED_ON_DISK));

    keys(cx, handle, "ctrl-shift-t");
    cx.simulate_input(handle, "x");
    keys(cx, handle, "enter escape");
    assert_eq!(fields(&note).0, ["x"]);
    assert!(read(&note).ends_with("\n---\ntheirs\n"));
    assert_eq!(state(cx, handle).as_deref(), Some(text::CHANGED_ON_DISK));
    assert_eq!(editor_text(cx, handle), "mine old\n");

    keys(cx, handle, "ctrl-shift-d enter");
    assert_eq!(state(cx, handle), None);
    assert!(read(&note).ends_with("\n---\nmine old\n"));
    assert!(read(&note).contains("pinned: true"));
    assert_eq!(fields(&note).0, ["x"]);
}

#[gpui_kit::test]
fn a_failed_reload_keeps_the_text_and_the_question(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;

    let fixture = Fixture::new().with_folder();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    cx.simulate_input(handle, "mine ");
    fixture.write_outside(cx, "Note.md", "theirs\n");
    fs::set_permissions(&note, fs::Permissions::from_mode(0o000)).unwrap();

    keys(cx, handle, "ctrl-shift-d down enter");
    assert_eq!(editor_text(cx, handle), "mine old\n");
    assert_eq!(state(cx, handle).as_deref(), Some(text::CHANGED_ON_DISK));
    fs::set_permissions(&note, fs::Permissions::from_mode(0o644)).unwrap();
    // The watcher sees the new permissions, so the core knows the new stamp.
    fixture.tell(cx, "Note.md");
    autosave(cx);
    assert_eq!(read(&note), "theirs\n", "the question still holds the text");

    keys(cx, handle, "ctrl-shift-d enter");
    assert_eq!(state(cx, handle), None);
    assert!(read(&note).ends_with("\n---\nmine old\n"));
}

/// The text that a disk question holds is not saved, so the pane stays on its note.
#[gpui_kit::test]
fn a_disk_question_keeps_the_pane_on_its_note(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    three_notes(&notes_dir);
    let plan = notes_dir.join("Plan.md");
    let handle = open(cx, fixture.startup(None));
    cx.simulate_input(handle, "mine ");
    fixture.write_outside(cx, "Plan.md", "theirs\n");

    keys(cx, handle, "ctrl-shift-l down down");
    assert_eq!(editor_text(cx, handle), "mine # Plan\nbuy milk\n");
    assert_eq!(state(cx, handle).as_deref(), Some(text::CHANGED_ON_DISK));
    assert_eq!(read(&plan), "theirs\n");

    keys(cx, handle, "escape");
    assert!(close(cx, handle), "the first close keeps the window");
    assert_eq!(message(cx, handle).as_deref(), Some(text::CLOSE_AGAIN));
    assert_eq!(read(&plan), "theirs\n");

    assert!(!close(cx, handle));
    assert!(
        read(&plan).ends_with("\n---\nmine # Plan\nbuy milk\n"),
        "no text is lost"
    );
}

#[gpui_kit::test]
fn a_broken_config_is_a_problem_in_the_message_line(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    fixture.write_config("version = [broken\n");
    let log = PathBuf::from("/state/holm/holm.log");
    let handle = open(cx, fixture.startup(Some(log.clone())));
    assert_eq!(folder_screen(cx, handle).as_deref(), Some(text::ASK_FOLDER));
    assert_eq!(message(cx, handle), Some(text::crashed(&log).to_string()));
    assert_eq!(problem_mark(cx, handle), Some(text::problem_mark(1)));

    keys(cx, handle, "ctrl-shift-e");
    assert_eq!(
        problem_list(cx, handle),
        [format!("{}*", text::CONFIG_WRITE)]
    );
    keys(cx, handle, "down up");
    assert_eq!(problem_list(cx, handle).len(), 1);
    keys(cx, handle, "escape");
    assert_eq!(problem_list(cx, handle), Vec::<String>::new());
    click(cx, handle, "problem-mark");
    assert_eq!(problem_list(cx, handle).len(), 1);
    click(cx, handle, "problem-mark");
    assert_eq!(problem_list(cx, handle), Vec::<String>::new());

    click(cx, handle, "use-default");
    assert!(empty_pane(cx, handle));
    assert_eq!(read(&fixture.config()), "version = [broken\n");
    assert_eq!(problem_mark(cx, handle), Some(text::problem_mark(1)));
}

#[gpui_kit::test]
fn the_message_line_does_not_move_the_folder_question(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let log = PathBuf::from("/state/holm/holm.log");
    let handle = open(cx, fixture.startup(Some(log.clone())));
    assert_eq!(message(cx, handle), Some(text::crashed(&log).to_string()));
    let with = bounds_of(cx, handle, "folder-question").unwrap();
    let line = bounds_of(cx, handle, "message-line").unwrap();
    let screen = bounds_of(cx, handle, "folder-screen").unwrap();
    assert_eq!(
        line.bottom(),
        screen.bottom(),
        "the line lies over the screen"
    );

    keys(cx, handle, "escape");
    assert_eq!(message(cx, handle), None);
    assert_eq!(bounds_of(cx, handle, "folder-question"), Some(with));
}

#[gpui_kit::test]
fn an_answer_shows_under_the_note_until_the_next_key(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "[gone](holm:01X)\n").unwrap();
    let log = PathBuf::from("/state/holm/holm.log");
    let handle = open(cx, fixture.startup(Some(log.clone())));
    assert_eq!(message(cx, handle), Some(text::crashed(&log).to_string()));
    let (line, editor) = screen(cx, handle, |window| {
        (
            window.find("message-line").bounds(),
            window.find("editor").bounds(),
        )
    });
    assert!(line.top() >= editor.bottom(), "the line is under the note");

    keys(cx, handle, "right");
    assert_eq!(message(cx, handle), None);
    assert!(
        screen(cx, handle, |window| window.try_find("message-line")).is_none(),
        "an empty line does not show"
    );

    keys(cx, handle, "ctrl-enter");
    assert_eq!(message(cx, handle).as_deref(), Some(text::NOTE_GONE));
    keys(cx, handle, "escape");
    assert_eq!(message(cx, handle), None);
}

#[gpui_kit::test]
fn a_failed_save_shows_not_saved_until_a_save_works(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    let note = notes_dir.join("Note.md");
    fs::write(&note, "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    read_only(&notes_dir, true);
    cx.simulate_input(handle, "a");
    autosave(cx);
    assert_eq!(message(cx, handle).as_deref(), Some(text::WRITE));
    assert_eq!(state(cx, handle).as_deref(), Some(text::NOT_SAVED));

    // A second fail does not show the reason again. Ctrl+Shift+D does.
    keys(cx, handle, "right");
    cx.simulate_input(handle, "b");
    autosave(cx);
    assert_eq!(message(cx, handle), None);
    assert_eq!(state(cx, handle).as_deref(), Some(text::NOT_SAVED));
    keys(cx, handle, "ctrl-shift-d");
    assert_eq!(state_reason(cx, handle).as_deref(), Some(text::WRITE));
    assert_eq!(message(cx, handle), None);
    assert_eq!(
        editor_text(cx, handle),
        "aobld\n",
        "Ctrl+Shift+D types nothing"
    );

    // A disk question wins over "Not saved", and its state closes the menu.
    fixture.write_outside(cx, "Note.md", "theirs\n");
    assert_eq!(state(cx, handle).as_deref(), Some(text::CHANGED_ON_DISK));
    assert_eq!(state_reason(cx, handle), None);

    read_only(&notes_dir, false);
    keys(cx, handle, "ctrl-shift-d enter");
    assert_eq!(state(cx, handle), None);
    assert!(read(&note).ends_with("aobld\n"));
}

/// Asks to close the window, as the title bar does, and gives whether it is still open.
fn close(cx: &mut TestAppContext, handle: AnyWindowHandle) -> bool {
    if VisualTestContext::from_window(handle, cx).simulate_close() {
        cx.update_window(handle, |_, window, _| window.remove_window())
            .unwrap();
    }
    cx.run_until_parked();
    cx.update_window(handle, |_, _, _| ()).is_ok()
}

#[gpui_kit::test]
fn a_close_with_a_saved_note_quits_at_once(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    cx.simulate_input(handle, "a");
    autosave(cx);
    assert!(!close(cx, handle));
}

#[gpui_kit::test]
fn a_close_saves_the_typed_text_first(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    cx.simulate_input(handle, "a");
    assert!(!close(cx, handle));
    assert!(read(&note).ends_with("aold\n"));
}

/// Types `text` in the editor. The keys after it run no autosave, so the text is not saved yet.
fn type_unsaved(cx: &mut TestAppContext, handle: AnyWindowHandle, text: &str) {
    assert_eq!(focus(cx, handle), "editor");
    cx.simulate_input(handle, text);
}

#[gpui_kit::test]
fn each_pane_action_sees_the_typed_text(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("A.md"), "# A\n").unwrap();
    fs::write(notes.join("B.md"), "# B\n").unwrap();
    set_time(&notes.join("A.md"), 200);
    set_time(&notes.join("B.md"), 100);
    let handle = open(cx, fixture.startup(None));
    let alpha = notes.join("Alpha.md");

    type_unsaved(cx, handle, "one ");
    keys_now(cx, handle, "f2");
    cx.simulate_input(handle, "Alpha");
    keys_now(cx, handle, "enter");
    assert!(read(&alpha).ends_with("one # A\n"), "the rename");

    type_unsaved(cx, handle, "two ");
    keys_now(cx, handle, "ctrl-n");
    assert!(new_note_open(cx, handle));
    assert!(read(&alpha).ends_with("one two # A\n"), "Ctrl+N");
    keys_now(cx, handle, "escape");

    // The link picker finds the heading that is not saved yet.
    keys_now(cx, handle, "ctrl-end");
    type_unsaved(cx, handle, "## Fresh");
    keys_now(cx, handle, "ctrl-k");
    cx.simulate_input(handle, "#fr");
    cx.run_until_parked();
    let options = labels(cx, handle, "link-picker", Role::ListBoxOption);
    assert_eq!(options, ["Fresh*"], "Ctrl+K");
    assert!(read(&alpha).ends_with("## Fresh\n"));
    keys_now(cx, handle, "escape");

    type_unsaved(cx, handle, " three");
    keys_now(cx, handle, "ctrl-k");
    cx.simulate_input(handle, "B");
    cx.run_until_parked();
    keys_now(cx, handle, "enter");
    assert!(read(&alpha).ends_with("## Fresh three\n"), "a link");

    type_unsaved(cx, handle, " four");
    keys_now(cx, handle, "ctrl-shift-o");
    cx.simulate_input(handle, "B");
    cx.run_until_parked();
    keys_now(cx, handle, "enter");
    assert_eq!(editor_text(cx, handle), "# B\n");
    assert!(read(&alpha).ends_with(" four\n"), "an open of another note");
}

#[gpui_kit::test]
fn each_sidebar_action_sees_the_typed_text(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::create_dir_all(notes.join("attachments")).unwrap();
    fs::write(notes.join("attachments/x.png"), "png").unwrap();
    fs::create_dir(notes.join("work")).unwrap();
    fs::write(notes.join("work/A.md"), "---\ntags:\n  - old\n---\n# A\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    keys_now(cx, handle, "ctrl-end");

    type_unsaved(cx, handle, "one");
    keys_now(cx, handle, "ctrl-shift-l escape down");
    assert_eq!(tree(cx, handle), ["All notes", "work*", "#old"]);
    keys_now(cx, handle, "f2 ctrl-a");
    cx.simulate_input(handle, "area");
    keys_now(cx, handle, "enter");
    let note = notes.join("area/A.md");
    assert!(read(&note).ends_with("# A\none\n"), "a folder rename");

    keys_now(cx, handle, "escape");
    type_unsaved(cx, handle, " two");
    keys_now(cx, handle, "ctrl-shift-n");
    cx.simulate_input(handle, "new");
    keys_now(cx, handle, "enter");
    assert!(read(&note).ends_with(" two\n"), "a new folder");

    keys_now(cx, handle, "escape");
    type_unsaved(cx, handle, " three");
    keys_now(cx, handle, "ctrl-shift-l down");
    assert_eq!(tree(cx, handle), ["All notes", "area", "new", "#old*"]);
    keys_now(cx, handle, "f2 ctrl-a");
    cx.simulate_input(handle, "job");
    keys_now(cx, handle, "enter");
    assert!(
        read(&note).ends_with(" three\n"),
        "the count of a tag rename"
    );
    keys_now(cx, handle, "enter");
    assert_eq!(fields(&note).0, ["job"]);

    keys_now(cx, handle, "escape");
    type_unsaved(cx, handle, " ![](../attachments/x.png)");
    keys_now(cx, handle, "ctrl-shift-l up up enter");
    assert_eq!(sidebar(cx, handle), "list area");
    keys_now(cx, handle, "delete");
    assert_eq!(
        question(cx, handle).as_deref(),
        Some("Delete “A”? This deletes 1 note and 1 file forever."),
        "the count of a delete"
    );
    keys_now(cx, handle, "escape");

    keys_now(cx, handle, "enter");
    type_unsaved(cx, handle, " four");
    keys_now(cx, handle, "ctrl-shift-l ctrl-shift-m enter");
    let note = notes.join("A.md");
    assert!(read(&note).ends_with(" four\n"), "a note move");

    keys_now(cx, handle, "enter");
    type_unsaved(cx, handle, " five");
    keys_now(cx, handle, "ctrl-shift-l escape down down down");
    assert_eq!(sidebar(cx, handle), "tree #job");
    keys_now(cx, handle, "delete");
    assert!(
        read(&note).ends_with(" five\n"),
        "the count of a tag delete"
    );
    keys_now(cx, handle, "enter");
    assert_eq!(fields(&note).0, Vec::<String>::new());
}

/// A click on the state between the two closes does not stop the second close.
#[gpui_kit::test]
fn a_click_between_two_closes_still_lets_the_second_close_close(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("Note.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    read_only(&notes_dir, true);
    cx.simulate_input(handle, "a");
    assert!(close(cx, handle));
    assert_eq!(message(cx, handle).as_deref(), Some(text::CLOSE_AGAIN));
    click(cx, handle, "message-state");
    assert_eq!(state_reason(cx, handle).as_deref(), Some(text::WRITE));

    assert!(!close(cx, handle), "the second close closes");
    read_only(&notes_dir, false);
}

#[gpui_kit::test]
fn ctrl_shift_d_on_not_saved_shows_its_reason(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("Note.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-d");
    assert_eq!(state_reason(cx, handle), None, "no state, no menu");

    read_only(&notes_dir, true);
    cx.simulate_input(handle, "a");
    autosave(cx);
    assert_eq!(state(cx, handle).as_deref(), Some(text::NOT_SAVED));
    keys(cx, handle, "ctrl-shift-d");
    assert_eq!(state_reason(cx, handle).as_deref(), Some(text::WRITE));
    assert_eq!(state_menu(cx, handle), Vec::<String>::new());
    assert_eq!(message(cx, handle), None);
    keys(cx, handle, "enter");
    assert_eq!(state_reason(cx, handle).as_deref(), Some(text::WRITE));
    keys(cx, handle, "escape");
    assert_eq!(state_reason(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");
    read_only(&notes_dir, false);
}

#[gpui_kit::test]
fn a_close_with_a_note_that_cannot_be_saved_asks_for_a_second_close(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("Note.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    read_only(&notes_dir, true);
    cx.simulate_input(handle, "a");
    assert!(close(cx, handle));
    assert_eq!(message(cx, handle).as_deref(), Some(text::CLOSE_AGAIN));
    assert_eq!(state(cx, handle).as_deref(), Some(text::NOT_SAVED));

    assert!(!close(cx, handle), "the second close closes");
    read_only(&notes_dir, false);
}

/// A key that changes no text keeps the second close.
#[gpui_kit::test]
fn a_failed_close_then_an_arrow_key_then_a_close_quits(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("Note.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    read_only(&notes_dir, true);
    cx.simulate_input(handle, "a");
    assert!(close(cx, handle));
    keys(cx, handle, "right");
    assert_eq!(message(cx, handle), None);
    assert!(!close(cx, handle));
    read_only(&notes_dir, false);
}

/// Typed text after a failed close stops the second close, so the text is never lost.
#[gpui_kit::test]
fn a_failed_close_then_a_typed_letter_then_a_close_does_not_quit(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("Note.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    read_only(&notes_dir, true);
    cx.simulate_input(handle, "a");
    assert!(close(cx, handle));
    cx.simulate_input(handle, "b");
    assert!(close(cx, handle), "the close tries to save again");
    assert_eq!(message(cx, handle).as_deref(), Some(text::CLOSE_AGAIN));

    assert!(!close(cx, handle));
    read_only(&notes_dir, false);
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

/// Which part has the focus: "sidebar", "editor", "tag field", "title field", "search", or
/// "other". The tag row lies inside the editor, so it comes first.
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

/// The labels of the elements with `role` in the part `id`. The chip or row in focus has `*`.
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

/// The lines of the list in the tag popup. A tag of the note has `✓`, the cursor has `*`.
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

/// The tags and the pin in the file of a note.
fn fields(path: &Path) -> (Vec<String>, bool) {
    let note = holm_markdown::NoteFile::parse(&read(path));
    (note.tags(), note.pinned())
}

fn modified(path: &Path) -> std::time::SystemTime {
    fs::metadata(path).unwrap().modified().unwrap()
}

/// The question of the open dialog.
fn question(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    screen(cx, handle, |window| {
        window
            .find_all_by_role(Role::Dialog)
            .first()
            .and_then(|dialog| dialog.label().map(str::to_owned))
    })
}

#[gpui_kit::test]
fn the_delete_dialog_counts_and_deletes(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::create_dir(notes_dir.join("attachments")).unwrap();
    fs::write(notes_dir.join("attachments/x.png"), "png").unwrap();
    fs::write(notes_dir.join("A.md"), "# A\n![](attachments/x.png)\n").unwrap();
    fs::write(notes_dir.join("B.md"), "# B\n").unwrap();
    set_time(&notes_dir.join("A.md"), 200);
    set_time(&notes_dir.join("B.md"), 100);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(editor_text(cx, handle), "# A\n![](attachments/x.png)\n");

    keys(cx, handle, "ctrl-shift-l delete");
    assert_eq!(
        question(cx, handle).as_deref(),
        Some("Delete “A”? This deletes 1 note and 1 file forever.")
    );
    keys(cx, handle, "escape");
    assert_eq!(question(cx, handle), None);
    assert!(notes_dir.join("A.md").is_file());
    assert_eq!(focus(cx, handle), "sidebar");

    // The dialog opens after the count, so the keys go one step at a time.
    keys(cx, handle, "delete");
    keys(cx, handle, "space");
    let checked = screen(cx, handle, |window| window.find("do-not-ask").checked());
    assert_eq!(checked, Some(true));
    keys(cx, handle, "enter");
    assert_eq!(question(cx, handle), None);
    assert!(!notes_dir.join("A.md").exists());
    assert!(!notes_dir.join("attachments/x.png").exists());
    assert_eq!(editor_text(cx, handle), "# B\n");
    assert!(!Config::load(fixture.config()).settings().ask_before_delete);

    keys(cx, handle, "delete");
    assert_eq!(
        question(cx, handle),
        None,
        "no dialog after Do not show again"
    );
    assert!(!notes_dir.join("B.md").exists());
    assert!(empty_pane(cx, handle));
    assert_eq!(files(&notes_dir), ["attachments"]);
    assert_eq!(focus(cx, handle), "sidebar");
    assert_eq!(Config::load(fixture.config()).settings().last_note, None);
}

#[gpui_kit::test]
fn the_keyboard_moves_through_tree_list_and_editor(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::create_dir(notes_dir.join("work")).unwrap();
    fs::write(notes_dir.join("work/Plan.md"), "# Plan\n").unwrap();
    fs::write(notes_dir.join("Top.md"), "# Top\n").unwrap();
    set_time(&notes_dir.join("work/Plan.md"), 100);
    set_time(&notes_dir.join("Top.md"), 200);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(sidebar(cx, handle), "list All notes");
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "ctrl-shift-l");
    assert_eq!(focus(cx, handle), "sidebar");

    keys(cx, handle, "escape down enter");
    assert_eq!(sidebar(cx, handle), "list work");

    keys(cx, handle, "down");
    assert_eq!(editor_text(cx, handle), "# Plan\n");
    keys(cx, handle, "f2");
    assert_eq!(focus(cx, handle), "title field");
    cx.simulate_input(handle, "Goals");
    keys(cx, handle, "enter");
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "ctrl-shift-l");
    assert_eq!(focus(cx, handle), "sidebar");
    keys(cx, handle, "escape");
    assert_eq!(sidebar(cx, handle), "tree work");
    // The rename keeps the time of the note, so it stays below "Top".
    keys(cx, handle, "up enter up");
    assert_eq!(editor_text(cx, handle), "# Top\n");
    assert_eq!(read(&notes_dir.join("work/Goals.md")), "# Plan\n");
    assert!(!notes_dir.join("work/Plan.md").exists());

    keys(cx, handle, "enter");
    assert_eq!(focus(cx, handle), "editor");
    keys(cx, handle, "ctrl-shift-l ctrl-shift-l");
    assert_eq!(sidebar(cx, handle), "hidden");
    assert_eq!(focus(cx, handle), "editor");
    keys(cx, handle, "ctrl-shift-l");
    assert_eq!(sidebar(cx, handle), "list All notes");
    assert_eq!(focus(cx, handle), "sidebar");

    let settings = Config::load(fixture.config()).settings().clone();
    assert_eq!(
        settings.sidebar.view,
        SavedSidebarView {
            list: true,
            item: SidebarItem::All,
        }
    );
    assert_eq!(settings.last_note, holm_core::NotePath::new("Top.md"));
}

#[gpui_kit::test]
fn tab_and_shift_tab_stay_in_the_editor(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("A.md"), "- a\n- b\n\nend").unwrap();
    let handle = open(cx, fixture.startup(None));
    assert_eq!(sidebar(cx, handle), "list All notes");
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "down tab");
    assert_eq!(editor_text(cx, handle), "- a\n  - b\n\nend");
    assert_eq!(focus(cx, handle), "editor");
    keys(cx, handle, "shift-tab");
    assert_eq!(editor_text(cx, handle), "- a\n- b\n\nend");
    assert_eq!(focus(cx, handle), "editor");

    // Outside a list, Tab types spaces and Shift+Tab does nothing.
    keys(cx, handle, "ctrl-end tab");
    assert_eq!(editor_text(cx, handle), "- a\n- b\n\nend    ");
    assert_eq!(focus(cx, handle), "editor");
    keys(cx, handle, "shift-tab");
    assert_eq!(editor_text(cx, handle), "- a\n- b\n\nend    ");
    assert_eq!(focus(cx, handle), "editor");
}

#[gpui_kit::test]
fn keys_make_rename_and_move_notes_and_folders(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("Top.md"), "# Top\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-n");
    assert_eq!(tree(cx, handle), ["All notes*", text::NEW_FOLDER]);
    assert_eq!(tree_field(cx, handle).as_deref(), Some(""));
    assert_eq!(folder_names(&notes_dir), ["Top.md"]);
    cx.simulate_input(handle, "work");
    keys(cx, handle, "enter");
    assert!(notes_dir.join("work").is_dir());
    assert_eq!(sidebar(cx, handle), "tree work");
    assert_eq!(tree(cx, handle), ["All notes", "work*"]);

    keys(cx, handle, "enter ctrl-n");
    cx.simulate_input(handle, "Plans");
    keys(cx, handle, "enter");
    assert!(notes_dir.join("work/Plans.md").is_file());
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "ctrl-shift-l escape up enter down");
    assert_eq!(editor_text(cx, handle), "# Top\n");
    keys(cx, handle, "ctrl-shift-m");
    keys(cx, handle, "down enter");
    assert!(notes_dir.join("work/Top.md").is_file());
    assert!(!notes_dir.join("Top.md").exists());

    // The editor follows the move, so the next save goes to the new file.
    keys(cx, handle, "enter");
    cx.simulate_input(handle, "x");
    autosave(cx);
    assert!(read(&notes_dir.join("work/Top.md")).contains('x'));
    assert!(!notes_dir.join("Top.md").exists());
}

/// Opens a notes folder with the folder `work`, and selects it in the tree.
fn open_on_work(cx: &mut TestAppContext) -> AnyWindowHandle {
    let fixture = Fixture::new().with_folder();
    fs::create_dir(fixture.notes().join("work")).unwrap();
    fs::write(fixture.notes().join("Top.md"), "# Top\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l escape down");
    assert_eq!(sidebar(cx, handle), "tree work");
    handle
}

#[gpui_kit::test]
fn space_types_a_space_in_the_rename_field_of_the_tree(cx: &mut TestAppContext) {
    let handle = open_on_work(cx);
    // Ticket 155 gives Space to the tree. This binding stands for its row.
    cx.update(|cx| {
        let bindings = holm_editor::shortcuts::Keys::plain("space").bindings(
            crate::shortcuts::Place::Tree.key_context(),
            |_| crate::shortcuts::Run(crate::shortcuts::Command::ShowNotes),
            cx,
        );
        cx.bind_keys(bindings);
    });
    keys(cx, handle, "f2 end space");
    assert_eq!(tree_field(cx, handle).as_deref(), Some("work "));
    assert_eq!(sidebar(cx, handle), "tree work");
}

#[gpui_kit::test]
fn space_types_a_space_in_an_input_inside_a_place_that_binds_it(cx: &mut TestAppContext) {
    let handle = open_on_work(cx);
    // A Space row of the notes view would take the space from the search field.
    cx.update(|cx| {
        let bindings = holm_editor::shortcuts::Keys::plain("space").bindings(
            crate::shortcuts::Place::Window.key_context(),
            |_| crate::shortcuts::Run(crate::shortcuts::Command::NewNote),
            cx,
        );
        cx.bind_keys(bindings);
    });
    keys(cx, handle, "ctrl-shift-f a space b");
    let search = screen(cx, handle, |window| {
        window
            .try_find("search")
            .and_then(|field| field.value().map(str::to_owned))
    });
    assert_eq!(search.as_deref(), Some("a b"));
}

#[gpui_kit::test]
fn tab_in_the_rename_field_leaves_the_tree_cursor(cx: &mut TestAppContext) {
    let handle = open_on_work(cx);
    keys(cx, handle, "f2 tab shift-tab");
    assert_eq!(tree_field(cx, handle).as_deref(), Some("work"));
    assert_eq!(sidebar(cx, handle), "tree work");
    keys(cx, handle, "end x");
    assert_eq!(
        tree_field(cx, handle).as_deref(),
        Some("workx"),
        "the field keeps the focus"
    );
}

#[gpui_kit::test]
fn tab_where_no_place_names_it_leaves_the_focus(cx: &mut TestAppContext) {
    let handle = open_on_work(cx);
    assert_eq!(focus(cx, handle), "sidebar");
    keys(cx, handle, "tab");
    assert_eq!(focus(cx, handle), "sidebar");
    keys(cx, handle, "shift-tab");
    assert_eq!(focus(cx, handle), "sidebar");
    assert_eq!(sidebar(cx, handle), "tree work");
}

#[gpui_kit::test]
fn the_tag_popup_adds_and_removes_tags_and_pins(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    let a = notes_dir.join("A.md");
    fs::write(&a, "---\ntags:\n  - work\n  - home\n---\n# A\n").unwrap();
    fs::write(
        notes_dir.join("B.md"),
        "---\ntags:\n  - project/holm\n  - holiday\n---\n# B\n",
    )
    .unwrap();
    set_time(&a, 200);
    set_time(&notes_dir.join("B.md"), 100);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(editor_text(cx, handle), "# A\n");
    assert_eq!(chips(cx, handle), ["work", "home"]);
    assert!(!popup_open(cx, handle));

    keys(cx, handle, "ctrl-shift-t");
    assert!(popup_open(cx, handle));
    assert_eq!(focus(cx, handle), "tag field");
    assert_eq!(options(cx, handle), ["#work ✓", "#home ✓"]);
    cx.simulate_input(handle, "ho");
    settle(cx);
    assert_eq!(
        options(cx, handle),
        ["#home ✓", "#holiday", "#project/holm", "Add #ho"]
    );
    keys(cx, handle, "down down enter");
    assert_eq!(chips(cx, handle), ["work", "home", "holiday"]);
    cx.simulate_input(handle, "big plans");
    keys(cx, handle, "enter");
    assert_eq!(chips(cx, handle), ["work", "home", "holiday", "big-plans"]);
    assert_eq!(fields(&a).0, ["work", "home", "holiday", "big-plans"]);

    keys(cx, handle, "down down enter");
    assert_eq!(
        options(cx, handle),
        ["#work ✓", "#holiday ✓*", "#big-plans ✓"]
    );
    assert_eq!(chips(cx, handle), ["work", "holiday", "big-plans"]);

    cx.update_window(handle, |_, window, cx| {
        window.click(
            gpui_kit::ElementId::NamedInteger("tag-option".into(), 2),
            cx,
        );
    })
    .unwrap();
    settle(cx);
    assert_eq!(chips(cx, handle), ["work", "holiday"]);
    assert_eq!(options(cx, handle), ["#work ✓", "#holiday ✓*"]);
    assert_eq!(focus(cx, handle), "tag field");
    assert_eq!(fields(&a).0, ["work", "holiday"]);

    keys(cx, handle, "escape");
    assert_eq!(focus(cx, handle), "editor");
    assert!(!popup_open(cx, handle));

    keys(cx, handle, "ctrl-shift-p");
    assert!(screen(cx, handle, |window| window
        .try_find("pin-mark")
        .is_some()));
    assert_eq!(
        fields(&a),
        (vec!["work".to_owned(), "holiday".to_owned()], true)
    );

    keys(cx, handle, "ctrl-z");
    autosave(cx);
    assert_eq!(
        fields(&a),
        (vec!["work".to_owned(), "holiday".to_owned()], true)
    );
    assert_eq!(
        modified(&a),
        std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(200),
        "tags and pins keep the modification time"
    );

    keys(cx, handle, "up");
    assert_eq!(focus(cx, handle), "tag field");
    keys(cx, handle, "escape");
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(editor_text(cx, handle), "# A\n");
}

/// Whether the tag row is on the screen, with its top inside the window.
fn tag_row_in_sight(cx: &mut TestAppContext, handle: AnyWindowHandle) -> bool {
    screen(cx, handle, |window| {
        window
            .try_find("tag-row")
            .is_some_and(|row| row.visible() && row.bounds().top() >= gpui_kit::px(0.))
    })
}

#[gpui_kit::test]
fn the_tag_row_scrolls_with_the_text_and_ctrl_shift_t_brings_it_back(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let lines: Vec<String> = (0..200).map(|line| format!("line {line}")).collect();
    fs::write(
        fixture.notes().join("Long.md"),
        format!(
            "---\ntags:\n  - work\n  - {}\n---\n# Long\n{}\n",
            "a".repeat(300),
            lines.join("\n")
        ),
    )
    .unwrap();
    let handle = open(cx, fixture.startup(None));
    assert!(tag_row_in_sight(cx, handle));
    let widest = screen(cx, handle, |window| {
        window
            .within("tag-row")
            .find_all_by_role(Role::ListItem)
            .iter()
            .map(|chip| chip.bounds().size.width)
            .fold(gpui_kit::px(0.), gpui_kit::Pixels::max)
    });
    assert!(widest <= crate::tags::CHIP_WIDTH, "a long tag ends with …");

    // The popup opens with no move of the row or its chips.
    let row_shape = |cx: &mut TestAppContext| {
        screen(cx, handle, |window| {
            let mut shape = vec![window.find("tag-row").bounds()];
            shape.extend(
                window
                    .within("tag-row")
                    .find_all_by_role(Role::ListItem)
                    .iter()
                    .map(gpui_kit::test::ElementSnapshot::bounds),
            );
            shape
        })
    };
    let closed = row_shape(cx);
    keys(cx, handle, "ctrl-shift-t");
    assert!(popup_open(cx, handle));
    assert_eq!(row_shape(cx), closed);
    cx.simulate_input(handle, "x");
    settle(cx);
    assert_eq!(row_shape(cx), closed);
    keys(cx, handle, "escape");

    keys(cx, handle, "ctrl-end");
    assert!(!tag_row_in_sight(cx, handle));

    keys(cx, handle, "ctrl-shift-t");
    assert!(tag_row_in_sight(cx, handle));
    assert_eq!(focus(cx, handle), "tag field");
    assert_eq!(chips(cx, handle), ["work".to_owned(), "a".repeat(300)]);

    keys(cx, handle, "escape");
    assert_eq!(focus(cx, handle), "editor");
    assert!(!tag_row_in_sight(cx, handle));
    cx.simulate_input(handle, "end");
    settle(cx);
    assert!(editor_text(cx, handle).ends_with("line 199\nend"));
}

/// Clicks the chip at `at` in the tag row.
fn click_chip(cx: &mut TestAppContext, handle: AnyWindowHandle, at: u64) {
    cx.update_window(handle, |_, window, cx| {
        window.click(gpui_kit::ElementId::NamedInteger("tag-chip".into(), at), cx);
    })
    .unwrap();
    settle(cx);
}

#[gpui_kit::test]
fn clicks_open_and_close_the_tag_popup(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(
        fixture.notes().join("A.md"),
        "---\ntags:\n  - work\n  - home\n---\n# A\n",
    )
    .unwrap();
    let handle = open(cx, fixture.startup(None));

    // A click on a chip opens the popup with the cursor on its tag. A click elsewhere in the
    // row opens it with no cursor.
    click_chip(cx, handle, 0);
    assert_eq!(focus(cx, handle), "tag field");
    assert_eq!(options(cx, handle), ["#work ✓*", "#home ✓"]);
    click_chip(cx, handle, 1);
    assert!(
        popup_open(cx, handle),
        "a click on the row keeps the popup open"
    );
    assert_eq!(options(cx, handle), ["#work ✓", "#home ✓*"]);
    keys(cx, handle, "escape");
    let right = screen(cx, handle, |window| {
        window.find("tag-row").bounds().size.width
    });
    cx.update_window(handle, |_, window, cx| {
        window.click_at(
            "tag-row",
            gpui_kit::point(right / 2., gpui_kit::px(16.)),
            cx,
        );
    })
    .unwrap();
    settle(cx);
    assert_eq!(focus(cx, handle), "tag field");
    assert_eq!(options(cx, handle), ["#work ✓", "#home ✓"]);

    // A click outside the popup closes it, as Esc does.
    let bottom = screen(cx, handle, |window| {
        window.find("editor").bounds().size.height
    });
    cx.update_window(handle, |_, window, cx| {
        window.click_at(
            "editor",
            gpui_kit::point(gpui_kit::px(200.), bottom - gpui_kit::px(20.)),
            cx,
        );
    })
    .unwrap();
    settle(cx);
    assert!(!popup_open(cx, handle));
    assert_eq!(focus(cx, handle), "editor");
}

#[gpui_kit::test]
fn a_note_with_no_tags_shows_the_add_chip(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let a = fixture.notes().join("A.md");
    fs::write(&a, "# A\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    let row = bounds_of(cx, handle, "tag-row").unwrap();
    assert!(row.size.height > gpui_kit::px(0.));
    assert!(bounds_of(cx, handle, "tag-add").is_some());
    assert_eq!(chips(cx, handle), Vec::<String>::new());
    let popup_under_row =
        |cx: &mut TestAppContext| bounds_of(cx, handle, "tag-popup").unwrap().top() >= row.bottom();

    // A click on the chip opens the popup under the row, not over the title.
    click(cx, handle, "tag-add");
    assert_eq!(focus(cx, handle), "tag field");
    assert!(popup_under_row(cx));
    keys(cx, handle, "escape");
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "ctrl-shift-t");
    assert_eq!(focus(cx, handle), "tag field");
    assert!(popup_under_row(cx));

    cx.simulate_input(handle, "work");
    keys(cx, handle, "enter");
    assert_eq!(chips(cx, handle), ["work"]);
    assert_eq!(bounds_of(cx, handle, "tag-add"), None);
    assert_eq!(bounds_of(cx, handle, "tag-row").unwrap(), row);
    assert_eq!(fields(&a).0, ["work"]);
}

/// The bounds of the part `id`, or `None` when it is not on the screen.
fn bounds_of(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    id: &'static str,
) -> Option<gpui_kit::Bounds<gpui_kit::Pixels>> {
    screen(cx, handle, |window| {
        window.try_find(id).map(|part| part.bounds())
    })
}

#[gpui_kit::test]
fn the_tag_row_stays_on_one_line_and_folds_the_rest_into_a_chip(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(
        notes.join("A.md"),
        "---\nid: 01A\ntags:\n  - work\n---\n# A\n",
    )
    .unwrap();
    fs::write(notes.join("B.md"), "# B\n[a](holm:01A)\n").unwrap();
    set_time(&notes.join("A.md"), 200);
    set_time(&notes.join("B.md"), 100);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(editor_text(cx, handle), "# A\n");
    assert_eq!(chips(cx, handle), ["work"]);
    assert_eq!(bounds_of(cx, handle, "tag-more"), None);
    let one_tag = bounds_of(cx, handle, "tag-row").unwrap().size.height;

    let tags: Vec<String> = (0..12)
        .map(|at| format!("a-long-tag-number-{at:02}-with-words"))
        .collect();
    let lines = tags.join("\n  - ");
    fixture.write_outside(
        cx,
        "A.md",
        &format!("---\nid: 01A\ntags:\n  - {lines}\n---\n# A\n"),
    );
    let shown = chips(cx, handle);
    assert!(!shown.is_empty() && shown.len() < tags.len(), "{shown:?}");
    assert_eq!(shown, tags[..shown.len()]);
    let row = bounds_of(cx, handle, "tag-row").unwrap();
    assert_eq!(row.size.height, one_tag);
    let count = bounds_of(cx, handle, "backlinks-button").unwrap();
    let more = bounds_of(cx, handle, "tag-more").unwrap();
    assert_eq!(
        count.center().y,
        more.center().y,
        "the count is on the line of the chips"
    );
    assert!(more.right() <= count.left());
    assert!(count.right() <= row.right());

    // A narrow window folds more tags.
    cx.simulate_window_resize(
        handle,
        gpui_kit::size(gpui_kit::px(560.), gpui_kit::px(600.)),
    );
    cx.run_until_parked();
    let narrow = chips(cx, handle);
    assert!(narrow.len() < shown.len(), "{narrow:?}");
    assert_eq!(
        bounds_of(cx, handle, "tag-row").unwrap().size.height,
        one_tag
    );

    // In a window too narrow for any chip, the count still does not cover the "+N" chip.
    cx.simulate_window_resize(
        handle,
        gpui_kit::size(gpui_kit::px(100.), gpui_kit::px(600.)),
    );
    cx.run_until_parked();
    assert_eq!(chips(cx, handle), Vec::<String>::new());
    let more = bounds_of(cx, handle, "tag-more").unwrap();
    let count = bounds_of(cx, handle, "backlinks-button").unwrap();
    assert!(more.right() <= count.left(), "{more:?} {count:?}");
    cx.simulate_window_resize(
        handle,
        gpui_kit::size(gpui_kit::px(560.), gpui_kit::px(600.)),
    );
    cx.run_until_parked();

    // A click on the "+N" chip puts the cursor on the first folded tag.
    click(cx, handle, "tag-more");
    assert_eq!(focus(cx, handle), "tag field");
    let cursor = options(cx, handle)
        .iter()
        .position(|line| line.ends_with('*'));
    assert_eq!(cursor, Some(narrow.len()));
}

#[gpui_kit::test]
fn tags_rename_and_delete_in_all_notes_after_the_dialog(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    let a = notes_dir.join("A.md");
    let b = notes_dir.join("B.md");
    fs::write(&a, "---\ntags:\n  - work\n---\n# A\n").unwrap();
    fs::write(&b, "---\ntags:\n  - work/holm\n  - home\n---\n# B\n").unwrap();
    set_time(&a, 200);
    set_time(&b, 100);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(chips(cx, handle), ["work"]);

    // The tree: All notes, then the tags home and work.
    keys(cx, handle, "ctrl-shift-l escape down down");
    assert_eq!(sidebar(cx, handle), "tree #work");
    keys(cx, handle, "f2");
    keys(cx, handle, "ctrl-a");
    cx.simulate_input(handle, "job");
    keys(cx, handle, "enter");
    assert_eq!(
        question(cx, handle).as_deref(),
        Some("Rename “#work” to “#job”? This changes 2 notes.")
    );
    keys(cx, handle, "escape");
    assert_eq!(question(cx, handle), None);
    assert_eq!(fields(&a).0, ["work"]);

    keys(cx, handle, "f2");
    keys(cx, handle, "ctrl-a");
    cx.simulate_input(handle, "job");
    keys(cx, handle, "enter");
    keys(cx, handle, "enter");
    assert_eq!(question(cx, handle), None);
    assert_eq!(fields(&a).0, ["job"]);
    assert_eq!(fields(&b).0, ["job/holm", "home"]);
    assert_eq!(sidebar(cx, handle), "tree #job");
    assert_eq!(
        chips(cx, handle),
        ["job"],
        "the open note shows the new name"
    );

    // The tag dialogs have no "Do not show again", so Space does nothing.
    keys(cx, handle, "delete");
    assert_eq!(
        question(cx, handle).as_deref(),
        Some("Remove “#job” from all notes? This changes 2 notes.")
    );
    assert!(screen(cx, handle, |window| window
        .try_find("do-not-ask")
        .is_none()));
    keys(cx, handle, "space enter");
    assert_eq!(fields(&a).0, Vec::<String>::new());
    assert_eq!(fields(&b).0, ["home"]);
    assert_eq!(chips(cx, handle), Vec::<String>::new());
    assert!(Config::load(fixture.config()).settings().ask_before_delete);

    // A gone top tag leaves the tree with All notes. The tree has All notes, then the tag home.
    assert_eq!(sidebar(cx, handle), "tree All notes");
    keys(cx, handle, "down");
    assert_eq!(sidebar(cx, handle), "tree #home");
    keys(cx, handle, "delete");
    assert_eq!(
        question(cx, handle).as_deref(),
        Some("Remove “#home” from all notes? This changes 1 note.")
    );
    keys(cx, handle, "enter");
    assert_eq!(fields(&b).0, Vec::<String>::new());

    // A later save of the open note keeps the new tags.
    keys(cx, handle, "ctrl-shift-l");
    cx.simulate_input(handle, "x");
    autosave(cx);
    assert_eq!(fields(&a).0, Vec::<String>::new());
    assert_eq!(
        modified(&b),
        std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(100)
    );
}

/// Presses `keys` and runs all tasks, before the sidebar reads the notes again.
fn keys_now(cx: &mut TestAppContext, handle: AnyWindowHandle, keys: &str) {
    cx.simulate_keystrokes(handle, keys);
    cx.run_until_parked();
}

/// The rows of the tree. The selected row has `*`.
fn tree(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    labels(cx, handle, "sidebar", Role::TreeItem)
}

/// Starts the name field on the selected row and types `name` over the old name.
fn type_new_name(cx: &mut TestAppContext, handle: AnyWindowHandle, name: &str) {
    keys(cx, handle, "f2");
    keys(cx, handle, "ctrl-a");
    cx.simulate_input(handle, name);
}

/// Presses `key` and runs no task, so a write that it starts has not run yet.
fn press(cx: &mut TestAppContext, handle: AnyWindowHandle, key: &'static str) {
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_keystroke(gpui_kit::Keystroke::parse(key).unwrap(), cx);
    })
    .unwrap();
}

/// The text of the name field in the tree, with or without the focus.
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

#[gpui_kit::test]
fn a_late_folder_rename_keeps_the_field_and_shows_writing(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::create_dir(notes_dir.join("work")).unwrap();
    fs::create_dir(notes_dir.join("work/sub")).unwrap();
    fs::write(notes_dir.join("Top.md"), "# Top\n").unwrap();
    let startup = Startup {
        write_budget: Duration::ZERO,
        ..fixture.startup(None)
    };
    let handle = open(cx, startup);

    keys(cx, handle, "ctrl-shift-l escape down right");
    type_new_name(cx, handle, "job");
    press(cx, handle, "enter");
    assert_eq!(tree_field(cx, handle).as_deref(), Some("job"));
    assert_eq!(tree(cx, handle), ["All notes", "work*", "work/sub"]);
    assert_eq!(state(cx, handle).as_deref(), Some(text::WRITING));

    // The field takes no keys until the write ends.
    press(cx, handle, "escape");
    press(cx, handle, "up");
    press(cx, handle, "backspace");
    assert_eq!(tree_field(cx, handle).as_deref(), Some("job"));
    assert_eq!(tree(cx, handle), ["All notes", "work*", "work/sub"]);

    cx.run_until_parked();
    assert_eq!(tree_field(cx, handle), None);
    assert_eq!(tree(cx, handle), ["All notes", "job*", "job/sub"]);
    assert_eq!(state(cx, handle), None);
    assert!(notes_dir.join("job/sub").is_dir());
}

#[gpui_kit::test]
fn a_folder_rename_to_a_taken_name_keeps_the_field_and_shows_the_answer(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::create_dir(notes_dir.join("job")).unwrap();
    fs::create_dir(notes_dir.join("work")).unwrap();
    fs::write(notes_dir.join("Top.md"), "# Top\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape down down");
    assert_eq!(tree(cx, handle), ["All notes", "job", "work*"]);
    type_new_name(cx, handle, "Job");
    keys(cx, handle, "enter");
    assert_eq!(tree(cx, handle), ["All notes", "job", "work*"]);
    assert_eq!(tree_field(cx, handle).as_deref(), Some("Job"));
    assert_eq!(message(cx, handle).as_deref(), Some(text::NAME_TAKEN));
    assert_eq!(folder_names(&notes_dir), ["Top.md", "job", "work"]);

    // The field takes a new name, and the write works.
    keys(cx, handle, "ctrl-a");
    cx.simulate_input(handle, "Work");
    keys(cx, handle, "enter");
    assert_eq!(tree_field(cx, handle), None);
    assert_eq!(tree(cx, handle), ["All notes", "Work*", "job"]);
    assert_eq!(folder_names(&notes_dir), ["Top.md", "Work", "job"]);
}

#[gpui_kit::test]
fn a_failed_folder_rename_shows_the_old_name_and_the_error(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::create_dir(notes_dir.join("work")).unwrap();
    fs::write(notes_dir.join("Top.md"), "# Top\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape down");
    fs::remove_dir(notes_dir.join("work")).unwrap();
    type_new_name(cx, handle, "job");
    keys_now(cx, handle, "enter");
    assert_eq!(tree_field(cx, handle), None);
    assert_eq!(tree(cx, handle), ["All notes", "work*"]);
    assert!(message(cx, handle).is_some());

    // Esc clears the answer, and the sidebar still gets it.
    keys(cx, handle, "escape");
    assert_eq!(message(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");
}

#[gpui_kit::test]
fn a_tag_rename_shows_the_new_name_after_the_confirm(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("A.md"), "---\ntags:\n  - old\n---\n# A\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape down");
    assert_eq!(tree(cx, handle), ["All notes", "#old*"]);
    type_new_name(cx, handle, "new");
    keys(cx, handle, "enter");
    assert!(question(cx, handle).is_some());
    assert_eq!(tree_field(cx, handle).as_deref(), Some("new"));
    assert_eq!(tree(cx, handle), ["All notes", "#old*"]);
    keys(cx, handle, "escape");
    assert_eq!(tree_field(cx, handle), None);
    assert_eq!(tree(cx, handle), ["All notes", "#old*"]);

    type_new_name(cx, handle, "new");
    keys(cx, handle, "enter");
    press(cx, handle, "enter");
    assert_eq!(question(cx, handle), None);
    assert_eq!(tree_field(cx, handle), None);
    assert_eq!(tree(cx, handle), ["All notes", "#new*"]);
    assert_eq!(tree_steps(cx, handle), [["All notes", "#new*"]]);
    settle(cx);
    assert_eq!(tree(cx, handle), ["All notes", "#new*"]);
    assert_eq!(fields(&notes_dir.join("A.md")).0, ["new"]);
}

#[gpui_kit::test]
fn a_tag_rename_onto_a_tag_merges_into_its_row(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("A.md"), "---\ntags:\n  - old\n---\n# A\n").unwrap();
    fs::write(notes_dir.join("B.md"), "---\ntags:\n  - new\n---\n# B\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape down down");
    assert_eq!(tree(cx, handle), ["All notes", "#new", "#old*"]);
    type_new_name(cx, handle, "new");
    keys(cx, handle, "enter");
    keys_now(cx, handle, "enter");
    assert_eq!(tree(cx, handle), ["All notes", "#new*"]);
    settle(cx);
    assert_eq!(tree(cx, handle), ["All notes", "#new*"]);
    assert_eq!(fields(&notes_dir.join("A.md")).0, ["new"]);
}

/// Runs the tasks one at a time, and gives the tree before them and after each one that
/// changes it. A frame can show each of these.
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

/// Renames the selected row to `name`, and gives the tree steps of the write.
fn rename_steps(cx: &mut TestAppContext, handle: AnyWindowHandle, name: &str) -> Vec<Vec<String>> {
    type_new_name(cx, handle, name);
    press(cx, handle, "enter");
    tree_steps(cx, handle)
}

/// A nested folder `area/work` with an open child `sub` and the note `A.md`, which is open.
fn nested_folders(fixture: &Fixture) {
    let notes_dir = fixture.notes();
    fs::create_dir_all(notes_dir.join("area/work/sub")).unwrap();
    fs::create_dir(notes_dir.join("area/bebra")).unwrap();
    fs::write(notes_dir.join("area/work/A.md"), "# A\n").unwrap();
}

#[gpui_kit::test]
fn a_folder_rename_keeps_the_row_selected_in_every_frame(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    nested_folders(&fixture);
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape down right down down right");
    let before = [
        "All notes",
        "area",
        "area/bebra",
        "area/work*",
        "area/work/sub",
    ];
    assert_eq!(tree(cx, handle), before);
    assert_eq!(
        rename_steps(cx, handle, "job"),
        [vec![
            "All notes",
            "area",
            "area/bebra",
            "area/job*",
            "area/job/sub"
        ],]
    );

    // The open note follows the folder, so the next save goes to the new file.
    assert_eq!(editor_text(cx, handle), "# A\n");
    keys(cx, handle, "ctrl-shift-l");
    cx.simulate_input(handle, "x");
    autosave(cx);
    let notes_dir = fixture.notes();
    assert!(read(&notes_dir.join("area/job/A.md")).contains('x'));
    assert!(!notes_dir.join("area/work").exists());
}

#[gpui_kit::test]
fn a_folder_rename_scrolls_to_the_new_place_of_the_row(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    for folder in 0..60 {
        fs::create_dir(fixture.notes().join(format!("f{folder:02}"))).unwrap();
    }
    fs::write(fixture.notes().join("Top.md"), "# Top\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape down");
    assert_eq!(sidebar(cx, handle), "tree f00");
    rename_steps(cx, handle, "zz");
    assert_eq!(sidebar(cx, handle), "tree zz");
}

#[gpui_kit::test]
fn a_folder_rename_to_a_taken_name_keeps_the_row_selected_in_every_frame(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    nested_folders(&fixture);
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape down right down down right");
    assert_eq!(
        rename_steps(cx, handle, "bebra"),
        [vec![
            "All notes",
            "area",
            "area/bebra",
            "area/work*",
            "area/work/sub"
        ],]
    );
    assert_eq!(tree_field(cx, handle).as_deref(), Some("bebra"));
}

#[gpui_kit::test]
fn a_new_folder_with_a_taken_name_keeps_the_field_and_esc_leaves_nothing(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    nested_folders(&fixture);
    let notes_dir = fixture.notes();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape down");
    assert_eq!(sidebar(cx, handle), "tree area");
    keys(cx, handle, "ctrl-shift-n");
    assert_eq!(
        tree(cx, handle),
        [
            "All notes",
            "area*",
            text::NEW_FOLDER,
            "area/bebra",
            "area/work"
        ]
    );
    cx.simulate_input(handle, "Bebra");
    keys(cx, handle, "enter");
    assert_eq!(tree_field(cx, handle).as_deref(), Some("Bebra"));
    assert_eq!(message(cx, handle).as_deref(), Some(text::NAME_TAKEN));
    // The field has the focus again.
    cx.simulate_input(handle, " 2");
    assert_eq!(tree_field(cx, handle).as_deref(), Some("Bebra 2"));

    keys(cx, handle, "escape");
    assert_eq!(tree_field(cx, handle), None);
    assert_eq!(
        tree(cx, handle),
        ["All notes", "area*", "area/bebra", "area/work"]
    );
    assert_eq!(folder_names(&notes_dir.join("area")), ["bebra", "work"]);

    // An empty name gets an answer too.
    keys(cx, handle, "ctrl-shift-n enter");
    assert_eq!(tree_field(cx, handle).as_deref(), Some(""));
    assert_eq!(message(cx, handle).as_deref(), Some(text::NAME_EMPTY));
}

#[gpui_kit::test]
fn a_move_into_a_clash_shows_the_answer_and_moves_nothing(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::create_dir(notes_dir.join("work")).unwrap();
    fs::write(notes_dir.join("Top.md"), "# Top\n").unwrap();
    fs::write(notes_dir.join("work/top.md"), "# Other\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    assert_eq!(editor_text(cx, handle), "# Top\n");
    keys(cx, handle, "ctrl-shift-l");
    keys(cx, handle, "ctrl-shift-m");
    keys(cx, handle, "down enter");
    assert_eq!(message(cx, handle).as_deref(), Some(text::NAME_TAKEN));
    assert_eq!(read(&notes_dir.join("Top.md")), "# Top\n");
    assert_eq!(read(&notes_dir.join("work/top.md")), "# Other\n");
}

#[gpui_kit::test]
fn the_move_picker_finds_a_folder_by_its_path(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::create_dir_all(notes_dir.join("work/old")).unwrap();
    fs::create_dir(notes_dir.join("home")).unwrap();
    fs::write(notes_dir.join("Top.md"), "# Top\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    let options =
        |cx: &mut TestAppContext| labels(cx, handle, "folder-picker", Role::ListBoxOption);

    keys(cx, handle, "ctrl-shift-l ctrl-shift-m");
    assert_eq!(question(cx, handle).as_deref(), Some(text::MOVE_TO));
    // The note is in the root folder already, so the root folder is not a choice.
    assert_eq!(options(cx), ["home*", "work", "work/old"]);

    cx.simulate_input(handle, "OLD");
    settle(cx);
    assert_eq!(options(cx), ["work/old*"]);
    keys(cx, handle, "enter");
    assert_eq!(question(cx, handle), None);
    assert!(notes_dir.join("work/old/Top.md").is_file());
    assert!(!notes_dir.join("Top.md").exists());
}

#[gpui_kit::test]
fn a_folder_move_takes_the_open_note_with_it(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    nested_folders(&fixture);
    let handle = open(cx, fixture.startup(None));

    // The picker lists the root folder first, then `area`.
    keys(cx, handle, "ctrl-shift-l escape down right down down");
    assert_eq!(sidebar(cx, handle), "tree area/work");
    keys(cx, handle, "ctrl-shift-m");
    keys(cx, handle, "enter");
    let notes_dir = fixture.notes();
    assert!(notes_dir.join("work/A.md").is_file());
    assert_eq!(sidebar(cx, handle), "tree work");

    keys(cx, handle, "ctrl-shift-l");
    cx.simulate_input(handle, "x");
    autosave(cx);
    assert!(read(&notes_dir.join("work/A.md")).contains('x'));
    assert!(!notes_dir.join("area/work").exists());
}

#[gpui_kit::test]
fn a_tag_rename_keeps_the_row_selected_in_every_frame(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(
        notes_dir.join("A.md"),
        "---\ntags:\n  - area/old/x\n  - area/new\n---\n# A\n",
    )
    .unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape down right down down right");
    assert_eq!(
        tree(cx, handle),
        [
            "All notes",
            "#area",
            "#area/new",
            "#area/old*",
            "#area/old/x"
        ]
    );
    type_new_name(cx, handle, "area/fresh");
    keys(cx, handle, "enter");
    press(cx, handle, "enter");
    assert_eq!(
        tree_steps(cx, handle),
        [vec![
            "All notes",
            "#area",
            "#area/fresh*",
            "#area/fresh/x",
            "#area/new"
        ],]
    );

    // A rename into a tag that exists merges into its row.
    keys(cx, handle, "f2");
    keys(cx, handle, "ctrl-a");
    cx.simulate_input(handle, "area/new");
    keys(cx, handle, "enter");
    press(cx, handle, "enter");
    let steps = tree_steps(cx, handle);
    assert!(
        steps
            .iter()
            .all(|step| !step.contains(&"#area*".to_owned())),
        "{steps:?}"
    );
    assert_eq!(
        steps.last().unwrap(),
        &["All notes", "#area", "#area/new*", "#area/new/x"]
    );
}

#[gpui_kit::test]
fn an_outside_folder_rename_selects_the_parent(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    nested_folders(&fixture);
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape down right down down right");
    assert_eq!(sidebar(cx, handle), "tree area/work");
    let notes_dir = fixture.notes();
    fs::rename(notes_dir.join("area/work"), notes_dir.join("area/job")).unwrap();
    let feed = fixture.feed.lock().unwrap();
    feed.as_ref()
        .unwrap()
        .changed(["area/work", "area/job"])
        .unwrap();
    drop(feed);
    settle(cx);
    assert_eq!(
        tree(cx, handle),
        ["All notes", "area*", "area/bebra", "area/job"]
    );
}

#[gpui_kit::test]
fn an_outside_rename_of_the_open_note_keeps_it_selected(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("A.md"), "---\nid: 01A\n---\n# A\n").unwrap();
    fs::write(notes_dir.join("B.md"), "---\nid: 01B\n---\n# B\n").unwrap();
    set_time(&notes_dir.join("A.md"), 200);
    set_time(&notes_dir.join("B.md"), 100);
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l");
    assert_eq!(notes_list(cx, handle), ["A*", "B"]);

    fs::rename(notes_dir.join("A.md"), notes_dir.join("C.md")).unwrap();
    let feed = fixture.feed.lock().unwrap();
    feed.as_ref().unwrap().changed(["A.md", "C.md"]).unwrap();
    drop(feed);
    settle(cx);

    assert_eq!(notes_list(cx, handle), ["C*", "B"]);
    assert_eq!(note_path(cx, handle).as_deref(), Some("C"));
}

/// The text of the name field in the sidebar, while it has the focus.
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

/// A has the tags `home` and `work/holm`, B has `work/holm`. A is open.
fn two_tagged_notes(fixture: &Fixture) -> (PathBuf, PathBuf) {
    let notes_dir = fixture.notes();
    let a = notes_dir.join("A.md");
    let b = notes_dir.join("B.md");
    fs::write(&a, "---\ntags:\n  - home\n  - work/holm\n---\n# A\n").unwrap();
    fs::write(&b, "---\ntags:\n  - work/holm\n---\n# B\n").unwrap();
    set_time(&a, 200);
    set_time(&b, 100);
    (a, b)
}

#[gpui_kit::test]
fn f2_in_the_tag_popup_renames_the_tag_in_the_sidebar(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let (a, b) = two_tagged_notes(&fixture);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(chips(cx, handle), ["home", "work/holm"]);

    keys(cx, handle, "ctrl-shift-t down down f2");
    assert!(!popup_open(cx, handle));
    assert_eq!(sidebar(cx, handle), "tree #work/holm");
    assert_eq!(name_field(cx, handle).as_deref(), Some("work/holm"));

    keys(cx, handle, "ctrl-a");
    cx.simulate_input(handle, "job");
    keys(cx, handle, "enter");
    assert_eq!(
        question(cx, handle).as_deref(),
        Some("Rename “#work/holm” to “#job”? This changes 2 notes.")
    );
    keys(cx, handle, "enter");
    assert_eq!(fields(&a).0, ["home", "job"]);
    assert_eq!(fields(&b).0, ["job"]);
    assert_eq!(chips(cx, handle), ["home", "job"]);
}

#[gpui_kit::test]
fn a_new_tag_renames_in_all_notes_with_no_wait(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let a = fixture.notes().join("A.md");
    fs::write(&a, "# A\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys_now(cx, handle, "ctrl-shift-t");
    cx.simulate_input(handle, "fresh");
    // F2 comes before the write of the new tag ends.
    press(cx, handle, "enter");
    press(cx, handle, "down");
    press(cx, handle, "f2");
    cx.run_until_parked();
    assert_eq!(name_field(cx, handle).as_deref(), Some("fresh"));
    assert_eq!(tree(cx, handle), ["All notes", "#fresh*"]);
    assert_eq!(tree_field(cx, handle).as_deref(), Some("fresh"));
    keys(cx, handle, "ctrl-a");
    cx.simulate_input(handle, "job");
    keys_now(cx, handle, "enter");
    assert_eq!(
        question(cx, handle).as_deref(),
        Some("Rename “#fresh” to “#job”? This changes 1 note.")
    );
    keys_now(cx, handle, "enter");
    assert_eq!(fields(&a).0, ["job"]);
}

/// Whether the last drawn frame shows `id`. It draws no new frame, so it shows a frame that
/// the window still owes.
fn drawn(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) -> bool {
    cx.update_window(handle, |_, window, _| shows(window, id))
        .unwrap()
}

fn shows(window: &Window, id: &'static str) -> bool {
    window.try_find(id).is_some_and(|element| element.visible())
}

/// Presses `key` and draws one frame. Work that waits for the end of the draw has not run yet.
fn first_frame_has(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    key: &'static str,
    id: &'static str,
) -> bool {
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_keystroke(gpui_kit::Keystroke::parse(key).unwrap(), cx);
        window.render_frame(cx);
        shows(window, id)
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_popup_closes_in_the_frame_of_the_focus_move(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    let target = "---\nid: 01T\ntags:\n  - work\n---\n# Target\n";
    fs::write(notes.join("Target.md"), target).unwrap();
    fs::write(notes.join("Other.md"), "# Other\n[t](holm:01T)\n").unwrap();
    set_time(&notes.join("Target.md"), 300);
    set_time(&notes.join("Other.md"), 100);
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l ctrl-shift-l ctrl-shift-t");
    assert_eq!(sidebar(cx, handle), "hidden");
    assert!(!first_frame_has(cx, handle, "ctrl-shift-l", "tag-popup"));
    settle(cx);
    assert!(!drawn(cx, handle, "tag-popup"));
    assert_eq!(focus(cx, handle), "sidebar");

    keys(cx, handle, "escape ctrl-shift-b");
    assert!(drawn(cx, handle, "backlinks"));
    assert!(!first_frame_has(cx, handle, "ctrl-shift-l", "backlinks"));
    settle(cx);
    assert!(!drawn(cx, handle, "backlinks"));

    // F2 moves the focus in an event after the key, so `first_frame_has` cannot see it.
    keys(cx, handle, "escape ctrl-shift-t down f2");
    assert!(!drawn(cx, handle, "tag-popup"));
    assert_eq!(name_field(cx, handle).as_deref(), Some("work"));
}

#[gpui_kit::test]
fn f2_in_the_tag_popup_shows_a_hidden_sidebar(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    two_tagged_notes(&fixture);
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l ctrl-shift-l");
    assert_eq!(sidebar(cx, handle), "hidden");

    keys(cx, handle, "ctrl-shift-t down f2");
    assert!(!popup_open(cx, handle));
    assert_eq!(sidebar(cx, handle), "tree #home");
    assert_eq!(name_field(cx, handle).as_deref(), Some("home"));
    keys(cx, handle, "escape");
    assert_eq!(focus(cx, handle), "sidebar");
    assert_eq!(name_field(cx, handle), None);
}

#[gpui_kit::test]
fn f2_in_the_tag_popup_needs_a_tag_line(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let (a, _) = two_tagged_notes(&fixture);
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l ctrl-shift-l");
    assert_eq!(sidebar(cx, handle), "hidden");

    keys(cx, handle, "ctrl-shift-t f2");
    assert!(popup_open(cx, handle), "with no cursor, F2 does nothing");
    cx.simulate_input(handle, "new");
    settle(cx);
    assert_eq!(options(cx, handle), ["Add #new"]);
    keys(cx, handle, "down f2");
    assert!(popup_open(cx, handle));
    assert_eq!(options(cx, handle), ["Add #new*"]);
    assert_eq!(focus(cx, handle), "tag field");
    assert_eq!(sidebar(cx, handle), "hidden");
    assert_eq!(fields(&a).0, ["home", "work/holm"]);
}

#[gpui_kit::test]
fn a_folder_delete_selects_the_parent_folder(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::create_dir_all(notes_dir.join("work/plans")).unwrap();
    fs::write(notes_dir.join("work/Goals.md"), "# Goals\n").unwrap();
    fs::write(notes_dir.join("work/plans/Plan.md"), "# Plan\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape down right down");
    assert_eq!(sidebar(cx, handle), "tree work/plans");
    keys(cx, handle, "delete");
    assert!(question(cx, handle).is_some());
    keys(cx, handle, "enter");
    assert!(!notes_dir.join("work/plans").exists());
    assert_eq!(sidebar(cx, handle), "tree work");

    keys(cx, handle, "delete");
    assert!(question(cx, handle).is_some());
    keys(cx, handle, "enter");
    assert!(!notes_dir.join("work").exists());
    assert_eq!(sidebar(cx, handle), "tree All notes");
    assert_eq!(
        Config::load(fixture.config()).settings().sidebar.view,
        SavedSidebarView {
            list: false,
            item: SidebarItem::All,
        }
    );
}

/// Whether the tree row `label` shows open. `None` means it has no child rows.
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

/// A note and the folders `area/work/plans`, with `area/work/plans` deleted in the app.
fn delete_the_last_child_folder(cx: &mut TestAppContext, fixture: &Fixture) -> AnyWindowHandle {
    fs::create_dir_all(fixture.notes().join("area/work/plans")).unwrap();
    fs::write(fixture.notes().join("A.md"), "# A\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l escape down right down right");
    assert_eq!(expanded(cx, handle, "area/work"), Some(true));
    keys(cx, handle, "down delete enter");
    assert_eq!(sidebar(cx, handle), "tree area/work");
    handle
}

#[gpui_kit::test]
fn a_folder_shows_closed_after_its_last_child_folder_is_deleted(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = delete_the_last_child_folder(cx, &fixture);

    assert_eq!(tree(cx, handle), ["All notes", "area", "area/work*"]);
    assert_eq!(expanded(cx, handle, "area/work"), None);
    // `←` on a closed folder selects its parent.
    keys(cx, handle, "left");
    assert_eq!(sidebar(cx, handle), "tree area");
}

#[gpui_kit::test]
fn a_new_child_folder_after_the_delete_shows_at_once(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = delete_the_last_child_folder(cx, &fixture);

    fs::create_dir(fixture.notes().join("area/work/later")).unwrap();
    fixture.tell(cx, "area/work/later");
    assert_eq!(
        tree(cx, handle),
        ["All notes", "area", "area/work*", "area/work/later"]
    );
    assert_eq!(expanded(cx, handle, "area/work"), Some(true));
}

#[gpui_kit::test]
fn esc_on_a_new_folder_in_a_folder_without_children_shows_it_closed(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::create_dir(fixture.notes().join("home")).unwrap();
    fs::write(fixture.notes().join("A.md"), "# A\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape down");
    assert_eq!(expanded(cx, handle, "home"), None);
    keys(cx, handle, "ctrl-shift-n");
    assert_eq!(tree(cx, handle), ["All notes", "home*", text::NEW_FOLDER]);
    assert_eq!(expanded(cx, handle, "home"), Some(true));

    keys(cx, handle, "escape");
    assert_eq!(tree(cx, handle), ["All notes", "home*"]);
    assert_eq!(expanded(cx, handle, "home"), None);
}

#[gpui_kit::test]
fn a_child_tag_delete_selects_the_parent_tag_while_it_exists(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    let a = notes_dir.join("A.md");
    let b = notes_dir.join("B.md");
    fs::write(&a, "---\ntags:\n  - work\n  - work/holm\n---\n# A\n").unwrap();
    fs::write(&b, "---\ntags:\n  - home/garden\n---\n# B\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    // The tree: All notes, then the tags home and work.
    keys(cx, handle, "ctrl-shift-l escape down down right down");
    assert_eq!(sidebar(cx, handle), "tree #work/holm");
    keys(cx, handle, "delete");
    keys(cx, handle, "enter");
    assert_eq!(fields(&a).0, ["work"]);
    assert_eq!(sidebar(cx, handle), "tree #work");

    // No note keeps the tag home, so it goes with its child.
    keys(cx, handle, "up right down");
    assert_eq!(sidebar(cx, handle), "tree #home/garden");
    keys(cx, handle, "delete");
    keys(cx, handle, "enter");
    assert_eq!(fields(&b).0, Vec::<String>::new());
    assert_eq!(sidebar(cx, handle), "tree All notes");
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

#[gpui_kit::test]
fn search_lists_the_notes_that_match_and_esc_returns(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    three_notes(&fixture.notes());
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l escape");
    assert_eq!(sidebar(cx, handle), "tree All notes");

    keys(cx, handle, "ctrl-shift-f");
    assert_eq!(focus(cx, handle), "search");
    cx.simulate_input(handle, "read");
    settle(cx);
    assert_eq!(sidebar(cx, handle), "list Search: read");
    assert_eq!(notes_list(cx, handle), ["Goals"]);

    keys(cx, handle, "down");
    assert_eq!(focus(cx, handle), "sidebar");
    assert_eq!(editor_text(cx, handle), "# Goals\nread more\n");
    assert_eq!(notes_list(cx, handle), ["Goals*"]);

    // The search also matches tags, and a new search selects the old text.
    keys(cx, handle, "ctrl-shift-f");
    cx.simulate_input(handle, "work");
    settle(cx);
    assert_eq!(sidebar(cx, handle), "list Search: work");
    assert_eq!(notes_list(cx, handle), ["Goals*"]);

    keys(cx, handle, "escape");
    assert_eq!(sidebar(cx, handle), "tree All notes");
    assert_eq!(focus(cx, handle), "sidebar");

    // A search with no match, and `Esc` in the field.
    keys(cx, handle, "ctrl-shift-f");
    cx.simulate_input(handle, "zebra");
    settle(cx);
    assert_eq!(notes_list(cx, handle), Vec::<String>::new());
    keys(cx, handle, "escape");
    assert_eq!(sidebar(cx, handle), "tree All notes");
    assert_eq!(focus(cx, handle), "sidebar");

    // Ctrl+Shift+F also shows a hidden sidebar. The view is not part of the config.
    keys(cx, handle, "ctrl-shift-l");
    assert_eq!(sidebar(cx, handle), "hidden");
    keys(cx, handle, "ctrl-shift-f");
    cx.simulate_input(handle, "milk");
    settle(cx);
    assert_eq!(notes_list(cx, handle), ["Plan"]);
    let settings = Config::load(fixture.config()).settings().clone();
    assert_eq!(
        settings.sidebar.view,
        SavedSidebarView {
            list: false,
            item: SidebarItem::All,
        }
    );
    assert!(!settings.sidebar.hidden);

    // The find bar of the editor lets Ctrl+Shift+F through.
    keys(cx, handle, "down enter");
    keys(cx, handle, "ctrl-f");
    assert_eq!(focus(cx, handle), "editor");
    keys(cx, handle, "ctrl-shift-f");
    assert_eq!(focus(cx, handle), "search");
}

/// The rows of the note list after each task, each change once.
fn list_steps(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<Vec<String>> {
    let mut steps = vec![notes_list(cx, handle)];
    while cx.executor().tick() {
        let now = notes_list(cx, handle);
        if steps.last() != Some(&now) {
            steps.push(now);
        }
    }
    steps
}

#[gpui_kit::test]
fn a_search_during_an_outside_rename_keeps_the_note_in_every_frame(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("Home.md"), "# Home\n").unwrap();
    fs::write(notes_dir.join("Trip.md"), "# Trip\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-f");
    cx.simulate_input(handle, "tri");
    settle(cx);
    assert_eq!(notes_list(cx, handle), ["Trip"]);

    // The core has the rename, and the sidebar has not seen it yet.
    fs::rename(notes_dir.join("Trip.md"), notes_dir.join("Trips.md")).unwrap();
    let feed = fixture.feed.lock().unwrap();
    feed.as_ref()
        .unwrap()
        .changed(["Trip.md", "Trips.md"])
        .unwrap();
    drop(feed);
    press(cx, handle, "p");

    assert_eq!(list_steps(cx, handle), [vec!["Trip"], vec!["Trips"]]);
}

#[gpui_kit::test]
fn quick_open_opens_a_note_by_its_title(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    three_notes(&fixture.notes());
    let handle = open(cx, fixture.startup(None));
    let options = |cx: &mut TestAppContext| labels(cx, handle, "quick-open", Role::ListBoxOption);
    assert_eq!(editor_text(cx, handle), "# Plan\nbuy milk\n");

    keys(cx, handle, "ctrl-shift-o");
    assert_eq!(question(cx, handle).as_deref(), Some(text::QUICK_OPEN));
    assert_eq!(options(cx), ["Plan*", "Goals", "Top"]);

    // Titles only: "milk" is in the text of Plan.
    cx.simulate_input(handle, "milk");
    settle(cx);
    assert_eq!(options(cx), Vec::<String>::new());

    keys(cx, handle, "escape");
    assert_eq!(question(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "ctrl-shift-l ctrl-shift-o");
    cx.simulate_input(handle, "o");
    settle(cx);
    assert_eq!(options(cx), ["Goals*", "Top"]);
    keys(cx, handle, "down enter");
    assert_eq!(question(cx, handle), None);
    assert_eq!(editor_text(cx, handle), "# Top\n");
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(notes_list(cx, handle), ["Plan", "Goals", "Top*"]);

    // `Esc` gives the focus back to the part that had it.
    keys(cx, handle, "ctrl-shift-l ctrl-shift-o escape");
    assert_eq!(focus(cx, handle), "sidebar");
}

/// Presses `key`, and runs no task.
fn press_only(cx: &mut TestAppContext, handle: AnyWindowHandle, key: &'static str) {
    cx.dispatch_keystroke(handle, gpui_kit::Keystroke::parse(key).unwrap());
}

/// The keys that the old note takes while another note opens reach its file, also after keys in
/// the new note.
#[gpui_kit::test]
fn text_typed_while_another_note_opens_reaches_its_file(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    three_notes(&fixture.notes());
    let startup = Startup {
        write_budget: Duration::ZERO,
        ..fixture.startup(None)
    };
    let handle = open(cx, startup);
    keys(cx, handle, "ctrl-shift-o");
    cx.simulate_input(handle, "Top");
    settle(cx);

    press_only(cx, handle, "enter");
    let mut typed = 0;
    while editor_text(cx, handle).contains("# Plan") {
        press_only(cx, handle, "m");
        typed += 1;
        if !cx.executor().tick() {
            break;
        }
    }
    assert!(typed > 1, "the open took {typed} keys");
    assert_eq!(editor_text(cx, handle), "# Top\n");
    cx.simulate_input(handle, "xy");
    autosave(cx);

    let plan = read(&fixture.notes().join("Plan.md"));
    assert!(
        plan.ends_with(&format!("{}# Plan\nbuy milk\n", "m".repeat(typed))),
        "{typed}: {plan}"
    );
    assert!(read(&fixture.notes().join("Top.md")).ends_with("xy# Top\n"));
}

/// The folder in each option of the popup `id`, or `None` for an option with no folder.
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

#[gpui_kit::test]
fn quick_open_shows_the_folder_of_each_note(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::create_dir_all(notes.join("work/old")).unwrap();
    fs::write(notes.join("Plan.md"), "# Plan\n").unwrap();
    fs::write(notes.join("work/old/Beta.md"), "# Beta\n").unwrap();
    set_time(&notes.join("Plan.md"), 300);
    set_time(&notes.join("work/old/Beta.md"), 200);
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-o");
    assert_eq!(
        labels(cx, handle, "quick-open", Role::ListBoxOption),
        ["Plan*", "Beta"]
    );
    assert_eq!(
        folders(cx, handle, "quick-open"),
        [None, Some("work/old".to_owned())]
    );
}

/// Clicks near the bottom left of the editor, where no popup panel lies.
fn click_below(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    let bottom = screen(cx, handle, |window| {
        window.find("editor").bounds().size.height
    });
    cx.update_window(handle, |_, window, cx| {
        window.click_at(
            "editor",
            gpui_kit::point(gpui_kit::px(20.), bottom - gpui_kit::px(20.)),
            cx,
        );
    })
    .unwrap();
    settle(cx);
}

#[gpui_kit::test]
fn a_click_outside_a_popup_never_reaches_the_parts_under_it(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    three_notes(&fixture.notes());
    let handle = open(cx, fixture.startup(None));

    // Quick open and the link picker close, as Esc does.
    keys(cx, handle, "ctrl-shift-l ctrl-shift-o");
    click_below(cx, handle);
    assert_eq!(question(cx, handle), None);
    assert_eq!(focus(cx, handle), "sidebar");
    keys(cx, handle, "ctrl-shift-o");
    assert_eq!(question(cx, handle).as_deref(), Some(text::QUICK_OPEN));

    keys(cx, handle, "escape ctrl-shift-l ctrl-k");
    assert_eq!(question(cx, handle).as_deref(), Some(text::LINK_TO_NOTE));
    click_below(cx, handle);
    assert_eq!(question(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(editor_text(cx, handle), "# Plan\nbuy milk\n");

    // A dialog stays open and keeps the focus.
    keys(cx, handle, "ctrl-shift-l delete");
    let delete = question(cx, handle);
    assert!(delete.is_some());
    click_below(cx, handle);
    assert_eq!(question(cx, handle), delete);
    assert_eq!(focus(cx, handle), "other");
}

/// The id in the frontmatter of a note file.
fn id_of(path: &Path) -> String {
    holm_markdown::NoteFile::parse(&read(path))
        .id()
        .expect("the note has an id")
}

#[gpui_kit::test]
fn the_link_picker_links_notes_headings_and_new_notes(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::create_dir(notes.join("work")).unwrap();
    fs::write(notes.join("Alpha.md"), "# Alpha\n").unwrap();
    fs::write(
        notes.join("work/Beta.md"),
        "# Beta\n\n## Bug list\n\n## Ideas\n",
    )
    .unwrap();
    set_time(&notes.join("Alpha.md"), 300);
    set_time(&notes.join("work/Beta.md"), 200);
    let handle = open(cx, fixture.startup(None));
    let options = |cx: &mut TestAppContext| labels(cx, handle, "link-picker", Role::ListBoxOption);
    assert_eq!(editor_text(cx, handle), "# Alpha\n");
    keys(cx, handle, "ctrl-end");

    // A note link. The note had no id, so it gets one.
    keys(cx, handle, "ctrl-k");
    assert_eq!(question(cx, handle).as_deref(), Some(text::LINK_TO_NOTE));
    assert_eq!(options(cx), ["Alpha*", "Beta"]);
    assert_eq!(
        folders(cx, handle, "link-picker"),
        [None, Some("work".to_owned())]
    );
    cx.simulate_input(handle, "bet");
    settle(cx);
    assert_eq!(options(cx), ["Beta*", "Create “bet”"]);
    keys(cx, handle, "enter");
    assert_eq!(question(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");
    let beta = id_of(&notes.join("work/Beta.md"));
    assert_eq!(
        editor_text(cx, handle),
        format!("# Alpha\n[Beta](holm:{beta})")
    );

    // A heading link. Tab fills in the title and switches to its headings.
    cx.simulate_input(handle, " ");
    keys(cx, handle, "ctrl-k");
    cx.simulate_input(handle, "be");
    keys(cx, handle, "tab");
    assert_eq!(options(cx), ["Beta*", "Bug list", "Ideas"]);
    cx.simulate_input(handle, "bug");
    settle(cx);
    keys(cx, handle, "enter");
    assert_eq!(
        editor_text(cx, handle),
        format!("# Alpha\n[Beta](holm:{beta}) [Bug list](<holm:{beta}#Bug list>)")
    );

    // A new note, in the folder of the open note. The user stays in this note.
    cx.simulate_input(handle, " ");
    keys(cx, handle, "ctrl-k");
    cx.simulate_input(handle, "Gamma");
    settle(cx);
    assert_eq!(options(cx), ["Create “Gamma”*"]);
    keys(cx, handle, "enter");
    let gamma_path = notes.join("Gamma.md");
    assert_eq!(
        holm_markdown::NoteFile::parse(&read(&gamma_path)).body(),
        ""
    );
    let gamma = id_of(&gamma_path);
    let text = editor_text(cx, handle);
    assert!(text.starts_with("# Alpha\n"));
    assert!(text.ends_with(&format!(" [Gamma](holm:{gamma})")));

    // In a link, the field starts with its target. Enter changes the target and keeps the text.
    keys(cx, handle, "ctrl-k");
    assert_eq!(options(cx), ["Gamma*"]);
    keys(cx, handle, "ctrl-a backspace");
    cx.simulate_input(handle, "Beta");
    settle(cx);
    keys(cx, handle, "enter");
    assert!(
        editor_text(cx, handle).ends_with(&format!(" [Gamma](holm:{beta})")),
        "{}",
        editor_text(cx, handle)
    );

    // Esc changes nothing.
    let before = editor_text(cx, handle);
    keys(cx, handle, "ctrl-k");
    cx.simulate_input(handle, "Alpha");
    keys(cx, handle, "escape");
    assert_eq!(question(cx, handle), None);
    assert_eq!(editor_text(cx, handle), before);
}

#[gpui_kit::test]
fn links_open_notes_at_headings_and_backlinks_lead_back(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(
        notes.join("Alpha.md"),
        "---\nid: 01A\n---\n# Alpha\n[ideas](holm:01B#Ideas)\n[gone](holm:01X)\n",
    )
    .unwrap();
    fs::write(
        notes.join("Beta.md"),
        "---\nid: 01B\n---\n# Beta\n\ntext\n\n## Ideas\n",
    )
    .unwrap();
    set_time(&notes.join("Alpha.md"), 300);
    set_time(&notes.join("Beta.md"), 200);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(backlink_count(cx, handle), None);

    // Ctrl+Enter on a heading link opens the note at the heading.
    keys(cx, handle, "down ctrl-enter");
    assert_eq!(editor_text(cx, handle), "# Beta\n\ntext\n\n## Ideas\n");
    cx.simulate_input(handle, "X");
    assert_eq!(editor_text(cx, handle), "# Beta\n\ntext\n\nX## Ideas\n");

    // The backlinks of Beta lead back to Alpha.
    assert_eq!(backlink_count(cx, handle).as_deref(), Some("← 1 note"));
    // The heading is at the top of the view, so the tag row is out of sight.
    keys(cx, handle, "ctrl-home");
    click(cx, handle, "backlinks-button");
    assert_eq!(backlinks(cx, handle), ["Alpha*"]);
    cx.update_window(handle, |_, window, cx| {
        window.click(gpui_kit::ElementId::NamedInteger("backlink".into(), 0), cx);
    })
    .unwrap();
    settle(cx);
    assert_eq!(focus(cx, handle), "editor");
    assert!(editor_text(cx, handle).starts_with("# Alpha\n"));
    assert_eq!(
        read(&notes.join("Beta.md")).lines().last(),
        Some("X## Ideas")
    );

    // A link to a missing note shows an answer.
    keys(cx, handle, "down down ctrl-enter");
    assert_eq!(message(cx, handle).as_deref(), Some(text::NOTE_GONE));
}

/// The label of the backlink count button, or `None` with no button.
fn backlink_count(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    screen(cx, handle, |window| {
        window
            .try_find("backlinks-button")
            .and_then(|button| button.label().map(str::to_owned))
    })
}

/// The notes in the open backlinks list. The cursor has `*`.
fn backlinks(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    labels(cx, handle, "backlinks", Role::ListBoxOption)
}

#[gpui_kit::test]
fn ctrl_shift_b_opens_the_backlinks_of_the_note(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("Target.md"), "---\nid: 01T\n---\n# Target\n").unwrap();
    fs::write(notes.join("Old.md"), "# Old\n[t](holm:01T)\n").unwrap();
    fs::write(notes.join("New.md"), "# New\n[t](holm:01T)\n").unwrap();
    set_time(&notes.join("Target.md"), 300);
    set_time(&notes.join("New.md"), 200);
    set_time(&notes.join("Old.md"), 100);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(editor_text(cx, handle), "# Target\n");
    assert_eq!(chips(cx, handle), Vec::<String>::new());
    assert_eq!(backlink_count(cx, handle).as_deref(), Some("← 2 notes"));
    assert!(
        screen(cx, handle, |window| window
            .find("tag-row")
            .bounds()
            .size
            .height)
            > gpui_kit::px(0.)
    );

    // Esc gives the focus back to the text.
    keys(cx, handle, "ctrl-shift-b");
    assert_eq!(backlinks(cx, handle), ["New*", "Old"]);
    keys(cx, handle, "escape");
    assert_eq!(backlinks(cx, handle), Vec::<String>::new());
    assert_eq!(focus(cx, handle), "editor");

    // `↓` and Enter open the second note.
    keys(cx, handle, "ctrl-shift-b down");
    assert_eq!(backlinks(cx, handle), ["New", "Old*"]);
    keys(cx, handle, "enter");
    assert_eq!(editor_text(cx, handle), "# Old\n[t](holm:01T)\n");
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(backlinks(cx, handle), Vec::<String>::new());
}

#[gpui_kit::test]
fn the_tag_popup_and_the_backlinks_list_take_turns(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    let target = "---\nid: 01T\ntags:\n  - work\n---\n# Target\n";
    fs::write(notes.join("Target.md"), target).unwrap();
    fs::write(notes.join("Other.md"), "# Other\n[t](holm:01T)\n").unwrap();
    set_time(&notes.join("Target.md"), 300);
    set_time(&notes.join("Other.md"), 100);
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-b");
    assert_eq!(backlinks(cx, handle), ["Other*"]);
    click_chip(cx, handle, 0);
    assert_eq!(backlinks(cx, handle), Vec::<String>::new());
    assert!(popup_open(cx, handle));
    assert_eq!(focus(cx, handle), "tag field");

    click(cx, handle, "backlinks-button");
    settle(cx);
    assert_eq!(backlinks(cx, handle), ["Other*"]);
    assert!(!popup_open(cx, handle));

    keys(cx, handle, "escape");
    assert_eq!(backlinks(cx, handle), Vec::<String>::new());
    assert!(!popup_open(cx, handle));
    assert_eq!(focus(cx, handle), "editor");
}

#[gpui_kit::test]
fn a_click_in_the_sidebar_keeps_the_focus_after_the_tag_popup_closes(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(
        fixture.notes().join("A.md"),
        "---\ntags:\n  - work\n---\n# A\n",
    )
    .unwrap();
    let handle = open(cx, fixture.startup(None));

    click_chip(cx, handle, 0);
    assert_eq!(focus(cx, handle), "tag field");
    cx.update_window(handle, |_, window, cx| {
        window.click(gpui_kit::ElementId::NamedInteger("note-row".into(), 0), cx);
    })
    .unwrap();
    settle(cx);
    assert!(!popup_open(cx, handle));
    assert_eq!(focus(cx, handle), "sidebar");
}

#[gpui_kit::test]
fn a_note_with_no_backlinks_has_no_count(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("Target.md"), "---\nid: 01T\n---\n# Target\n").unwrap();
    fs::write(notes.join("Other.md"), "# Other\n").unwrap();
    set_time(&notes.join("Target.md"), 300);
    set_time(&notes.join("Other.md"), 100);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(backlink_count(cx, handle), None);
    keys(cx, handle, "ctrl-shift-b");
    assert_eq!(backlinks(cx, handle), Vec::<String>::new());
    assert_eq!(focus(cx, handle), "editor");

    // A new link from another note makes the count grow.
    fixture.write_outside(cx, "Other.md", "# Other\n[t](holm:01T)\n");
    assert_eq!(backlink_count(cx, handle).as_deref(), Some("← 1 note"));
}

#[gpui_kit::test]
fn ctrl_o_embeds_a_file_where_it_is(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::create_dir(notes.join("work")).unwrap();
    fs::create_dir(notes.join("docs")).unwrap();
    fs::write(notes.join("work/a.md"), "# A\n").unwrap();
    fs::write(notes.join("docs/plan.pdf"), "pdf").unwrap();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-end ctrl-o");
    assert!(cx.did_prompt_for_paths());
    let picked = notes.join("docs/plan.pdf");
    cx.simulate_path_prompt_response(move |_| Some(vec![picked]));
    settle(cx);
    assert_eq!(
        editor_text(cx, handle),
        "# A\n![plan.pdf](../docs/plan.pdf)"
    );
    assert_eq!(fs::read_dir(notes.join("work")).unwrap().count(), 1);
}

#[gpui_kit::test]
fn ctrl_o_asks_for_files_and_ctrl_j_for_folders(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("a.md"), "# A\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    for (key, folders) in [("ctrl-o", false), ("ctrl-j", true)] {
        keys(cx, handle, key);
        cx.simulate_path_prompt_response(|options| {
            assert_eq!(options.files, !folders, "{key}");
            assert_eq!(options.directories, folders, "{key}");
            assert!(options.multiple, "{key}");
            assert_eq!(options.prompt.as_deref(), Some(text::LINK_FILE), "{key}");
            None
        });
        settle(cx);
    }
    assert_eq!(editor_text(cx, handle), "# A\n");
}

#[gpui_kit::test]
fn ctrl_j_embeds_a_folder_at_the_cursor(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::create_dir(notes.join("work")).unwrap();
    fs::create_dir(notes.join("docs")).unwrap();
    fs::write(notes.join("work/a.md"), "# A\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-end ctrl-j");
    let picked = notes.join("docs");
    cx.simulate_path_prompt_response(move |_| Some(vec![picked]));
    settle(cx);
    cx.simulate_input(handle, "!");
    assert_eq!(editor_text(cx, handle), "# A\n![docs](../docs)!");
}

#[gpui_kit::test]
fn an_empty_folder_shows_the_empty_pane_and_makes_no_note(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));

    assert!(empty_pane(cx, handle));
    assert_eq!(files(&fixture.notes()), Vec::<String>::new());
    assert_eq!(focus(cx, handle), "sidebar");
    assert_eq!(chips(cx, handle), Vec::<String>::new());

    // The keys that need a note do nothing.
    keys(cx, handle, "ctrl-shift-p ctrl-shift-t ctrl-shift-b ctrl-k");
    assert_eq!(files(&fixture.notes()), Vec::<String>::new());
    assert!(empty_pane(cx, handle));

    // With no sidebar, the window keys still work.
    keys(cx, handle, "ctrl-shift-l");
    assert_eq!(sidebar(cx, handle), "hidden");
    keys(cx, handle, "ctrl-shift-l");
    assert_eq!(focus(cx, handle), "sidebar");

    keys(cx, handle, "ctrl-n");
    assert!(!empty_pane(cx, handle));
    assert!(new_note_open(cx, handle));
    assert_eq!(focus(cx, handle), "title field");
    assert_eq!(files(&fixture.notes()), Vec::<String>::new());
    cx.simulate_input(handle, "Plans");
    keys(cx, handle, "enter");
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(files(&fixture.notes()), ["Plans.md"]);
    let settings = Config::load(fixture.config()).settings().clone();
    assert_eq!(settings.last_note, holm_core::NotePath::new("Plans.md"));
}

#[gpui_kit::test]
fn an_outside_add_on_the_empty_pane_only_lists_the_note(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));

    fixture.write_outside(cx, "Plans.md", "# Plans\n");

    assert!(empty_pane(cx, handle));
    keys(cx, handle, "enter");
    assert_eq!(notes_list(cx, handle), ["Plans"]);
    assert!(empty_pane(cx, handle));
}

#[gpui_kit::test]
fn an_outside_delete_opens_the_neighbor_or_the_empty_pane(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    three_notes(&notes_dir);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(editor_text(cx, handle), "# Plan\nbuy milk\n");

    fixture.delete_outside(cx, "Plan.md");
    assert_eq!(editor_text(cx, handle), "# Goals\nread more\n");

    fixture.delete_outside(cx, "Goals.md");
    fixture.delete_outside(cx, "Top.md");
    assert!(empty_pane(cx, handle));
    assert_eq!(files(&notes_dir), Vec::<String>::new());
    assert_eq!(state(cx, handle), None);
}

#[gpui_kit::test]
fn an_outside_delete_with_unsaved_text_asks_to_keep_or_close(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    three_notes(&notes_dir);
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-end");
    cx.simulate_input(handle, "mine");

    fixture.delete_outside(cx, "Plan.md");
    assert_eq!(state(cx, handle).as_deref(), Some(text::DELETED_ON_DISK));
    autosave(cx);
    assert!(!notes_dir.join("Plan.md").exists(), "no autosave");

    keys(cx, handle, "ctrl-shift-d");
    assert_eq!(state_menu(cx, handle), ["Keep mine*", "Close"]);
    assert_eq!(
        state_reason(cx, handle).as_deref(),
        Some(text::DELETED_REASON)
    );
    keys(cx, handle, "escape");
    assert_eq!(state(cx, handle).as_deref(), Some(text::DELETED_ON_DISK));
    keys(cx, handle, "ctrl-shift-d enter");
    assert_eq!(state(cx, handle), None);
    assert!(read(&notes_dir.join("Plan.md")).ends_with("# Plan\nbuy milk\nmine\n"));

    cx.simulate_input(handle, "!");
    fixture.delete_outside(cx, "Plan.md");
    assert_eq!(state(cx, handle).as_deref(), Some(text::DELETED_ON_DISK));
    keys(cx, handle, "ctrl-shift-d down enter");
    assert_eq!(editor_text(cx, handle), "# Goals\nread more\n");
    assert!(!notes_dir.join("Plan.md").exists());
    assert_eq!(state(cx, handle), None);
}

#[gpui_kit::test]
fn the_link_keys_do_nothing_on_the_empty_pane_with_no_sidebar(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l");
    assert_eq!(sidebar(cx, handle), "hidden");
    assert!(empty_pane(cx, handle));

    keys(cx, handle, "ctrl-k ctrl-o ctrl-j");
    assert!(!cx.did_prompt_for_paths());
    let picker = screen(cx, handle, |window| {
        window.try_find("link-picker").is_some()
    });
    assert!(!picker);
    assert!(empty_pane(cx, handle));
}

#[gpui_kit::test]
fn the_start_with_no_last_note_opens_the_first_note_of_the_note_order(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let config = fs::read_to_string(fixture.config()).unwrap();
    fixture.write_config(&format!("{config}note_order = \"title\"\n"));
    three_notes(&fixture.notes());
    let handle = open(cx, fixture.startup(None));
    assert_eq!(editor_text(cx, handle), "# Goals\nread more\n");
}

#[gpui_kit::test]
fn an_outside_delete_after_a_view_change_opens_the_first_note(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    three_notes(&fixture.notes());
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l down down");
    assert_eq!(editor_text(cx, handle), "# Top\n");

    // The tag work holds only Goals.
    keys(cx, handle, "escape down");
    assert_eq!(sidebar(cx, handle), "tree #work");

    fixture.delete_outside(cx, "Top.md");
    assert_eq!(editor_text(cx, handle), "# Plan\nbuy milk\n");
}

#[gpui_kit::test]
fn a_delete_of_the_open_note_opens_its_neighbor(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    three_notes(&fixture.notes());
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l down");
    assert_eq!(editor_text(cx, handle), "# Goals\nread more\n");

    keys(cx, handle, "delete");
    keys(cx, handle, "enter");
    assert_eq!(editor_text(cx, handle), "# Top\n");
    assert_eq!(files(&fixture.notes()), ["Plan.md", "Top.md"]);
}

#[gpui_kit::test]
fn close_after_an_outside_delete_writes_no_file(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    three_notes(&notes_dir);
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-end");
    cx.simulate_input(handle, "mine");
    let times = || ["Goals.md", "Top.md"].map(|note| modified(&notes_dir.join(note)));
    let before = times();

    fixture.delete_outside(cx, "Plan.md");
    keys(cx, handle, "ctrl-shift-d down enter");
    autosave(cx);
    assert_eq!(editor_text(cx, handle), "# Goals\nread more\n");
    assert_eq!(files(&notes_dir), ["Goals.md", "Top.md"]);
    assert_eq!(times(), before);
}

#[gpui_kit::test]
fn the_delete_dialog_names_the_note_by_its_title(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    new_note(cx, handle, "Plans");
    cx.simulate_input(handle, "# Spring plans");
    assert_eq!(files(&fixture.notes()), ["Plans.md"]);

    keys(cx, handle, "ctrl-shift-l delete");
    assert_eq!(
        question(cx, handle).as_deref(),
        Some("Delete “Plans”? This deletes 1 note forever.")
    );
}

#[gpui_kit::test]
fn a_quit_keeps_the_name_of_the_open_note(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    new_note(cx, handle, "Plans");
    cx.simulate_input(handle, "# Spring plans");

    cx.quit();
    assert_eq!(files(&fixture.notes()), ["Plans.md"]);
    assert!(read(&fixture.notes().join("Plans.md")).ends_with("\n# Spring plans\n"));
    let settings = Config::load(fixture.config()).settings().clone();
    assert_eq!(settings.last_note, holm_core::NotePath::new("Plans.md"));
}

#[gpui_kit::test]
fn a_window_close_keeps_the_name_of_the_open_note(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    new_note(cx, handle, "Plans");
    cx.simulate_input(handle, "# Spring plans");

    cx.update_window(handle, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    assert_eq!(files(&fixture.notes()), ["Plans.md"]);
    assert!(read(&fixture.notes().join("Plans.md")).ends_with("\n# Spring plans\n"));
    let settings = Config::load(fixture.config()).settings().clone();
    assert_eq!(settings.last_note, holm_core::NotePath::new("Plans.md"));
}

/// Ctrl+N, then `title` and Enter.
fn new_note(cx: &mut TestAppContext, handle: AnyWindowHandle, title: &str) {
    keys(cx, handle, "ctrl-n");
    cx.simulate_input(handle, title);
    keys(cx, handle, "enter");
}

/// The path at the end of the tag row, like `work › Plans`.
fn note_path(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    label(cx, handle, "note-path")
}

/// Whether the pane shows a new note with no file yet.
fn new_note_open(cx: &mut TestAppContext, handle: AnyWindowHandle) -> bool {
    screen(cx, handle, |window| window.try_find("new-note").is_some())
}

/// The text of the title field, while it is open.
fn title_field(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    screen(cx, handle, |window| {
        window
            .try_find("title-input")
            .and_then(|field| field.value().map(str::to_owned))
    })
}

#[gpui_kit::test]
fn f2_renames_the_note_and_esc_keeps_the_old_title(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::create_dir(notes.join("work")).unwrap();
    fs::write(notes.join("work/Plans.md"), "# Spring plans\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    assert_eq!(note_path(cx, handle).as_deref(), Some("work › Plans"));

    keys(cx, handle, "f2");
    assert_eq!(focus(cx, handle), "title field");
    assert_eq!(title_field(cx, handle).as_deref(), Some("Plans"));
    cx.simulate_input(handle, "Goals");
    keys(cx, handle, "escape");
    assert_eq!(title_field(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(files(&notes.join("work")), ["Plans.md"]);

    keys(cx, handle, "f2");
    cx.simulate_input(handle, "Goals");
    keys(cx, handle, "enter");
    assert_eq!(title_field(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(files(&notes.join("work")), ["Goals.md"]);
    assert_eq!(read(&notes.join("work/Goals.md")), "# Spring plans\n");
    assert_eq!(note_path(cx, handle).as_deref(), Some("work › Goals"));

    // The text goes on in the renamed file.
    cx.simulate_input(handle, "x");
    autosave(cx);
    assert!(read(&notes.join("work/Goals.md")).ends_with("---\nx# Spring plans\n"));
    assert_eq!(files(&notes.join("work")), ["Goals.md"]);
}

#[gpui_kit::test]
fn f2_with_a_taken_title_shows_the_answer_and_keeps_the_field(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("Plans.md"), "").unwrap();
    fs::write(notes.join("Ideas.md"), "").unwrap();
    set_time(&notes.join("Plans.md"), 200);
    set_time(&notes.join("Ideas.md"), 100);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(note_path(cx, handle).as_deref(), Some("Plans"));

    keys(cx, handle, "f2");
    cx.simulate_input(handle, "ideas");
    keys(cx, handle, "enter");
    assert_eq!(message(cx, handle).as_deref(), Some(text::NAME_TAKEN));
    assert_eq!(title_field(cx, handle).as_deref(), Some("ideas"));
    assert_eq!(focus(cx, handle), "title field");
    assert_eq!(files(&notes), ["Ideas.md", "Plans.md"]);

    // F2 in the note list opens the same field.
    keys(cx, handle, "escape ctrl-shift-l f2");
    assert_eq!(focus(cx, handle), "title field");
    assert_eq!(title_field(cx, handle).as_deref(), Some("Plans"));
}

#[gpui_kit::test]
fn ctrl_n_names_the_note_first_and_esc_leaves_nothing(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("Old.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-n");
    assert_eq!(focus(cx, handle), "title field");
    assert_eq!(title_field(cx, handle).as_deref(), Some(""));
    assert!(new_note_open(cx, handle));
    keys(cx, handle, "escape");
    assert!(!new_note_open(cx, handle));
    assert_eq!(files(&notes), ["Old.md"]);
    assert_eq!(editor_text(cx, handle), "old\n");
    assert_eq!(focus(cx, handle), "editor");

    // A refused title keeps the new note.
    keys(cx, handle, "ctrl-n");
    cx.simulate_input(handle, "old");
    keys(cx, handle, "enter");
    assert_eq!(message(cx, handle).as_deref(), Some(text::NAME_TAKEN));
    assert_eq!(title_field(cx, handle).as_deref(), Some("old"));
    keys(cx, handle, "ctrl-a backspace");
    cx.simulate_input(handle, "Plans");
    keys(cx, handle, "enter");
    assert_eq!(files(&notes), ["Old.md", "Plans.md"]);
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(note_path(cx, handle).as_deref(), Some("Plans"));
    cx.simulate_input(handle, "text");
    autosave(cx);
    assert!(read(&notes.join("Plans.md")).ends_with("---\ntext\n"));
}

#[gpui_kit::test]
fn enter_on_an_empty_title_shows_the_answer_with_no_wait(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Old.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    press(cx, handle, "ctrl-n");
    press(cx, handle, "enter");
    assert_eq!(title_field(cx, handle).as_deref(), Some(""));
    assert_eq!(focus(cx, handle), "title field");
    assert_eq!(message(cx, handle).as_deref(), Some(text::NAME_EMPTY));
}

/// Presses `key`, then runs the tasks one at a time. It gives whether a frame between them
/// shows the empty pane.
fn empty_pane_in_a_step(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    key: &'static str,
) -> bool {
    press(cx, handle, key);
    let mut shown = empty_pane(cx, handle);
    while cx.executor().tick() {
        shown |= empty_pane(cx, handle);
    }
    shown
}

#[gpui_kit::test]
fn esc_on_a_new_note_keeps_the_note_from_before_with_its_undo(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("Old.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    cx.simulate_input(handle, "new ");
    assert_eq!(editor_text(cx, handle), "new old\n");

    keys(cx, handle, "ctrl-n");
    assert!(!empty_pane_in_a_step(cx, handle, "escape"));
    assert_eq!(focus(cx, handle), "editor");
    keys(cx, handle, "ctrl-z");
    assert_eq!(editor_text(cx, handle), "old\n");
}

#[gpui_kit::test]
fn enter_on_a_new_note_shows_it_with_no_empty_pane(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("Old.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-n");
    cx.simulate_input(handle, "Plans");
    assert!(!empty_pane_in_a_step(cx, handle, "enter"));
    assert_eq!(note_path(cx, handle).as_deref(), Some("Plans"));
    assert_eq!(focus(cx, handle), "editor");
    cx.simulate_input(handle, "text");
    assert_eq!(editor_text(cx, handle), "text");
}

#[gpui_kit::test]
fn ctrl_n_in_a_tag_view_gives_the_note_that_tag(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("A.md"), "---\ntags: [work]\n---\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l escape down down down enter");
    assert_eq!(sidebar(cx, handle), "list #work");

    keys(cx, handle, "ctrl-n");
    assert_eq!(chips(cx, handle), ["work"]);
    cx.simulate_input(handle, "Plans");
    keys(cx, handle, "enter");
    assert_eq!(fields(&notes.join("Plans.md")).0, ["work"]);
}

#[gpui_kit::test]
fn a_click_under_the_title_field_of_a_new_note_does_nothing(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-n");
    let bottom = screen(cx, handle, |window| {
        window.find("new-note").bounds().size.height
    });
    cx.update_window(handle, |_, window, cx| {
        window.click_at(
            "new-note",
            gpui_kit::point(gpui_kit::px(200.), bottom - gpui_kit::px(20.)),
            cx,
        );
    })
    .unwrap();
    settle(cx);
    assert_eq!(focus(cx, handle), "title field");
    cx.simulate_input(handle, "x");
    assert_eq!(title_field(cx, handle).as_deref(), Some("x"));
    assert_eq!(files(&fixture.notes()), Vec::<String>::new());
}

#[gpui_kit::test]
fn a_click_between_the_title_and_the_backlinks_opens_no_popup(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("Target.md"), "---\nid: 01T\n---\n").unwrap();
    fs::write(notes.join("Old.md"), "[t](holm:01T)\n").unwrap();
    set_time(&notes.join("Target.md"), 200);
    set_time(&notes.join("Old.md"), 100);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(backlink_count(cx, handle).as_deref(), Some("← 1 note"));

    let row = bounds_of(cx, handle, "tag-row").unwrap();
    let title = bounds_of(cx, handle, "note-title").unwrap();
    let dot = bounds_of(cx, handle, "path-dot").unwrap();
    let y = title.center().y - row.top();
    for x in [(title.right() + dot.left()) / 2., dot.center().x] {
        cx.update_window(handle, |_, window, cx| {
            window.click_at("tag-row", gpui_kit::point(x - row.left(), y), cx);
        })
        .unwrap();
        settle(cx);
        assert!(!popup_open(cx, handle));
    }
}

#[gpui_kit::test]
fn the_title_field_opens_in_the_place_of_the_title(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::create_dir(notes.join("work")).unwrap();
    fs::write(notes.join("work/Plans.md"), "").unwrap();
    let handle = open(cx, fixture.startup(None));

    let before = bounds_of(cx, handle, "note-path").unwrap();
    keys(cx, handle, "f2");
    assert_eq!(focus(cx, handle), "title field");
    assert_eq!(bounds_of(cx, handle, "note-path"), Some(before));
}

#[gpui_kit::test]
fn a_click_on_the_dot_or_the_count_does_not_reach_the_open_title_field(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("Target.md"), "---\nid: 01T\n---\n").unwrap();
    fs::write(notes.join("Old.md"), "[t](holm:01T)\n").unwrap();
    set_time(&notes.join("Target.md"), 200);
    set_time(&notes.join("Old.md"), 100);
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "f2");

    let row = bounds_of(cx, handle, "tag-row").unwrap();
    let click = |cx: &mut TestAppContext, id: &'static str| {
        let at = bounds_of(cx, handle, id).unwrap();
        let y = at.center().y - row.top();
        cx.update_window(handle, |_, window, cx| {
            window.click_at(
                "tag-row",
                gpui_kit::point(at.center().x - row.left(), y),
                cx,
            );
        })
        .unwrap();
        settle(cx);
    };
    click(cx, "path-dot");
    assert!(!popup_open(cx, handle));
    assert_eq!(focus(cx, handle), "title field");
    // The click put no caret in the field, so the title is still all selected.
    cx.simulate_input(handle, "Goal");
    assert_eq!(title_field(cx, handle).as_deref(), Some("Goal"));

    click(cx, "backlinks-button");
    assert_eq!(backlinks(cx, handle), ["Old*"]);
    assert_eq!(title_field(cx, handle), None);
    assert_eq!(files(&notes), ["Old.md", "Target.md"]);
}

#[gpui_kit::test]
fn ctrl_n_after_a_refused_title_starts_clean(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes = fixture.notes();
    fs::write(notes.join("Target.md"), "---\nid: 01T\n---\n").unwrap();
    fs::write(notes.join("Old.md"), "[t](holm:01T)\n").unwrap();
    set_time(&notes.join("Target.md"), 200);
    set_time(&notes.join("Old.md"), 100);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(backlink_count(cx, handle).as_deref(), Some("← 1 note"));

    keys(cx, handle, "f2");
    cx.simulate_input(handle, "old");
    keys(cx, handle, "enter");
    assert_eq!(title_field(cx, handle).as_deref(), Some("old"));

    keys(cx, handle, "ctrl-n");
    assert_eq!(title_field(cx, handle).as_deref(), Some(""));
    assert_eq!(backlink_count(cx, handle), None);
}

#[path = "window_speed.rs"]
mod speed;

/// Changes the settings, as the settings dialog does.
fn change_settings(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    change: impl FnOnce(&mut Settings),
) {
    cx.update_window(handle, |root, _, cx| {
        let root = root.downcast::<gpui_kit::base::Root>().unwrap();
        let view = root.read(cx).view().clone();
        let config = view
            .downcast::<HolmWindow>()
            .unwrap()
            .read(cx)
            .config
            .clone();
        config.update(cx, |config, cx| config.change(change, cx));
    })
    .unwrap();
    settle(cx);
}

fn dark(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| theme::appearance(cx) == holm_editor::Appearance::Dark)
}

#[gpui_kit::test]
fn a_new_theme_applies_at_once(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    assert!(!dark(cx));

    change_settings(cx, handle, |settings| {
        settings.theme = AppearanceSetting::Dark;
    });
    assert!(dark(cx));
    change_settings(cx, handle, |settings| {
        settings.theme = AppearanceSetting::System;
    });
    assert!(!dark(cx));
}

#[gpui_kit::test]
fn a_new_note_order_sorts_the_sidebar_at_once(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    three_notes(&fixture.notes());
    let handle = open(cx, fixture.startup(None));
    assert_eq!(notes_list(cx, handle), ["Plan*", "Goals", "Top"]);

    change_settings(cx, handle, |settings| settings.note_order = Order::Title);
    assert_eq!(notes_list(cx, handle), ["Goals", "Plan*", "Top"]);
}

/// Opens an empty note, types `text` and presses `key` with `help`. Gives the editor text.
fn typed_with(cx: &mut TestAppContext, help: WritingHelp, text: &str, key: &str) -> String {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "").unwrap();
    let handle = open(cx, fixture.startup(None));
    change_settings(cx, handle, |settings| settings.writing_help = help);
    type_unsaved(cx, handle, text);
    keys(cx, handle, key);
    editor_text(cx, handle)
}

#[gpui_kit::test]
fn format_keys_off_apply_at_once(cx: &mut TestAppContext) {
    let off = WritingHelp {
        format_keys: false,
        ..WritingHelp::default()
    };
    assert_eq!(typed_with(cx, off, "a", "shift-left ctrl-b"), "a");
}

#[gpui_kit::test]
fn continue_lists_off_applies_at_once(cx: &mut TestAppContext) {
    let off = WritingHelp {
        continue_lists: false,
        ..WritingHelp::default()
    };
    assert_eq!(typed_with(cx, off, "- a", "enter"), "- a\n");
}

#[gpui_kit::test]
fn indent_lists_off_applies_at_once(cx: &mut TestAppContext) {
    let off = WritingHelp {
        indent_lists: false,
        ..WritingHelp::default()
    };
    assert_eq!(typed_with(cx, off, "- a", "tab"), "- a    ");
}

/// A notes folder with the note `Note.md` and the folder `other` with `A.md`.
fn two_folders() -> (Fixture, PathBuf) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "old\n").unwrap();
    let other = fixture.dir.path().join("other");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("A.md"), "a\n").unwrap();
    (fixture, other)
}

#[gpui_kit::test]
fn a_new_notes_folder_saves_the_text_and_opens_at_once(cx: &mut TestAppContext) {
    let (fixture, other) = two_folders();
    let handle = open(cx, fixture.startup(None));
    type_unsaved(cx, handle, "x");

    change_settings(cx, handle, |settings| {
        settings.notes_folder = Some(other.clone());
    });

    assert_eq!(editor_text(cx, handle), "a\n");
    assert!(read(&fixture.notes().join("Note.md")).ends_with("\nxold\n"));
    let config = Config::load(fixture.config());
    assert_eq!(config.settings().notes_folder, Some(other));
    assert_eq!(
        config.settings().last_note.as_ref().map(AsRef::as_ref),
        Some(Path::new("A.md"))
    );
}

#[gpui_kit::test]
fn a_new_notes_folder_with_a_failed_save_keeps_the_old_folder(cx: &mut TestAppContext) {
    let (fixture, other) = two_folders();
    let handle = open(cx, fixture.startup(None));
    read_only(&fixture.notes(), true);
    type_unsaved(cx, handle, "x");

    change_settings(cx, handle, |settings| {
        settings.notes_folder = Some(other.clone());
    });

    assert_eq!(editor_text(cx, handle), "xold\n");
    assert_eq!(state(cx, handle).as_deref(), Some(text::NOT_SAVED));
    let config = Config::load(fixture.config());
    assert_eq!(config.settings().notes_folder, Some(fixture.notes()));

    // The user picks again after the save works.
    read_only(&fixture.notes(), false);
    change_settings(cx, handle, |settings| {
        settings.notes_folder = Some(other.clone());
    });
    assert_eq!(editor_text(cx, handle), "a\n");
    assert!(read(&fixture.notes().join("Note.md")).ends_with("\nxold\n"));
}

/// The rows of the settings dialog. The row with the cursor has `*`.
fn setting_rows(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    labels(cx, handle, "settings", Role::ListItem)
}

#[gpui_kit::test]
fn ctrl_shift_s_opens_the_settings_and_esc_gives_the_focus_back(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "a\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "ctrl-shift-s");
    assert_eq!(question(cx, handle).as_deref(), Some(text::SETTINGS));
    assert_eq!(
        setting_rows(cx, handle),
        [
            "Notes folder*",
            "Theme",
            "Note order",
            "Format keys",
            "Continue lists on Enter",
            "Indent lists with Tab",
        ]
    );

    keys(cx, handle, "escape");
    assert_eq!(question(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(editor_text(cx, handle), "a\n");
}

#[gpui_kit::test]
fn each_setting_changes_by_keys_only(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    let settings = |fixture: &Fixture| Config::load(fixture.config()).settings().clone();
    keys(cx, handle, "ctrl-shift-s");

    keys(cx, handle, "tab right right");
    assert_eq!(setting_rows(cx, handle)[1], "Theme*");
    assert!(dark(cx));
    keys(cx, handle, "right");
    assert_eq!(
        settings(&fixture).theme,
        AppearanceSetting::Dark,
        "Right stops at the end"
    );
    keys(cx, handle, "left");
    assert_eq!(settings(&fixture).theme, AppearanceSetting::Light);
    assert!(!dark(cx));

    keys(cx, handle, "tab right");
    assert_eq!(settings(&fixture).note_order, Order::Title);
    keys(cx, handle, "left");
    assert_eq!(settings(&fixture).note_order, Order::LastChange);

    keys(cx, handle, "tab space");
    assert!(!settings(&fixture).writing_help.format_keys);
    keys(cx, handle, "tab space");
    assert!(!settings(&fixture).writing_help.continue_lists);
    keys(cx, handle, "tab space");
    assert!(!settings(&fixture).writing_help.indent_lists);
    keys(cx, handle, "space");
    assert!(settings(&fixture).writing_help.indent_lists);

    keys(cx, handle, "tab");
    assert_eq!(setting_rows(cx, handle)[0], "Notes folder*", "Tab wraps");
    keys(cx, handle, "shift-tab");
    assert_eq!(
        setting_rows(cx, handle)[5],
        "Indent lists with Tab*",
        "Shift+Tab wraps"
    );
    assert_eq!(question(cx, handle).as_deref(), Some(text::SETTINGS));
}

#[gpui_kit::test]
fn the_theme_options_keep_their_place_when_the_choice_moves(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-s tab");
    // The chosen option covers the line on its left, so only the right edges stay.
    let places = |cx: &mut TestAppContext| {
        let bounds = ["System", "Light", "Dark"].map(|id| bounds_of(cx, handle, id).unwrap());
        (bounds[0].left(), bounds.map(|bounds| bounds.right()))
    };

    let first = places(cx);
    keys(cx, handle, "right");
    assert_eq!(places(cx), first);
    keys(cx, handle, "right");
    assert_eq!(places(cx), first);
}

#[gpui_kit::test]
fn enter_on_the_folder_row_picks_a_new_notes_folder(cx: &mut TestAppContext) {
    let (fixture, other) = two_folders();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-s");

    keys(cx, handle, "tab enter");
    assert!(
        !cx.did_prompt_for_paths(),
        "Enter on a row with options picks nothing"
    );
    keys(cx, handle, "shift-tab enter");
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| None);
    settle(cx);
    assert_eq!(question(cx, handle).as_deref(), Some(text::SETTINGS));

    keys(cx, handle, "space");
    cx.simulate_path_prompt_response(|_| Some(vec![other.clone()]));
    settle(cx);

    assert_eq!(editor_text(cx, handle), "a\n");
    assert_eq!(
        Config::load(fixture.config()).settings().notes_folder,
        Some(other)
    );
}

#[gpui_kit::test]
fn the_format_keys_row_shows_the_keys_of_the_editor_table(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-s");

    assert_eq!(
        labels(cx, handle, "format-keys", Role::Label),
        ["Ctrl+B", "Ctrl+I", "Ctrl+1–6"]
    );
}

#[gpui_kit::test]
fn a_click_on_the_folder_button_picks_a_new_notes_folder(cx: &mut TestAppContext) {
    let (fixture, other) = two_folders();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-s");
    assert_eq!(
        label(cx, handle, "notes-folder"),
        Some(text::home_path(&fixture.notes()))
    );

    click(cx, handle, "notes-folder");
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| Some(vec![other.clone()]));
    settle(cx);

    assert_eq!(
        Config::load(fixture.config()).settings().notes_folder,
        Some(other)
    );
}

#[gpui_kit::test]
fn the_folder_screen_ignores_ctrl_shift_s(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let handle = open(cx, fixture.startup(None));
    assert_eq!(folder_screen(cx, handle).as_deref(), Some(text::ASK_FOLDER));

    keys(cx, handle, "ctrl-shift-s");
    assert_eq!(question(cx, handle), None);
}

/// The rows of the shortcut list, as "label: keys".
fn shortcut_rows(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    labels(cx, handle, "shortcuts", Role::ListItem)
}

/// The top of the first row of the shortcut list.
fn first_shortcut_top(cx: &mut TestAppContext, handle: AnyWindowHandle) -> gpui_kit::Pixels {
    screen(cx, handle, |window| {
        window.within("shortcuts").find_all_by_role(Role::ListItem)[0]
            .bounds()
            .top()
    })
}

#[gpui_kit::test]
fn ctrl_question_mark_opens_the_shortcut_list_and_esc_gives_the_focus_back(
    cx: &mut TestAppContext,
) {
    use crate::shortcuts;
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "a\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "ctrl-?");
    assert_eq!(question(cx, handle).as_deref(), Some(text::SHORTCUT_LIST));
    // The screen reads the two columns row by row, so the test compares the rows in any order.
    let mut rows = shortcut_rows(cx, handle);
    let mut want: Vec<_> = shortcuts::list()
        .iter()
        .map(|shortcut| {
            let keys = gpui_kit::component::kbd::Kbd::format(&shortcuts::keystroke(shortcut.keys));
            format!("{}: {keys}", shortcut.label)
        })
        .collect();
    rows.sort();
    want.sort();
    assert_eq!(rows, want, "one row for each row of both tables");
    for row in [
        "Bold: Ctrl+B",
        "Heading 6: Ctrl+6",
        "Word left: Ctrl+Left",
        "Next match: Enter",
        "Previous match: Shift+Enter",
        "New folder: Ctrl+Shift+N",
        "Shortcuts: Ctrl+?",
        "App menu: F10",
        "Close: Esc",
    ] {
        assert!(rows.iter().any(|shown| shown == row), "{row} is missing");
    }

    keys(cx, handle, "escape");
    assert_eq!(question(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(editor_text(cx, handle), "a\n");
}

#[gpui_kit::test]
fn the_shortcut_list_names_each_place_once_and_no_label_runs_into_its_keys(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-?");

    let mut headings = labels(cx, handle, "shortcuts", Role::Heading);
    headings.sort();
    let mut want = [
        text::IN_SIDEBAR,
        text::IN_TREE,
        text::IN_NOTE_LIST,
        text::IN_TAG_ROW,
        text::IN_QUESTION,
        text::IN_POPUP_LIST,
        text::IN_NAME_FIELD,
        text::IN_SETTINGS,
        text::IN_SHORTCUT_LIST,
        text::IN_FIND,
        text::IN_COLOR_WINDOW,
        text::IN_HEX_FIELD,
    ];
    want.sort_unstable();
    assert_eq!(headings, want);

    let count = crate::shortcuts::list().len();
    let crowded: Vec<String> = screen(cx, handle, |window| {
        (0..count)
            .filter_map(|at| {
                let item = window.try_find(("shortcut", at))?;
                let (edge, name) = (
                    item.bounds().right(),
                    item.label().unwrap_or_default().to_owned(),
                );
                let row = window.within(("shortcut", at));
                let label = row.try_find("label")?.bounds();
                let keys = row.try_find("keys")?.bounds();
                (label.right() >= keys.left() || keys.right() > edge).then_some(name)
            })
            .collect()
    });
    assert!(
        crowded.is_empty(),
        "labels run into their keys: {crowded:?}"
    );
}

#[gpui_kit::test]
fn a_popup_over_the_tag_field_gives_the_focus_home_when_it_closes(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "a\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    for popup in ["ctrl-?", "ctrl-shift-s"] {
        keys(cx, handle, "ctrl-shift-t");
        assert_eq!(focus(cx, handle), "tag field");
        // The popup takes the focus, so the tag field closes.
        keys(cx, handle, popup);
        keys(cx, handle, "escape");
        assert_eq!(question(cx, handle), None);
        assert_eq!(focus(cx, handle), "editor", "after {popup}");
    }
    keys(cx, handle, "ctrl-?");
    assert_eq!(question(cx, handle).as_deref(), Some(text::SHORTCUT_LIST));
}

#[gpui_kit::test]
fn up_and_down_scroll_the_shortcut_list(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-?");
    let top = first_shortcut_top(cx, handle);

    keys(cx, handle, "up");
    assert_eq!(first_shortcut_top(cx, handle), top, "Up stops at the top");
    keys(cx, handle, "down");
    let down = first_shortcut_top(cx, handle);
    assert!(down < top, "Down scrolls the list");
    keys(cx, handle, "down");
    assert!(first_shortcut_top(cx, handle) < down);
    keys(cx, handle, "up up");
    assert_eq!(first_shortcut_top(cx, handle), top);
    assert_eq!(question(cx, handle).as_deref(), Some(text::SHORTCUT_LIST));
}

#[gpui_kit::test]
fn the_folder_screen_ignores_ctrl_question_mark(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let handle = open(cx, fixture.startup(None));
    assert_eq!(folder_screen(cx, handle).as_deref(), Some(text::ASK_FOLDER));

    keys(cx, handle, "ctrl-?");
    assert_eq!(question(cx, handle), None);
}

/// The rows of the open app menu. The row under the cursor has `*`.
fn app_menu(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    labels(cx, handle, "app-menu", Role::ListBoxOption)
}

/// Clicks the item at `at` in the app menu.
fn click_menu_item(cx: &mut TestAppContext, handle: AnyWindowHandle, at: u64) {
    cx.update_window(handle, |_, window, cx| {
        window.click(
            gpui_kit::ElementId::NamedInteger("menu-item".into(), at),
            cx,
        );
    })
    .unwrap();
    settle(cx);
}

fn window_open(cx: &mut TestAppContext, handle: AnyWindowHandle) -> bool {
    cx.update_window(handle, |_, _, _| ()).is_ok()
}

#[gpui_kit::test]
fn f10_and_the_menu_button_open_the_app_menu_under_the_button(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "a\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "f10");
    assert_eq!(
        app_menu(cx, handle),
        ["Settings*", "Shortcuts", "Pin note", "Quit"]
    );
    let menu = bounds_of(cx, handle, "app-menu").unwrap();
    let button = bounds_of(cx, handle, "app-menu-button").unwrap();
    assert_eq!(menu.left(), button.left());
    assert!(menu.top() >= button.bottom());
    keys(cx, handle, "down down down");
    assert_eq!(
        app_menu(cx, handle),
        ["Settings", "Shortcuts", "Pin note", "Quit*"]
    );
    keys(cx, handle, "escape");
    assert_eq!(app_menu(cx, handle), Vec::<String>::new());
    assert_eq!(focus(cx, handle), "editor");

    // A second F10 or click closes the menu.
    keys(cx, handle, "f10");
    keys(cx, handle, "f10");
    assert_eq!(app_menu(cx, handle), Vec::<String>::new());
    assert_eq!(focus(cx, handle), "editor");
    click(cx, handle, "app-menu-button");
    assert_eq!(app_menu(cx, handle).len(), 4);
    click(cx, handle, "app-menu-button");
    assert_eq!(app_menu(cx, handle), Vec::<String>::new());
    assert_eq!(focus(cx, handle), "editor");

    // A click outside closes it too.
    click(cx, handle, "app-menu-button");
    click(cx, handle, "editor");
    assert_eq!(app_menu(cx, handle), Vec::<String>::new());
    assert_eq!(editor_text(cx, handle), "a\n");
}

#[gpui_kit::test]
fn each_app_menu_item_runs_its_action_by_keys_and_by_click(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "a\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "f10 enter");
    assert_eq!(question(cx, handle).as_deref(), Some(text::SETTINGS));
    keys(cx, handle, "escape");
    assert_eq!(focus(cx, handle), "editor");
    keys(cx, handle, "f10");
    click_menu_item(cx, handle, 0);
    assert_eq!(question(cx, handle).as_deref(), Some(text::SETTINGS));
    keys(cx, handle, "escape");

    keys(cx, handle, "f10 down enter");
    assert_eq!(question(cx, handle).as_deref(), Some(text::SHORTCUT_LIST));
    keys(cx, handle, "escape");
    keys(cx, handle, "f10");
    click_menu_item(cx, handle, 1);
    assert_eq!(question(cx, handle).as_deref(), Some(text::SHORTCUT_LIST));
    keys(cx, handle, "escape");
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "f10 down down enter");
    assert!(read(&note).contains("pinned: true"));
    keys(cx, handle, "f10");
    click_menu_item(cx, handle, 2);
    assert!(!read(&note).contains("pinned: true"));
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "f10");
    click_menu_item(cx, handle, 3);
    assert!(!window_open(cx, handle), "Quit closes the window");
}

#[gpui_kit::test]
fn quit_in_the_app_menu_closes_the_window_by_keys(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "a\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "f10 up");
    assert_eq!(
        app_menu(cx, handle),
        ["Settings*", "Shortcuts", "Pin note", "Quit"],
        "up stays on the first item"
    );
    keys(cx, handle, "down down down enter");
    assert!(!window_open(cx, handle));
}

#[gpui_kit::test]
fn pin_note_is_off_with_no_note_open(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    assert!(empty_pane(cx, handle));

    keys(cx, handle, "f10 down down");
    assert_eq!(
        app_menu(cx, handle),
        ["Settings", "Shortcuts", "Pin note", "Quit*"]
    );
    keys(cx, handle, "up");
    assert_eq!(
        app_menu(cx, handle),
        ["Settings", "Shortcuts*", "Pin note", "Quit"]
    );
    click_menu_item(cx, handle, 2);
    assert_eq!(
        app_menu(cx, handle).len(),
        4,
        "a click on Pin note does nothing"
    );
    keys(cx, handle, "escape");
    assert_eq!(focus(cx, handle), "sidebar");
}

#[gpui_kit::test]
fn f10_with_the_sidebar_hidden_opens_the_menu_at_the_corner(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "a\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l ctrl-shift-l");
    assert_eq!(sidebar(cx, handle), "hidden");

    keys(cx, handle, "f10");
    assert_eq!(app_menu(cx, handle).len(), 4);
    assert_eq!(sidebar(cx, handle), "hidden");
    let menu = bounds_of(cx, handle, "app-menu").unwrap();
    assert_eq!(
        (menu.left(), menu.top()),
        (gpui_kit::px(4.), gpui_kit::px(4.))
    );

    keys(cx, handle, "enter");
    assert_eq!(question(cx, handle).as_deref(), Some(text::SETTINGS));
    assert_eq!(sidebar(cx, handle), "hidden");
}

#[gpui_kit::test]
fn the_folder_screen_ignores_f10_and_ctrl_shift_q(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let handle = open(cx, fixture.startup(None));
    assert_eq!(folder_screen(cx, handle).as_deref(), Some(text::ASK_FOLDER));

    keys(cx, handle, "f10");
    assert_eq!(app_menu(cx, handle), Vec::<String>::new());
    keys(cx, handle, "ctrl-shift-q");
    assert!(window_open(cx, handle));
    assert_eq!(folder_screen(cx, handle).as_deref(), Some(text::ASK_FOLDER));
}

#[gpui_kit::test]
fn ctrl_shift_q_with_text_that_cannot_be_saved_needs_a_second_press(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("Note.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    read_only(&notes_dir, true);
    cx.simulate_input(handle, "a");
    keys(cx, handle, "ctrl-shift-q");
    assert!(window_open(cx, handle));
    assert_eq!(message(cx, handle).as_deref(), Some(text::CLOSE_AGAIN));

    keys(cx, handle, "ctrl-shift-q");
    assert!(!window_open(cx, handle));
    read_only(&notes_dir, false);
}

#[gpui_kit::test]
fn ctrl_shift_q_saves_the_typed_text_first(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    cx.simulate_input(handle, "a");
    keys(cx, handle, "ctrl-shift-q");
    assert!(!window_open(cx, handle));
    assert!(read(&note).ends_with("\naold\n"));
}
