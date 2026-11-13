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
use crate::{notes_folder, text, theme};

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

/// Opens an active window, as the user sees it. An inactive window sends no focus events.
fn open(cx: &mut TestAppContext, startup: Startup) -> AnyWindowHandle {
    let opened = cx.update(|cx| {
        gpui_kit::init(cx);
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

/// Lets the UI see the outside changes, and runs all tasks. The autosave does not run yet.
fn settle(cx: &mut TestAppContext) {
    cx.executor().advance_clock(notes_folder::POLL);
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
    assert!(empty_pane(cx, handle));
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

/// Which part has the focus: "sidebar", "editor", "tag field", "tag row", "search", or
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
    assert!(notes_dir.join(text::NEW_FOLDER).is_dir());
    assert_eq!(tree(cx, handle), ["All notes", "New folder*"]);
    assert_eq!(tree_field(cx, handle).as_deref(), Some(text::NEW_FOLDER));
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
            gpui_kit::point(right - gpui_kit::px(4.), gpui_kit::px(16.)),
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
        count.top(),
        more.top(),
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
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(gpui_kit::component::input::SelectAll), cx);
    })
    .unwrap();
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
fn a_folder_rename_locks_the_field_until_the_write_ends(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::create_dir(notes_dir.join("work")).unwrap();
    fs::create_dir(notes_dir.join("work/sub")).unwrap();
    fs::write(notes_dir.join("Top.md"), "# Top\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape down right");
    assert_eq!(tree(cx, handle), ["All notes", "work*", "work/sub"]);
    type_new_name(cx, handle, "job");
    assert_eq!(tree_field(cx, handle).as_deref(), Some("job"));
    press(cx, handle, "enter");
    assert_eq!(tree_field(cx, handle).as_deref(), Some("job"));
    assert_eq!(tree(cx, handle), ["All notes", "work*", "work/sub"]);

    // A key on the locked field does nothing.
    press(cx, handle, "escape");
    assert_eq!(tree_field(cx, handle).as_deref(), Some("job"));

    cx.run_until_parked();
    assert_eq!(tree_field(cx, handle), None);
    assert_eq!(tree(cx, handle), ["All notes", "job*", "job/sub"]);
    settle(cx);
    assert_eq!(tree(cx, handle), ["All notes", "job*", "job/sub"]);
    assert!(notes_dir.join("job/sub").is_dir());
}

#[gpui_kit::test]
fn a_folder_rename_on_a_clash_shows_the_name_on_disk(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let notes_dir = fixture.notes();
    fs::create_dir(notes_dir.join("job")).unwrap();
    fs::create_dir(notes_dir.join("work")).unwrap();
    fs::write(notes_dir.join("Top.md"), "# Top\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "ctrl-shift-l escape down down");
    assert_eq!(tree(cx, handle), ["All notes", "job", "work*"]);
    type_new_name(cx, handle, "job");
    keys_now(cx, handle, "enter");
    assert_eq!(tree(cx, handle), ["All notes", "job", "job 2*"]);
    settle(cx);
    assert_eq!(tree(cx, handle), ["All notes", "job", "job 2*"]);
    assert!(notes_dir.join("job 2").is_dir());
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
    assert_eq!(bars(cx, handle).len(), 1);
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
    assert_eq!(tree_field(cx, handle).as_deref(), Some("new"));
    assert_eq!(tree(cx, handle), ["All notes", "#old*"]);
    cx.run_until_parked();
    assert_eq!(tree_field(cx, handle), None);
    assert_eq!(tree(cx, handle), ["All notes", "#new*"]);
    settle(cx);
    assert_eq!(tree(cx, handle), ["All notes", "#new*"]);
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

    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(gpui_kit::component::input::SelectAll), cx);
    })
    .unwrap();
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
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(gpui_kit::component::input::SelectAll), cx);
    })
    .unwrap();
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
        View::Tree(Item::All)
    );
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
    assert_eq!(settings.sidebar.view, View::Tree(Item::All));
    assert!(!settings.sidebar.hidden);

    // The find bar of the editor lets Ctrl+Shift+F through.
    keys(cx, handle, "down enter");
    keys(cx, handle, "ctrl-f");
    assert_eq!(focus(cx, handle), "editor");
    keys(cx, handle, "ctrl-shift-f");
    assert_eq!(focus(cx, handle), "search");
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
    holm_markdown::Note::parse(&read(path))
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
    assert_eq!(options(cx), ["Bug list*", "Ideas"]);
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
    assert!(read(&gamma_path).ends_with("---\n# Gamma\n"));
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

    // A link to a missing note shows a quiet bar.
    keys(cx, handle, "down down ctrl-enter");
    assert_eq!(bars(cx, handle), [text::NOTE_GONE]);
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
    assert_eq!(editor_text(cx, handle), "");
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(files(&fixture.notes()), ["Untitled.md"]);
    let settings = Config::load(fixture.config()).settings().clone();
    assert_eq!(settings.last_note, Some(PathBuf::from("Untitled.md")));
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
    assert_eq!(bars(cx, handle), Vec::<String>::new());
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
    assert_eq!(bars(cx, handle), [text::DELETED_ON_DISK]);
    autosave(cx);
    assert!(!notes_dir.join("Plan.md").exists(), "no autosave");

    click(cx, handle, "keep-deleted");
    assert_eq!(bars(cx, handle), Vec::<String>::new());
    assert!(read(&notes_dir.join("Plan.md")).ends_with("# Plan\nbuy milk\nmine\n"));

    cx.simulate_input(handle, "!");
    fixture.delete_outside(cx, "Plan.md");
    assert_eq!(bars(cx, handle), [text::DELETED_ON_DISK]);
    click(cx, handle, "close-deleted");
    assert_eq!(editor_text(cx, handle), "# Goals\nread more\n");
    assert!(!notes_dir.join("Plan.md").exists());
    assert_eq!(bars(cx, handle), Vec::<String>::new());
}

