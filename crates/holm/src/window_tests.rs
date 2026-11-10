//! One test for each flow of the main window, with a real core on a temp folder.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests")]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AnyWindowHandle, AppContext as _, Role, TestAppContext, Window};
use holm_core::{Core, Feed};
use tempfile::TempDir;

use super::{HolmWindow, Startup};
use crate::config::Config;
use crate::sidebar::{Item, View};
use crate::{changes, text, theme};

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
            crash_log,
        }
    }
}

fn open(cx: &mut TestAppContext, startup: Startup) -> AnyWindowHandle {
    let opened = cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(cx);
        gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| HolmWindow::new(startup, window, cx))
        })
    });
    let (handle, _) = opened.unwrap();
    cx.run_until_parked();
    handle
}

/// Lets the UI see the outside changes, and runs all tasks. The autosave does not run yet.
fn settle(cx: &mut TestAppContext) {
    cx.executor().advance_clock(changes::POLL);
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

fn editor_text(cx: &mut TestAppContext, handle: AnyWindowHandle) -> String {
    screen(cx, handle, |window| {
        window
            .find("editor")
            .value()
            .expect("the editor shows its text")
            .to_owned()
    })
}

/// The text of each quiet bar, from the top.
fn bars(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    screen(cx, handle, |window| {
        window
            .find_all_by_role(Role::Status)
            .iter()
            .filter_map(|bar| bar.label().map(str::to_owned))
            .collect()
    })
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
    assert_eq!(editor_text(cx, handle), "");
    let default = fixture.dir.path().join("Notes");
    assert!(default.join("Untitled.md").is_file());
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
        Some(text::folder_missing(&fixture.notes()))
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

#[gpui_kit::test]
fn an_outside_change_reloads_quietly_with_no_unsaved_text(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    fixture.write_outside(cx, "Note.md", "new\n");

    assert_eq!(editor_text(cx, handle), "new\n");
    assert_eq!(bars(cx, handle), Vec::<String>::new());
}

#[gpui_kit::test]
fn an_outside_change_with_unsaved_text_asks_to_reload_or_keep_mine(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "old\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    cx.simulate_input(handle, "mine ");
    fixture.write_outside(cx, "Note.md", "theirs\n");
    assert_eq!(bars(cx, handle), [text::CHANGED_ON_DISK]);
    autosave(cx);
    assert_eq!(read(&note), "theirs\n", "no autosave during the question");

    click(cx, handle, "keep-mine");
    assert!(read(&note).ends_with("\n---\nmine old\n"));
    assert_eq!(bars(cx, handle), Vec::<String>::new());

    cx.simulate_input(handle, "x");
    fixture.write_outside(cx, "Note.md", "theirs again\n");
    click(cx, handle, "reload");
    assert_eq!(editor_text(cx, handle), "theirs again\n");
    assert_eq!(bars(cx, handle), Vec::<String>::new());
}

#[gpui_kit::test]
fn a_broken_config_shows_a_bar_and_stays_as_it_is(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    fixture.write_config("version = [broken\n");
    let handle = open(cx, fixture.startup(None));
    assert_eq!(bars(cx, handle), [text::CONFIG_BROKEN]);

    click(cx, handle, "use-default");
    assert_eq!(editor_text(cx, handle), "");
    assert_eq!(read(&fixture.config()), "version = [broken\n");

    click(cx, handle, "config-broken-close");
    assert_eq!(bars(cx, handle), Vec::<String>::new());
}

#[gpui_kit::test]
fn a_crash_in_the_last_run_shows_the_log_path(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let log = PathBuf::from("/state/holm/holm.log");
    let handle = open(cx, fixture.startup(Some(log.clone())));
    assert_eq!(bars(cx, handle), [text::crashed(&log)]);
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

/// Which part has the focus: "sidebar", "editor", "tag field", "tag row", or "other".
fn focus(cx: &mut TestAppContext, handle: AnyWindowHandle) -> &'static str {
    screen(cx, handle, |window| {
        let focused = |id| window.try_find(id).and_then(|part| part.focused()) == Some(true);
        if focused("sidebar") {
            "sidebar"
        } else if focused("editor") {
            "editor"
        } else if focused("tag-input") {
            "tag field"
        } else if focused("tag-row") {
            "tag row"
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

/// The tags and the pin in the file of a note.
fn fields(path: &Path) -> (Vec<String>, bool) {
    let note = holm_markdown::Note::parse(&read(path));
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
    assert_eq!(editor_text(cx, handle), "");
    assert!(notes_dir.join("Untitled.md").is_file());
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
    assert_eq!(focus(cx, handle), "editor");
    cx.simulate_input(handle, "Goals");
    autosave(cx);

    keys(cx, handle, "ctrl-shift-l");
    assert_eq!(focus(cx, handle), "sidebar");
    keys(cx, handle, "escape");
    assert_eq!(sidebar(cx, handle), "tree work");
    keys(cx, handle, "up enter down");
    assert_eq!(editor_text(cx, handle), "# Top\n");
    assert!(read(&notes_dir.join("work/Goals.md")).ends_with("---\n# Goals\n"));
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
    assert_eq!(settings.sidebar.view, View::List(Item::All));
    assert_eq!(settings.last_note, Some(PathBuf::from("Top.md")));
}

#[gpui_kit::test]
fn keys_make_rename_and_move_notes_and_folders(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::write(notes_dir.join("Top.md"), "# Top\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-n");
    assert!(notes_dir.join(text::NEW_FOLDER).is_dir());
    cx.simulate_input(handle, "work");
    keys(cx, handle, "enter");
    assert!(notes_dir.join("work").is_dir());
    assert!(!notes_dir.join(text::NEW_FOLDER).exists());
    assert_eq!(sidebar(cx, handle), "tree work");

    keys(cx, handle, "enter ctrl-n");
    assert!(notes_dir.join("work/Untitled.md").is_file());
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

#[gpui_kit::test]
fn the_tag_row_adds_and_removes_tags_and_pins(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    let a = notes_dir.join("A.md");
    fs::write(&a, "---\ntags:\n  - work\n---\n# A\n").unwrap();
    fs::write(
        notes_dir.join("B.md"),
        "---\ntags:\n  - project/holm\n---\n# B\n",
    )
    .unwrap();
    set_time(&a, 200);
    set_time(&notes_dir.join("B.md"), 100);
    let handle = open(cx, fixture.startup(None));
    assert_eq!(editor_text(cx, handle), "# A\n");
    assert_eq!(chips(cx, handle), ["work"]);
    assert!(screen(cx, handle, |window| window
        .try_find("tag-input")
        .is_none()));

    keys(cx, handle, "ctrl-shift-t");
    assert_eq!(focus(cx, handle), "tag field");
    cx.simulate_input(handle, "ho");
    settle(cx);
    assert_eq!(
        labels(cx, handle, "tag-suggestions", Role::ListBoxOption),
        ["project/holm"]
    );
    keys(cx, handle, "tab");
    cx.simulate_input(handle, "big plans");
    keys(cx, handle, "enter");
    assert_eq!(chips(cx, handle), ["work", "project/holm", "big-plans"]);
    assert_eq!(fields(&a).0, ["work", "project/holm", "big-plans"]);

    keys(cx, handle, "left left");
    assert_eq!(focus(cx, handle), "tag row");
    assert_eq!(chips(cx, handle), ["work", "project/holm*", "big-plans"]);
    keys(cx, handle, "backspace");
    assert_eq!(chips(cx, handle), ["work", "big-plans*"]);
    keys(cx, handle, "escape");
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "ctrl-shift-p");
    assert!(screen(cx, handle, |window| window
        .try_find("pin-mark")
        .is_some()));
    assert_eq!(
        fields(&a),
        (vec!["work".to_owned(), "big-plans".to_owned()], true)
    );

    keys(cx, handle, "ctrl-z");
    autosave(cx);
    assert_eq!(
        fields(&a),
        (vec!["work".to_owned(), "big-plans".to_owned()], true)
    );
    assert_eq!(
        modified(&a),
        std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(200),
        "tags and pins keep the modification time"
    );

    keys(cx, handle, "up");
    assert_eq!(focus(cx, handle), "tag field");
    keys(cx, handle, "down");
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(editor_text(cx, handle), "# A\n");
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
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(gpui_kit::component::input::SelectAll), cx);
    })
    .unwrap();
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
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(gpui_kit::component::input::SelectAll), cx);
    })
    .unwrap();
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

    // The gone tag leaves the list of all notes. The tree has All notes, then the tag home.
    assert_eq!(sidebar(cx, handle), "list All notes");
    keys(cx, handle, "escape down");
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
