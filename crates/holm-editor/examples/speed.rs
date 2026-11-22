//! Plays a fixed key list in a real window with a 1 MB note and prints the time of each frame.
//!
//! A frame time is the key handling plus the layout and paint of the frame.
//! The example draws each frame itself, so the time does not wait for the screen refresh.
//! The GPU work after the paint is not in it.
//! It fails when any frame takes 16 ms or more.
//!
//! ```sh
//! cargo run --release -p holm-editor --example speed
//! ```
//!
//! On a Wayland desktop it opens a Wayland window. Without `WAYLAND_DISPLAY` it uses X11.

use std::borrow::Cow;
use std::cell::RefCell;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::{
    App, AppContext, Bounds, Context, Entity, Focusable, IntoElement, Keystroke, ParentElement, Render, Styled, Window,
    WindowBounds, WindowOptions, div, px, rgb, size,
};
use holm_editor::Editor;

const LIMIT: Duration = Duration::from_millis(16);
const PAUSE: Duration = Duration::from_millis(50);

const KEYS: &[&str] = &[
    "down",
    "down",
    "down",
    "down",
    "down",
    "down",
    "down",
    "down",
    "right",
    "right",
    "right",
    "right",
    "h",
    "e",
    "l",
    "l",
    "o",
    "space",
    "w",
    "o",
    "r",
    "l",
    "d",
    "backspace",
    "backspace",
    "backspace",
    "enter",
    "up",
    "up",
    "up",
    "delete",
    "delete",
    "left",
    "left",
    "left",
    "down",
    "down",
    "down",
    "down",
    "down",
    "down",
    "down",
    "down",
    "down",
    "down",
    "down",
    "down",
    "x",
    "y",
    "z",
    "backspace",
    "up",
    "up",
    "up",
    "up",
    "up",
    "up",
    "right",
    "right",
    "enter",
    "backspace",
];

struct Host {
    editor: Entity<Editor>,
}

impl Render for Host {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x00fb_f7f0))
            .p(px(24.))
            .font_family("Fira Mono")
            .text_size(px(16.))
            .line_height(px(24.))
            .child(self.editor.clone())
    }
}

fn main() -> ExitCode {
    let times = Rc::new(RefCell::new(Vec::new()));
    let result = times.clone();
    gpui_kit::application().run(move |cx: &mut App| {
        let font = include_bytes!("../tests/fonts/FiraMono-Medium.ttf");
        if let Err(error) = cx.text_system().add_fonts(vec![Cow::Borrowed(font)]) {
            eprintln!("The font does not load: {error}");
            cx.quit();
            return;
        }
        let note = one_mb_note();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(900.), px(700.)),
                cx,
            ))),
            ..WindowOptions::default()
        };
        let window = cx.open_window(options, |window, cx| {
            let editor = cx.new(|cx| {
                let mut editor = Editor::new(cx);
                editor.set_text(&note, cx);
                editor
            });
            window.focus(&editor.focus_handle(cx), cx);
            cx.new(|_| Host { editor })
        });
        let Ok(window) = window else {
            eprintln!("The window does not open.");
            cx.quit();
            return;
        };
        let times = times.clone();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            for key in KEYS {
                let Ok(keystroke) = Keystroke::parse(key) else {
                    continue;
                };
                let _ = cx.update_window(window.into(), |_, window, cx| {
                    let start = Instant::now();
                    window.dispatch_keystroke(keystroke, cx);
                    window.draw(cx).clear(cx);
                    times.borrow_mut().push(start.elapsed());
                });
                cx.background_executor().timer(PAUSE).await;
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
    });

    let times = result.borrow();
    for (key, time) in KEYS.iter().zip(times.iter()) {
        println!("{key:>10} {:6.2} ms", time.as_secs_f64() * 1000.);
    }
    let Some(slowest) = times.iter().max() else {
        eprintln!("No frame was drawn.");
        return ExitCode::FAILURE;
    };
    println!(
        "{} keys, slowest frame {:.2} ms",
        times.len(),
        slowest.as_secs_f64() * 1000.
    );
    if *slowest >= LIMIT || times.len() < KEYS.len() {
        eprintln!("Too slow, or a key was not played. The limit is 16 ms.");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// About 1 MB of note text.
fn one_mb_note() -> String {
    const CHUNK: &str = include_str!("../tests/data/chunk.md");
    CHUNK.repeat(1_000_000 / CHUNK.len() + 1)
}
