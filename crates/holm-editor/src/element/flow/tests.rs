use std::borrow::Cow;

use gpui_kit::{TestAppContext, TestDispatcher, black, font};

use super::*;

const SIZE: Pixels = px(16.);
const LINE: Pixels = px(20.);

/// Runs `test` in a window with the real text system and the repo font.
fn in_window(test: impl FnOnce(&mut Window)) {
    let text_system = gpui_kit::platform::current_platform(true).text_system();
    text_system
        .add_fonts(vec![
            Cow::Borrowed(include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/test_data/fonts/FiraMono-Medium.ttf"
            ))),
            // A narrower font than Fira Mono.
            Cow::Borrowed(include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/test_data/fonts/Inter-Regular.ttf"
            ))),
        ])
        .expect("the repo font loads");
    let mut app = TestAppContext::build_with_text_system(TestDispatcher::new(0), None, text_system);
    app.add_empty_window().update(|window, _| test(window));
}

fn run(len: usize) -> TextRun {
    TextRun {
        len,
        font: font("Fira Mono"),
        color: black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

fn shape(text: &str, font_size: Pixels, window: &Window) -> ShapedLine {
    window
        .text_system()
        .shape_line(text.to_owned().into(), font_size, &[run(text.len())], None)
}

/// `before`, a chip with `name` and `detail`, then `after`, in rows of `wrap`. It also gives the screen text of the
/// chip.
fn flow(
    (before, name, detail, after): (&str, &str, Option<&str>, &str),
    wrap: Pixels,
    window: &Window,
) -> (FlowLine, Range<ScreenOffset>) {
    let text = format!("{before}\u{fffc}{after}");
    let range = ScreenOffset::new(before.len())..ScreenOffset::new(text.len() - after.len());
    let look = run(0);
    let chip = ChipBox::new(
        range.clone(),
        Icon::File,
        (name, &look),
        detail.map(|detail| (detail.to_owned(), &look)),
        SIZE,
        window,
    );
    let line = FlowLine::new(
        text.clone().into(),
        &[run(text.len())],
        vec![chip],
        &font("Fira Mono"),
        SIZE,
        wrap,
        window,
    );
    (line, range)
}

fn x(line: &FlowLine, index: ScreenOffset) -> Pixels {
    line.position_for_index(index, LINE).expect("in the line").x
}

fn close(a: Pixels, b: Pixels) -> bool {
    (a - b).abs() < px(0.01)
}

#[test]
fn a_cutter_gives_the_parts_of_a_split() {
    in_window(|window| {
        let line = shape("one two three four five", SIZE, window);
        let same = |part: &ShapedLine, range: Range<usize>| {
            let (head, _) = line.split_at(range.end);
            let split = head.split_at(range.start).1;
            assert_eq!(part.len(), split.len());
            assert!(close(part.width(), split.width()));
            for at in 0..=part.len() {
                assert!(close(part.x_for_index(at), split.x_for_index(at)));
            }
        };
        let mut cutter = Cutter::new(1);
        for range in [0..4, 4..8, 8..14, 14..line.len()] {
            same(&cutter.cut(0, &line, range.clone()), range);
        }
        for range in [4..8, 0..4, 14..line.len(), 8..19] {
            same(&cutter.cut(0, &line, range.clone()), range);
        }
        assert!(close(cutter.cut(0, &line, 0..line.len()).width(), line.width()));
    });
}

#[test]
fn a_long_name_loses_chars_in_the_middle() {
    let fits = |name: &str| name.chars().count() <= 12;
    assert_eq!(shorten("short.txt", fits), "short.txt");
    assert_eq!(shorten("aaaaaaaaaaaaaaaa.txt", fits), "aaaa\u{2026}aaa.txt");
    assert_eq!(shorten("abcdefghijklmnopq", fits), "abcdefgh\u{2026}opq");
}

#[test]
fn a_chip_box_has_the_width_of_its_parts() {
    in_window(|window| {
        let (line, chip) = flow(("See ", "report.pdf", Some("48 KB"), " now"), px(600.), window);
        let parts = SIZE * (CHIP_PAD + CHIP_ICON + CHIP_ICON_GAP)
            + shape("report.pdf", SIZE, window).width()
            + SIZE * CHIP_DETAIL_GAP
            + shape("48 KB", SIZE * DETAIL_SCALE, window).width()
            + SIZE * CHIP_PAD;
        assert!(close(x(&line, chip.start), shape("See ", SIZE, window).width()));
        assert!(close(x(&line, chip.end) - x(&line, chip.start), parts));
        let [paint] = line.chip_paints(LINE).try_into().ok().expect("one piece");
        assert!(close(paint.bounds.size.width, parts));
        assert_eq!(line.size(LINE).height, LINE);
    });
}

#[test]
fn a_line_wraps_before_a_chip_that_does_not_fit() {
    in_window(|window| {
        let (one_row, _) = flow(("aaaa ", "report.pdf", None, " b"), px(600.), window);
        let chip_width = one_row.width() - shape("aaaa  b", SIZE, window).width();
        let wrap = chip_width + SIZE * 2.;
        let (line, chip) = flow(("aaaa ", "report.pdf", None, " b"), wrap, window);
        let [paint] = line.chip_paints(LINE).try_into().ok().expect("one piece");
        assert!(paint.icon.is_some());
        assert_eq!(paint.bounds.origin.x, px(0.));
        assert!(paint.bounds.origin.y > LINE);
        assert!(close(paint.bounds.size.width, chip_width));
        assert_eq!(line.position_for_index(chip.end, LINE).map(|at| at.y), Some(LINE));

        // A chip that only just fits a row moves whole to the next row.
        let (line, _) = flow(("aaaa ", "report.pdf", None, " b"), chip_width + px(1.), window);
        let [paint] = line.chip_paints(LINE).try_into().ok().expect("one piece");
        assert!(paint.icon.is_some() && paint.bounds.origin.y > LINE);
    });
}

#[test]
fn a_chip_wraps_at_a_space_in_its_name_with_padding_on_each_piece() {
    in_window(|window| {
        let detail = Some("2 KB");
        let (whole, _) = flow(("", "my report.pdf", detail, ""), px(600.), window);
        let (line, _) = flow(("", "my report.pdf", detail, ""), whole.width() - px(1.), window);
        let [first, last] = line.chip_paints(LINE).try_into().ok().expect("two pieces");
        let pad = SIZE * CHIP_PAD;
        assert!(first.icon.is_some() && first.detail.is_none());
        assert!(last.icon.is_none() && last.detail.is_some());
        assert_eq!(first.name.0.text.as_ref(), "my ");
        assert_eq!(last.name.0.text.as_ref(), "report.pdf");
        assert!(close(
            first.bounds.size.width,
            SIZE * (CHIP_PAD + CHIP_ICON + CHIP_ICON_GAP) + shape("my ", SIZE, window).width() + pad
        ));
        assert!(close(last.name.1.x, pad));
        assert_eq!(line.size(LINE).height, LINE * 2.);
    });
}

#[test]
fn the_caret_and_a_click_after_a_chip_land_on_the_right_offset() {
    in_window(|window| {
        let (line, chip) = flow(("a", "report.pdf", Some("1 KB"), "bc"), px(600.), window);
        let b = shape("b", SIZE, window).width();
        let after = x(&line, chip.end);
        assert!(close(x(&line, chip.end + 1), after + b));
        let click = |x: Pixels| line.closest_index_for_position(point(x, LINE / 2.), LINE);
        assert_eq!(click(after + b * 0.3), Ok(chip.end));
        assert_eq!(click(after + b * 0.7), Ok(chip.end + 1));
        // A click anywhere on the chip gives its start.
        assert_eq!(click(after - px(1.)), Ok(chip.start));
        assert_eq!(click(x(&line, chip.start) + px(1.)), Ok(chip.start));
    });
}

/// A line laid out at its narrowest width breaks only between pieces: after a space, after a hyphen, or at a space
/// in the name of a chip.
#[test]
fn a_line_at_its_narrowest_width_breaks_no_word() {
    in_window(|window| {
        let widths = |text: &str, runs: &[TextRun], base: &Font| {
            FlowLine::widths(text.into(), runs, Vec::new(), base, SIZE, window)
        };
        let fira = font("Fira Mono");
        let inter = font("Inter");
        let code = |len: usize| run(len);
        let text = |len: usize| TextRun {
            len,
            font: inter.clone(),
            ..run(0)
        };
        let cases: [(&str, Vec<TextRun>, &Font); 5] = [
            ("Axis", vec![run(4)], &fira),
            ("Property one long word", vec![run(22)], &fira),
            // Code in a wider font than the base font of the wrapper.
            ("see justify-content now", vec![text(4), code(15), text(4)], &inter),
            ("flex-direction", vec![code(14)], &inter),
            // Text in a narrower font than the base font of the wrapper.
            ("Distributes leftover space", vec![text(26)], &fira),
        ];
        for (line_text, runs, base) in &cases {
            let (min, max) = widths(line_text, runs, base);
            assert!(min <= max, "{line_text}");
            let line = FlowLine::new((*line_text).into(), runs, Vec::new(), base, SIZE, min, window);
            for row in &line.rows {
                let end = row.range.bytes().end;
                assert!(
                    end == line_text.len() || matches!(&line_text[end - 1..end], " " | "-"),
                    "a row of {min:?} ends in {:?}",
                    &line_text[row.range.bytes()]
                );
            }
        }
        let (min, _) = widths("Axis", &[run(4)], &fira);
        assert!(close(min, shape("Axis", SIZE, window).width()));

        let detail = Some("2 KB");
        let (whole, _) = flow(("", "my report.pdf", detail, ""), px(600.), window);
        let text = "\u{fffc}";
        let look = run(0);
        let chip = ChipBox::new(
            ScreenOffset::default()..ScreenOffset::new(text.len()),
            Icon::File,
            ("my report.pdf", &look),
            detail.map(|detail| (detail.to_owned(), &look)),
            SIZE,
            window,
        );
        let (min, max) = FlowLine::widths(text.into(), &[run(text.len())], vec![chip], &fira, SIZE, window);
        assert!(close(max, whole.width()));
        assert!(min < max);
        let (line, _) = flow(("", "my report.pdf", detail, ""), min, window);
        let names: Vec<&str> = line
            .chip_paints(LINE)
            .iter()
            .map(|paint| paint.name.0.text.as_ref())
            .collect();
        assert_eq!(names, ["my ", "report.pdf"]);
    });
}

/// Code in a wider font than the text makes rows wider than the wrapper thinks. A row after the first one of a wrap
/// pass that is too wide still breaks between words.
#[test]
fn a_too_wide_row_with_code_breaks_between_words() {
    in_window(|window| {
        let text = "items scale with width or height. It uses modern CSS: gap, logical \
                    properties, clamp(), aspect-ratio, dynamic viewport units, and queries.";
        let inter = font("Inter");
        let mut runs = Vec::new();
        let mut at = 0;
        for code in ["gap", "clamp()", "aspect-ratio"] {
            let start = text.find(code).expect("the code is in the text");
            runs.push(TextRun {
                len: start - at,
                font: inter.clone(),
                ..run(0)
            });
            runs.push(run(code.len()));
            at = start + code.len();
        }
        runs.push(TextRun {
            len: text.len() - at,
            font: inter.clone(),
            ..run(0)
        });
        let word = text.find("aspect-ratio").expect("the word is in the text");
        let after_hyphen = word + "aspect-".len();
        for half in 300_u16..1400 {
            let wrap = px(f32::from(half) / 2.);
            let line = FlowLine::new(text.into(), &runs, Vec::new(), &inter, SIZE, wrap, window);
            for row in &line.rows {
                let end = row.range.bytes().end;
                assert!(
                    end <= word || end >= word + "aspect-ratio".len() || end == after_hyphen,
                    "a row of {wrap:?} ends in {:?}",
                    &text[word..end]
                );
            }
        }
    });
}
