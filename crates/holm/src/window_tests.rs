//! One test for each flow of the main window, with a real core on a temp folder.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests")]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use holm_core::{Core, Feed};
use tempfile::TempDir;

use super::{HolmWindow, Stage, Startup};
use crate::config::Config;
use crate::pane::NotePane;
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

fn open(cx: &mut TestAppContext, startup: Startup) -> (AnyWindowHandle, Entity<HolmWindow>) {
    let opened = cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(cx);
        gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| HolmWindow::new(startup, window, cx))
        })
    });
    let opened = opened.unwrap();
    cx.run_until_parked();
    opened
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

fn pane(cx: &TestAppContext, window: &Entity<HolmWindow>) -> Option<Entity<NotePane>> {
    cx.read(|cx| match &window.read(cx).stage {
        Stage::Open(pane) => Some(pane.clone()),
        _ => None,
    })
}

fn editor_text(cx: &TestAppContext, window: &Entity<HolmWindow>) -> String {
    let pane = pane(cx, window).expect("a note is open");
    cx.read(|cx| pane.read(cx).text(cx))
}

fn click(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) {
    cx.update_window(handle, |_, window, cx| window.click(id, cx))
        .unwrap();
    cx.run_until_parked();
}

fn notices(cx: &TestAppContext, window: &Entity<HolmWindow>) -> Vec<String> {
    cx.read(|cx| {
        window
            .read(cx)
            .notices
            .iter()
            .map(|notice| notice.text.to_string())
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
    let (handle, window) = open(cx, fixture.startup(None));
    assert!(cx.read(|cx| matches!(window.read(cx).stage, Stage::Ask)));

    click(cx, handle, "pick-folder");
    assert!(cx.did_prompt_for_paths());
    let notes = fixture.notes();
    cx.simulate_path_prompt_response(|_| Some(vec![notes]));
    cx.run_until_parked();

    assert_eq!(editor_text(cx, &window), "# Plans\n");
    let config = Config::load(fixture.config());
    assert_eq!(config.settings().notes_folder, Some(fixture.notes()));
}

#[gpui_kit::test]
fn the_first_start_can_make_the_default_folder(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let (handle, window) = open(cx, fixture.startup(None));

    click(cx, handle, "use-default");

    assert!(pane(cx, &window).is_some());
    let default = fixture.dir.path().join("Notes");
    assert!(default.join("Untitled.md").is_file());
    let config = Config::load(fixture.config());
    assert_eq!(config.settings().notes_folder, Some(default));
}

#[gpui_kit::test]
fn a_missing_folder_offers_to_pick_another(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::remove_dir(fixture.notes()).unwrap();
    let (handle, window) = open(cx, fixture.startup(None));
    let message = cx.read(|cx| match &window.read(cx).stage {
        Stage::Failed(message) => message.to_string(),
        _ => String::new(),
    });
    assert_eq!(message, text::folder_missing(&fixture.notes()));

    let other = fixture.dir.path().join("other");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("A.md"), "a\n").unwrap();
    click(cx, handle, "pick-folder");
    cx.simulate_path_prompt_response(|_| Some(vec![other]));
    cx.run_until_parked();

    assert_eq!(editor_text(cx, &window), "a\n");
}

#[gpui_kit::test]
fn typing_saves_the_note_with_its_line_breaks(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "---\nid: 01A\n---\r\none\r\ntwo\r\n").unwrap();
    let fixture = fixture.with_folder();
    let (handle, window) = open(cx, fixture.startup(None));
    assert_eq!(editor_text(cx, &window), "one\ntwo\n");

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
    let (_, window) = open(cx, fixture.startup(None));

    fixture.write_outside(cx, "Note.md", "new\n");

    assert_eq!(editor_text(cx, &window), "new\n");
    let pane = pane(cx, &window).unwrap();
    assert!(!cx.read(|cx| pane.read(cx).has_conflict()));
}

#[gpui_kit::test]
fn an_outside_change_with_unsaved_text_asks_to_reload_or_keep_mine(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "old\n").unwrap();
    let (handle, window) = open(cx, fixture.startup(None));
    let pane = pane(cx, &window).unwrap();

    cx.simulate_input(handle, "mine ");
    fixture.write_outside(cx, "Note.md", "theirs\n");
    assert!(cx.read(|cx| pane.read(cx).has_conflict()));
    autosave(cx);
    assert_eq!(read(&note), "theirs\n", "no autosave during the question");

    click(cx, handle, "keep-mine");
    assert!(read(&note).ends_with("\n---\nmine old\n"));
    assert!(!cx.read(|cx| pane.read(cx).has_conflict()));

    cx.simulate_input(handle, "x");
    fixture.write_outside(cx, "Note.md", "theirs again\n");
    click(cx, handle, "reload");
    assert_eq!(editor_text(cx, &window), "theirs again\n");
    assert!(!cx.read(|cx| pane.read(cx).has_conflict()));
}

#[gpui_kit::test]
fn a_broken_config_shows_a_bar_and_stays_as_it_is(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    fixture.write_config("version = [broken\n");
    let (handle, window) = open(cx, fixture.startup(None));
    assert_eq!(notices(cx, &window), [text::CONFIG_BROKEN]);

    click(cx, handle, "use-default");
    assert!(pane(cx, &window).is_some());
    assert_eq!(read(&fixture.config()), "version = [broken\n");

    click(cx, handle, "config-broken-close");
    assert_eq!(notices(cx, &window), Vec::<String>::new());
}

#[gpui_kit::test]
fn a_crash_in_the_last_run_shows_the_log_path(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let log = PathBuf::from("/state/holm/holm.log");
    let (_, window) = open(cx, fixture.startup(Some(log.clone())));
    assert_eq!(notices(cx, &window), [text::crashed(&log)]);
}