#[gpui_kit::test]
fn the_link_keys_do_nothing_on_the_empty_pane_with_no_sidebar(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l");
    assert_eq!(sidebar(cx, handle), "hidden");
    assert!(empty_pane(cx, handle));

    keys(cx, handle, "ctrl-k ctrl-o");
    assert!(!cx.did_prompt_for_paths());
    let picker = screen(cx, handle, |window| {
        window.try_find("link-picker").is_some()
    });
    assert!(!picker);
    assert!(empty_pane(cx, handle));
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
    click(cx, handle, "close-deleted");
    autosave(cx);
    assert_eq!(editor_text(cx, handle), "# Goals\nread more\n");
    assert_eq!(files(&notes_dir), ["Goals.md", "Top.md"]);
    assert_eq!(times(), before);
}

#[gpui_kit::test]
fn the_delete_dialog_names_the_note_by_its_title(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-n");
    cx.simulate_input(handle, "# Plans");
    assert_eq!(files(&fixture.notes()), ["Untitled.md"]);

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
    keys(cx, handle, "ctrl-n");
    cx.simulate_input(handle, "# Plans");
    assert_eq!(files(&fixture.notes()), ["Untitled.md"]);

    cx.quit();
    assert_eq!(files(&fixture.notes()), ["Untitled.md"]);
    assert!(read(&fixture.notes().join("Untitled.md")).ends_with("\n# Plans\n"));
    let settings = Config::load(fixture.config()).settings().clone();
    assert_eq!(settings.last_note, Some(PathBuf::from("Untitled.md")));
}

#[gpui_kit::test]
fn a_window_close_keeps_the_name_of_the_open_note(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-n");
    cx.simulate_input(handle, "# Plans");

    cx.update_window(handle, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    assert_eq!(files(&fixture.notes()), ["Untitled.md"]);
    assert!(read(&fixture.notes().join("Untitled.md")).ends_with("\n# Plans\n"));
    let settings = Config::load(fixture.config()).settings().clone();
    assert_eq!(settings.last_note, Some(PathBuf::from("Untitled.md")));
}

#[path = "window_speed.rs"]
mod speed;
