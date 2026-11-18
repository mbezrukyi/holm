//! Times of the app flows in a notes folder with many notes. It prints the times and checks nothing.
//!
//! The test window has no real text system, so the times leave out text shaping.
//!
//! ```sh
//! cargo test --release -p holm window_speed -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, Keystroke, TestAppContext};
use holm_core::sample::{self, FOLDERS};

use super::Startup;
use crate::pane::NotePane;
use crate::window::{HolmWindow, Stage};

use super::{Fixture, autosave, keys, open, press, screen, settle, sidebar, type_new_name};

fn time(what: &str, run: impl FnOnce()) {
    let start = Instant::now();
    run();
    println!(
        "{what:>32} {:8.2} ms",
        start.elapsed().as_secs_f64() * 1000.
    );
}

/// Types `key` in the search field and draws the new rows. It prints the time on the UI thread
/// and the time from the key to the rows apart.
///
/// The test runs the background tasks on this thread too. The UI time leaves them out.
/// The test window draws a frame after each update, so the rows are on screen at the end.
fn search_key(cx: &mut TestAppContext, handle: AnyWindowHandle, key: &'static str) {
    let executor = cx.executor();
    let dispatcher = executor.dispatcher().as_test().unwrap();
    let start = Instant::now();
    let mut background = Duration::ZERO;
    cx.dispatch_keystroke(handle, Keystroke::parse(key).unwrap());
    loop {
        let before = Instant::now();
        while dispatcher.tick(true) {}
        background += before.elapsed();
        if !dispatcher.tick(false) {
            break;
        }
    }
    let total = start.elapsed();
    let ms = |time: Duration| time.as_secs_f64() * 1000.;
    println!(
        "{:>32} {:8.2} ms",
        format!("search key {key}, UI thread"),
        ms(total.saturating_sub(background))
    );
    println!(
        "{:>32} {:8.2} ms",
        format!("search key {key}, key to rows"),
        ms(total)
    );
}

#[gpui_kit::test]
#[ignore = "prints times for a person to read"]
fn window_speed(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    sample::notes_folder(&fixture.notes()).unwrap();

    let mut handle = None;
    time("start with a new index", || {
        handle = Some(open(cx, fixture.startup(None)));
    });
    let handle = handle.unwrap();
    let frame = |cx: &mut TestAppContext| screen(cx, handle, |_| ());
    time("frame", || frame(cx));

    time("key press", || {
        cx.simulate_input(handle, "x");
        frame(cx);
    });
    time("autosave and sidebar refresh", || {
        autosave(cx);
        settle(cx);
        frame(cx);
    });

    time("link picker", || {
        keys(cx, handle, "ctrl-k");
        frame(cx);
    });
    for key in ["n", "o", "t", "e", " ", "1", "2"] {
        time(&format!("link picker key {key:?}"), || {
            cx.simulate_input(handle, key);
            settle(cx);
            frame(cx);
        });
    }
    keys(cx, handle, "escape");

    keys(cx, handle, "ctrl-shift-f");
    for key in ["w", "o", "r", "k"] {
        search_key(cx, handle, key);
    }
    keys(cx, handle, "escape");

    time("quick open", || {
        keys(cx, handle, "ctrl-shift-o");
        frame(cx);
    });
    for key in ["n", "o", "t", "e", " ", "1", "2"] {
        time(&format!("quick open key {key:?}"), || {
            cx.simulate_input(handle, key);
            settle(cx);
            frame(cx);
        });
    }
    keys(cx, handle, "escape");

    // Quick open gives the focus back to the note list, after the search.
    keys(cx, handle, "escape down");
    type_new_name(cx, handle, "renamed");
    time("folder rename", || {
        cx.simulate_keystrokes(handle, "enter");
        settle(cx);
        frame(cx);
    });
}

/// Presses `key` and runs the tasks one at a time until the pane is `shown`, then draws one
/// frame. It prints the time.
///
/// The check reads the pane and draws no frame, so the time holds only the frames a window
/// would draw. The test runs the background tasks on this thread too, so the time holds a
/// write that runs.
fn key_until(
    what: &str,
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    key: &'static str,
    shown: impl Fn(&NotePane) -> bool,
) {
    let executor = cx.executor();
    let dispatcher = executor.dispatcher().as_test().unwrap();
    let pane = pane(cx, handle);
    time(what, || {
        cx.dispatch_keystroke(handle, Keystroke::parse(key).unwrap());
        while !cx.read(|cx| shown(pane.read(cx))) {
            assert!(dispatcher.tick(false), "{what}: it never shows");
        }
        screen(cx, handle, |_| ());
    });
}

/// The note pane of the open notes folder.
fn pane(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Entity<NotePane> {
    cx.update_window(handle, |root, _, cx| {
        let root = root.downcast::<gpui_kit::base::Root>().unwrap();
        let holm = root
            .read(cx)
            .view()
            .clone()
            .downcast::<HolmWindow>()
            .unwrap();
        match &holm.read(cx).stage {
            Stage::Open(notes) => notes.read(cx).pane().clone(),
            _ => panic!("the notes folder is not open"),
        }
    })
    .unwrap()
}

/// Opens the note with `number` in quick open, until the pane shows it.
fn quick_open(what: &str, cx: &mut TestAppContext, handle: AnyWindowHandle, number: &'static str) {
    press(cx, handle, "ctrl-shift-o");
    for digit in number.split("") {
        if !digit.is_empty() {
            press(cx, handle, digit);
        }
    }
    let title = format!("Note {number}");
    key_until(what, cx, handle, "enter", move |pane| {
        pane.shown_note()
            .is_some_and(|path| path.title() == title.as_str())
    });
}

/// Ctrl+N and an open of a note, while a tag rename of all notes runs. The write budget is zero,
/// so the rename does not end in the update of its key.
#[gpui_kit::test]
#[ignore = "prints times for a person to read"]
fn window_speed_during_a_tag_rename(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    sample::notes_folder(&fixture.notes()).unwrap();
    let startup = Startup {
        write_budget: Duration::ZERO,
        ..fixture.startup(None)
    };
    let handle = open(cx, startup);

    key_until(
        "Ctrl+N, empty line",
        cx,
        handle,
        "ctrl-n",
        NotePane::shows_new_note,
    );
    keys(cx, handle, "escape");
    quick_open("open a note, empty line", cx, handle, "4321");

    // The tree: All notes, the folders, then the tag `area`. Each note has a tag under it.
    keys(cx, handle, "ctrl-shift-l escape");
    keys(cx, handle, &["down"; FOLDERS + 1].join(" "));
    assert_eq!(sidebar(cx, handle), "tree #area");
    type_new_name(cx, handle, "zone");
    keys(cx, handle, "enter");
    press(cx, handle, "enter");
    key_until(
        "Ctrl+N, tag rename",
        cx,
        handle,
        "ctrl-n",
        NotePane::shows_new_note,
    );
    cx.run_until_parked();
    keys(cx, handle, "escape");

    keys(cx, handle, "ctrl-shift-l");
    assert_eq!(sidebar(cx, handle), "tree #zone");
    type_new_name(cx, handle, "area");
    keys(cx, handle, "enter");
    press(cx, handle, "enter");
    quick_open("open a note, tag rename", cx, handle, "1234");
}
