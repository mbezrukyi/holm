use gpui_kit::{ScrollDelta, ScrollEvent, point, px};

fn main() {
    divan::main();
}

#[divan::bench(sample_count = 50)]
fn key_press_in_1_mb_note(bencher: divan::Bencher<'_, '_>) {
    let mut app = holm_editor::testing::app();
    let note = one_mb_note();
    let (_editor, cx) = holm_editor::testing::open(&mut app, &note, px(800.), px(600.));
    bencher.bench_local(|| cx.simulate_input("a"));
}

#[divan::bench(sample_count = 50)]
fn key_press_in_chip_lines(bencher: divan::Bencher<'_, '_>) {
    let mut app = holm_editor::testing::app();
    let note = chip_lines_note();
    let (_editor, cx) = holm_editor::testing::open(&mut app, &note, px(800.), px(600.));
    bencher.bench_local(|| cx.simulate_input("a"));
}

/// Each run scrolls one screen down, so the view lays out new lines.
#[divan::bench(sample_count = 100, sample_size = 1)]
fn scroll_in_1_mb_note(bencher: divan::Bencher<'_, '_>) {
    let mut app = holm_editor::testing::app();
    let note = one_mb_note();
    let (_editor, cx) = holm_editor::testing::open(&mut app, &note, px(800.), px(600.));
    bencher.bench_local(|| {
        cx.simulate_event(ScrollEvent {
            position: point(px(100.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-600.))),
            ..Default::default()
        });
    });
}

/// A part of a note with every element, each block closed.
const CHUNK: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/test_data/notes/chunk.md"));

fn one_mb_note() -> String {
    CHUNK.repeat(1_000_000 / CHUNK.len() + 1)
}

/// The lines wrap.
fn chip_lines_note() -> String {
    let part = "some words before a file ![report](attachments/report.pdf) and more words after it, ";
    let line = part.repeat(12);
    vec![line.trim_end(); 200].join("\n\n")
}
