//! Speed of the editor: a key press or a scroll, with the layout of the view.

#[path = "../tests/common/mod.rs"]
mod common;

use gpui_kit::{ScrollDelta, ScrollEvent, point, px};

fn main() {
    divan::main();
}

#[divan::bench(sample_count = 50)]
fn key_press_in_1_mb_note(bencher: divan::Bencher<'_, '_>) {
    let mut app = common::app();
    let note = common::one_mb_note();
    let (_editor, cx) = common::open(&mut app, &note, px(800.), px(600.));
    bencher.bench_local(|| cx.simulate_input("a"));
}

#[divan::bench(sample_count = 50)]
fn key_press_in_chip_lines(bencher: divan::Bencher<'_, '_>) {
    let mut app = common::app();
    let note = common::chip_lines_note();
    let (_editor, cx) = common::open(&mut app, &note, px(800.), px(600.));
    bencher.bench_local(|| cx.simulate_input("a"));
}

/// Each run scrolls one screen down, so the view lays out new lines.
#[divan::bench(sample_count = 100, sample_size = 1)]
fn scroll_in_1_mb_note(bencher: divan::Bencher<'_, '_>) {
    let mut app = common::app();
    let note = common::one_mb_note();
    let (_editor, cx) = common::open(&mut app, &note, px(800.), px(600.));
    bencher.bench_local(|| {
        cx.simulate_event(ScrollEvent {
            position: point(px(100.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-600.))),
            ..Default::default()
        });
    });
}
