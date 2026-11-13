//! Times of the app flows in a notes folder with many notes. It prints the times and checks nothing.
//!
//! The test window has no real text system, so the times leave out text shaping.
//!
//! ```sh
//! cargo test --release -p holm window_speed -- --ignored --nocapture
//! ```

use std::fs;
use std::path::Path;
use std::time::Instant;

use gpui_kit::TestAppContext;

use super::{Fixture, autosave, keys, open, screen, settle, type_new_name};

const NOTES: usize = 5_000;
const FOLDERS: usize = 50;
const TAGS: usize = 100;

/// [`NOTES`] notes in [`FOLDERS`] folders, each with two tags, two note links and about 2 KB of text.
fn many_notes(dir: &Path) {
    for folder in 0..FOLDERS {
        fs::create_dir(dir.join(format!("folder {folder}"))).unwrap();
    }
    let words = "a line of plain words that a person may write in a note about work and home. ";
    for note in 0..NOTES {
        let text = format!(
            "---\nid: N{note:05}\ntags: [tag{}, area/tag{}]\n---\n# Note {note}\n\n{}\n\n\
            See [this](holm:N{:05}) and [that](holm:N{:05}).\n",
            note % TAGS,
            (note / 7) % TAGS,
            words.repeat(24),
            (note + 1) % NOTES,
            (note + 2) % NOTES,
        );
        let path = dir
            .join(format!("folder {}", note % FOLDERS))
            .join(format!("Note {note}.md"));
        fs::write(path, text).unwrap();
    }
}

fn time(what: &str, run: impl FnOnce()) {
    let start = Instant::now();
    run();
    println!(
        "{what:>32} {:8.2} ms",
        start.elapsed().as_secs_f64() * 1000.
    );
}

#[gpui_kit::test]
#[ignore = "prints times for a person to read"]
fn window_speed(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    many_notes(&fixture.notes());

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
        time(&format!("search key {key}"), || {
            cx.simulate_input(handle, key);
            settle(cx);
            frame(cx);
        });
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
