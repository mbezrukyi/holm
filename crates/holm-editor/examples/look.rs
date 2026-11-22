//! Opens a window with a note that has every element, to check the look by eye.
//!
//! ```sh
//! cargo run -p holm-editor --example look -- [--dark] [--tail] [--font NAME] [--size PX] [--line PX]
//! ```
//!
//! `--tail` shows the note from its code block on.

use std::path::PathBuf;

use gpui_kit::{
    App, AppContext, Bounds, Context, Entity, Focusable, IntoElement, ParentElement, Render, Styled, TitlebarOptions,
    Window, WindowBounds, WindowOptions, div, px, size,
};
use holm_editor::{Appearance, Editor};

struct Host {
    editor: Entity<Editor>,
    font: String,
    size: f32,
    line: f32,
}

impl Render for Host {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .font_family(self.font.clone())
            .text_size(px(self.size))
            .line_height(px(self.line))
            .child(self.editor.clone())
    }
}

fn main() {
    let mut dark = false;
    let mut tail = false;
    let mut font = "DejaVu Serif".to_owned();
    let mut text_size = 15.;
    let mut line = 25.;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dark" => dark = true,
            "--tail" => tail = true,
            "--font" => font = args.next().unwrap_or(font),
            "--size" => {
                text_size = args.next().and_then(|v| v.parse().ok()).unwrap_or(text_size);
            }
            "--line" => line = args.next().and_then(|v| v.parse().ok()).unwrap_or(line),
            _ => {}
        }
    }
    gpui_kit::application().run(move |cx: &mut App| {
        let title = format!("{font} {text_size}/{line}");
        let options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some(title.into()),
                ..TitlebarOptions::default()
            }),
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(900.), px(1000.)),
                cx,
            ))),
            ..WindowOptions::default()
        };
        let opened = cx.open_window(options, |window, cx| {
            let editor = cx.new(|cx| {
                let mut editor = Editor::new(cx);
                let note = include_str!("look.md");
                let from = if tail { note.find("## Code").unwrap_or(0) } else { 0 };
                editor.set_text(&note[from..], cx);
                editor.set_folder(Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data")), cx);
                let appearance = if dark { Appearance::Dark } else { Appearance::Light };
                editor.set_appearance(appearance, cx);
                editor
            });
            window.focus(&editor.focus_handle(cx), cx);
            cx.new(|_| Host {
                editor,
                font,
                size: text_size,
                line,
            })
        });
        if opened.is_err() {
            eprintln!("The window does not open.");
            cx.quit();
        }
    });
}
