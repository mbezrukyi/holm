use std::time::Duration;

use super::*;
use display::plain_chip;
use gpui_kit::px;
use holm_markdown::list::Move;
use proptest::prelude::*;

fn now() -> Instant {
    Instant::now()
}

fn model_at(text: &str, cursor: usize) -> Model {
    let mut model = Model::new(text);
    model.set_cursor(SourceOffset::new(cursor), Selection::Collapse);
    model
}

fn row_caret(offset: usize) -> Caret {
    let goal_row = GoalRow {
        direction: Direction::Down,
        selection: Selection::Collapse,
    };
    Caret::at(SourceOffset::new(offset))
        .with_affinity(Affinity::RowStart)
        .with_goal_x(px(40.))
        .with_goal_row(Some(goal_row))
}

#[test]
fn a_row_move_keeps_the_affinity_and_the_goals() {
    let mut model = Model::new("one two three");
    model.set_caret(row_caret(4), Selection::Collapse);
    assert_eq!(model.caret(), row_caret(4));
}

#[test]
fn any_other_cursor_change_drops_the_affinity_and_the_goals() {
    let mut model = Model::new("one two three");
    model.set_caret(row_caret(4), Selection::Collapse);
    model.move_right(Selection::Collapse);
    assert_eq!(model.caret(), Caret::at(SourceOffset::new(5)));

    model.set_caret(row_caret(4), Selection::Collapse);
    model.type_text("x", now());
    assert_eq!(model.caret(), Caret::at(SourceOffset::new(5)));

    model.set_caret(row_caret(4), Selection::Collapse);
    model.undo();
    assert_eq!(model.caret(), Caret::at(SourceOffset::new(4)));
}

#[test]
fn a_caret_off_a_char_boundary_drops_the_affinity_and_the_goals() {
    let mut model = Model::new("aé");
    model.set_caret(row_caret(2), Selection::Collapse);
    assert_eq!(model.caret(), Caret::at(SourceOffset::new(1)));
}

#[test]
fn the_goal_row_goes_once_and_the_goal_x_stays() {
    let mut model = Model::new("one two three");
    model.set_caret(row_caret(4), Selection::Collapse);
    assert!(model.take_goal_row().is_some());
    assert_eq!(model.take_goal_row(), None);
    assert_eq!(model.caret().goal_x(), Some(px(40.)));
    assert_eq!(model.caret().affinity(), Affinity::RowStart);
}

#[test]
fn typing_replaces_the_selection() {
    let mut model = Model::new("one two");
    model.select(SourceOffset::range(4..7));
    model.type_text("three", now());
    assert_eq!(model.text(), "one three");
    assert_eq!(model.cursor(), SourceOffset::new(9));
    assert!(model.selection().is_empty());
}

#[test]
fn backspace_and_delete_remove_whole_chars() {
    let mut model = model_at("aé🙂b", 7);
    assert!(model.backspace(Step::Char, now()));
    assert_eq!(model.text(), "aéb");
    assert_eq!(model.cursor(), SourceOffset::new(3));
    assert!(model.backspace(Step::Char, now()));
    assert_eq!(model.text(), "ab");
    assert!(model.delete(Step::Char, now()));
    assert_eq!(model.text(), "a");
    assert!(!model.delete(Step::Char, now()));
    model.set_cursor(SourceOffset::new(0), Selection::Collapse);
    assert!(!model.backspace(Step::Char, now()));
}

#[test]
fn arrows_collapse_a_selection() {
    let mut model = Model::new("abcdef");
    model.select(SourceOffset::range(2..4));
    model.move_left(Selection::Collapse);
    assert_eq!(model.selection(), SourceOffset::range(2..2));
    model.move_right(Selection::Extend);
    model.move_right(Selection::Extend);
    assert_eq!(model.selection(), SourceOffset::range(2..4));
    model.move_right(Selection::Collapse);
    assert_eq!(model.selection(), SourceOffset::range(4..4));
}

#[test]
fn word_moves_skip_spaces_and_one_run_of_chars() {
    let mut model = Model::new("one, two\nthree");
    model.move_word_right(Selection::Collapse);
    assert_eq!(model.cursor(), SourceOffset::new(3));
    model.move_word_right(Selection::Collapse);
    assert_eq!(model.cursor(), SourceOffset::new(4));
    model.move_word_right(Selection::Collapse);
    assert_eq!(model.cursor(), SourceOffset::new(8));
    model.move_word_right(Selection::Collapse);
    assert_eq!(model.cursor(), SourceOffset::new(9));
    model.move_word_left(Selection::Extend);
    assert_eq!(model.selection(), SourceOffset::range(8..9));
    model.move_word_left(Selection::Collapse);
    assert_eq!(model.cursor(), SourceOffset::new(5));
}

#[test]
fn word_keys_remove_a_word() {
    let mut model = model_at("one two", 7);
    assert!(model.backspace(Step::Word, now()));
    assert_eq!(model.text(), "one ");
    model.set_cursor(SourceOffset::new(0), Selection::Collapse);
    assert!(model.delete(Step::Word, now()));
    assert_eq!(model.text(), " ");
}

#[test]
fn a_double_click_picks_a_word_and_a_triple_click_a_line() {
    let model = Model::new("say héllo!\nnext");
    assert_eq!(model.word_at(SourceOffset::new(6)), SourceOffset::range(4..10));
    assert_eq!(model.word_at(SourceOffset::new(4)), SourceOffset::range(4..10));
    assert_eq!(model.word_at(SourceOffset::new(11)), SourceOffset::range(10..11));
    assert_eq!(model.word_at(SourceOffset::new(3)), SourceOffset::range(3..4));
    assert_eq!(model.line_at(SourceOffset::new(2)), SourceOffset::range(0..12));
    assert_eq!(model.line_at(SourceOffset::new(13)), SourceOffset::range(12..16));
    assert_eq!(
        Model::new("a\n\nb").word_at(SourceOffset::new(2)),
        SourceOffset::range(2..2)
    );
}

#[test]
fn the_text_has_only_lf_line_breaks() {
    let mut model = Model::new("a\r\nb\rc");
    assert_eq!(model.text(), "a\nb\nc");
    model.set_cursor(SourceOffset::new(5), Selection::Collapse);
    model.insert("\r\nd");
    assert_eq!(model.text(), "a\nb\nc\nd");
}

#[test]
fn undo_takes_back_typing_one_word_at_a_time() {
    let start = now();
    let mut model = Model::new("");
    for (ix, c) in "one two".chars().enumerate() {
        model.type_text(&c.to_string(), start + Duration::from_millis(ix as u64 * 100));
    }
    assert!(model.undo());
    assert_eq!(model.text(), "one ");
    assert_eq!(model.cursor(), SourceOffset::new(4));
    assert!(model.undo());
    assert_eq!(model.text(), "");
    assert!(!model.undo());
    assert!(model.redo());
    assert!(model.redo());
    assert_eq!(model.text(), "one two");
    assert_eq!(model.cursor(), SourceOffset::new(7));
    assert!(!model.redo());
}

#[test]
fn a_pause_starts_a_new_undo_step() {
    let start = now();
    let mut model = Model::new("");
    model.type_text("a", start);
    model.type_text("b", start + Duration::from_millis(500));
    model.type_text("c", start + Duration::from_millis(1600));
    model.undo();
    assert_eq!(model.text(), "ab");
}

#[test]
fn removals_join_one_undo_step() {
    let start = now();
    let mut model = model_at("abcd", 2);
    model.backspace(Step::Char, start);
    model.backspace(Step::Char, start);
    model.delete(Step::Char, start);
    assert_eq!(model.text(), "d");
    model.undo();
    assert_eq!(model.text(), "abcd");
    assert_eq!(model.cursor(), SourceOffset::new(2));
}

#[test]
fn undo_puts_back_the_cursor_and_the_selection() {
    let mut model = Model::new("one two");
    model.select(SourceOffset::range(4..7));
    model.toggle_format(Format::Bold);
    model.set_cursor(SourceOffset::new(0), Selection::Collapse);
    model.undo();
    assert_eq!(model.text(), "one two");
    assert_eq!(model.selection(), SourceOffset::range(4..7));
}

#[test]
fn a_new_edit_clears_the_redo_steps() {
    let mut model = Model::new("");
    model.insert("a");
    model.undo();
    model.insert("b");
    assert!(!model.redo());
    assert_eq!(model.text(), "b");
}

#[test]
fn a_reload_is_one_undo_step_and_keeps_the_cursor_near() {
    let mut model = model_at("one two three", 10);
    assert!(model.reload("one 2 three"));
    assert_eq!(model.cursor(), SourceOffset::new(8));
    assert!(!model.reload("one 2 three"));
    model.undo();
    assert_eq!(model.text(), "one two three");
    assert_eq!(model.cursor(), SourceOffset::new(10));
    assert!(model.reload("é"));
    assert!(model.reload("è"));
    assert_eq!(model.text(), "è");
}

#[test]
fn lines_have_no_line_break() {
    let model = Model::new("one\ntwo\n");
    assert_eq!(model.line_count(), 3);
    assert_eq!(model.line_range(LineIndex(0)), SourceOffset::range(0..3));
    assert_eq!(model.line_range(LineIndex(1)), SourceOffset::range(4..7));
    assert_eq!(model.line_range(LineIndex(2)), SourceOffset::range(8..8));
    assert_eq!(model.line_of(SourceOffset::new(3)), LineIndex(0));
    assert_eq!(model.line_of(SourceOffset::new(4)), LineIndex(1));
}

#[test]
fn syntax_hides_away_from_the_cursor_line() {
    let model = model_at("a **b** c\n\nnext", 12);
    let line = model.display_line(LineIndex(0), &mut plain_chip);
    assert_eq!(line.syntax, Syntax::Hidden);
    assert_eq!(line.shown.text, "a b c");
    assert_eq!(line.shown.runs.len(), 3);
    assert!(line.shown.runs[1].look.styles.contains(Styles::STRONG));
    assert_eq!(line.to_source(ScreenOffset::new(2)), SourceOffset::new(4));
    assert_eq!(line.to_source(ScreenOffset::new(3)), SourceOffset::new(7));
    assert_eq!(line.to_display(SourceOffset::new(4)), ScreenOffset::new(2));
    assert_eq!(line.to_display(SourceOffset::new(2)), ScreenOffset::new(2));
}

#[test]
fn syntax_shows_on_the_cursor_line() {
    let model = model_at("a **b** c\n\nnext", 1);
    let line = model.display_line(LineIndex(0), &mut plain_chip);
    assert_eq!(line.syntax, Syntax::Shown);
    assert_eq!(line.shown.text, "a **b** c");
    assert_eq!(line.shown.runs[1].look.kind, TextKind::Syntax);
    assert_eq!(line.to_source(ScreenOffset::new(5)), SourceOffset::new(5));
}

#[test]
fn a_selection_shows_syntax_on_each_line_it_touches() {
    let mut model = Model::new("**a**\n**b**\n**c**");
    model.select(SourceOffset::range(2..8));
    assert_eq!(model.display_line(LineIndex(0), &mut plain_chip).syntax, Syntax::Shown);
    assert_eq!(model.display_line(LineIndex(1), &mut plain_chip).syntax, Syntax::Shown);
    assert_eq!(model.display_line(LineIndex(2), &mut plain_chip).syntax, Syntax::Hidden);
}

#[test]
fn embedded_images_hide_and_are_listed() {
    let model = model_at("top\n\n![cat](cat.png)\n", 0);
    let line = model.display_line(LineIndex(2), &mut |_| Embed::Image);
    assert_eq!(line.shown.text, "");
    assert_eq!(line.images, ["cat.png"]);
    assert_eq!(line.shown.chips, []);
    assert_eq!(line.to_source(ScreenOffset::new(0)), line.source().start);
}

#[test]
fn an_embedded_file_shows_as_a_chip_in_its_line() {
    let text = "top\n\nSee ![f](docs/report.pdf) now\n";
    let model = model_at(text, 0);
    let line = model.display_line(LineIndex(2), &mut |_| Embed::Chip);
    assert_eq!(line.images, [] as [String; 0]);
    let [chip] = line.shown.chips.as_slice() else {
        panic!("one chip");
    };
    assert_eq!(chip.target, "docs/report.pdf");
    assert_eq!(line.shown.text, "See \u{fffc} now");
    assert_eq!(&line.shown.text[..chip.range.bytes().start], "See ");
    assert_eq!(&line.shown.text[chip.range.bytes().end..], " now");
    let bang = text.find('!').unwrap_or_default();
    assert_eq!(line.to_source(chip.range.start), SourceOffset::new(bang));
    assert_eq!(line.to_display(SourceOffset::new(bang + 3)), chip.range.start);
    assert_eq!(
        line.to_source(chip.range.end),
        SourceOffset::new(text.find(" now").unwrap_or_default())
    );
}

#[test]
fn the_right_edge_of_a_chip_at_the_end_of_a_line_maps_to_the_end_of_its_link() {
    for text in ["![](a)\nb", "c ![](a\n)"] {
        let model = model_at(text, text.len());
        let line = model.display_line(LineIndex(0), &mut plain_chip);
        let [chip] = line.shown.chips.as_slice() else {
            panic!("one chip in {text:?}");
        };
        assert_eq!(chip.range.end, ScreenOffset::new(line.shown.text.len()), "{text:?}");
        let end = line.source().end;
        assert_eq!(line.to_source(chip.range.end), end, "{text:?}");
        assert_eq!(line.to_display(end), chip.range.end, "{text:?}");
        let bang = text.find('!').unwrap_or_default();
        assert_eq!(line.to_source(chip.range.start), SourceOffset::new(bang), "{text:?}");
        assert_eq!(line.to_display(SourceOffset::new(bang)), chip.range.start, "{text:?}");
    }
}

#[test]
fn a_line_with_the_cursor_shows_the_chip_as_markdown() {
    let model = model_at("![f](report.pdf)\n", 3);
    let line = model.display_line(LineIndex(0), &mut plain_chip);
    assert_eq!(line.shown.text, "![f](report.pdf)");
    assert_eq!(line.shown.chips, []);
}

#[test]
fn heading_lines_know_their_level() {
    let model = model_at("x\n\n## Plans\n", 0);
    let line = model.display_line(LineIndex(2), &mut plain_chip);
    assert_eq!(line.heading, Some(2));
    assert_eq!(line.shown.text, "Plans");
}

#[test]
fn splices_join_into_one_changed_range() {
    let mut model = Model::new("abcdefgh");
    model.splice(SourceOffset::range(6..7), "XYZ");
    model.splice(SourceOffset::range(1..3), "");
    assert_eq!(model.text(), "adefXYZh");
    assert_eq!(model.changed, Some((SourceOffset::range(1..7), SourceOffset::new(7))));
    model.reparse();
    assert_eq!(model.changed, None);
}

#[test]
fn utf16_offsets_round_trip() {
    let model = Model::new("a🙂b");
    assert_eq!(model.offset_to_utf16(SourceOffset::new(5)), Utf16Offset(3));
    assert_eq!(model.utf16_to_offset(Utf16Offset(3)), SourceOffset::new(5));
}

fn text() -> impl Strategy<Value = String> {
    proptest::string::string_regex("([a-c é🙂*_=`#>\\[\\]()!\n-]){0,40}").unwrap()
}

#[derive(Clone, Debug)]
enum Op {
    Type(String, u64),
    Insert(String),
    Backspace(Step),
    Delete(Step),
    Left(Selection),
    Right(Selection),
    Cursor(usize, Selection),
    Format(usize),
    Heading(u8),
    Newline,
    Indent(Move),
    Undo,
    Redo,
}

const FORMATS: [Format; 6] = [
    Format::Bold,
    Format::Italic,
    Format::Underline,
    Format::Strikethrough,
    Format::Highlight,
    Format::Code,
];

fn selection() -> impl Strategy<Value = Selection> {
    prop_oneof![Just(Selection::Collapse), Just(Selection::Extend)]
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![Just(Step::Char), Just(Step::Word)]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (text(), 0..1500u64).prop_map(|(text, ms)| Op::Type(text, ms)),
        text().prop_map(Op::Insert),
        step().prop_map(Op::Backspace),
        step().prop_map(Op::Delete),
        selection().prop_map(Op::Left),
        selection().prop_map(Op::Right),
        (0..60usize, selection()).prop_map(|(at, selection)| Op::Cursor(at, selection)),
        (0..FORMATS.len()).prop_map(Op::Format),
        (0..=6u8).prop_map(Op::Heading),
        Just(Op::Newline),
        prop_oneof![Just(Move::In), Just(Move::Out)].prop_map(Op::Indent),
        Just(Op::Undo),
        Just(Op::Redo),
    ]
}

/// The clock moves on by the pause in each [`Op::Type`].
fn run(model: &mut Model, op: Op, clock: &mut Instant) {
    match op {
        Op::Type(text, ms) => {
            *clock += Duration::from_millis(ms);
            model.type_text(&text, *clock);
        }
        Op::Insert(text) => {
            model.insert(&text);
        }
        Op::Backspace(step) => {
            model.backspace(step, *clock);
        }
        Op::Delete(step) => {
            model.delete(step, *clock);
        }
        Op::Left(selection) => model.move_left(selection),
        Op::Right(selection) => model.move_right(selection),
        Op::Cursor(at, selection) => model.set_cursor(SourceOffset::new(at), selection),
        Op::Format(ix) => {
            model.toggle_format(FORMATS[ix]);
        }
        Op::Heading(level) => {
            model.toggle_heading(level);
        }
        Op::Newline => {
            model.newline(*clock);
        }
        Op::Indent(key) => {
            model.relist(key);
        }
        Op::Undo => {
            model.undo();
        }
        Op::Redo => {
            model.redo();
        }
    }
}

proptest! {
    #[test]
    fn the_cursor_stays_on_a_char_boundary(start in text(), ops in prop::collection::vec(op(), 0..30)) {
        let mut model = Model::new(&start);
        let mut clock = now();
        for op in ops {
            run(&mut model, op, &mut clock);
            prop_assert!(model.text().is_char_boundary(model.cursor().get()));
            prop_assert!(model.text().is_char_boundary(model.selection().start.get()));
            prop_assert_eq!(model.rope.to_string(), model.text());
        }
    }

    #[test]
    fn the_parse_after_each_edit_equals_a_full_parse(start in text(), ops in prop::collection::vec(op(), 0..30)) {
        let mut model = Model::new(&start);
        let mut clock = now();
        for op in ops {
            run(&mut model, op, &mut clock);
            let full = Body::parse(model.text());
            prop_assert_eq!(model.body.blocks(), full.blocks());
            prop_assert_eq!(model.body.spans(), full.spans());
            prop_assert_eq!(model.body.hidden(), full.hidden());
            prop_assert_eq!(model.body.links(), full.links());
            prop_assert_eq!(model.body.headings(), full.headings());
        }
    }

    #[test]
    fn undo_then_redo_gives_the_same_text(start in text(), ops in prop::collection::vec(op(), 0..30)) {
        let mut model = Model::new(&start);
        let mut clock = now();
        for op in ops {
            run(&mut model, op, &mut clock);
        }
        let end = model.text().to_owned();
        let mut steps = 0;
        while model.undo() {
            steps += 1;
            prop_assert!(model.text().is_char_boundary(model.cursor().get()));
        }
        let start = lf_only(&start).into_owned();
        prop_assert_eq!(model.text(), start);
        for _ in 0..steps {
            prop_assert!(model.redo());
        }
        prop_assert_eq!(model.text(), end);
        prop_assert_eq!(model.rope.to_string(), model.text());
    }

    #[test]
    fn screen_lines_map_back_into_their_source(start in text(), at in 0..60usize, image in any::<bool>()) {
        let model = model_at(&start, at);
        let embed = if image { Embed::Image } else { Embed::Chip };
        for line in LineIndex::FIRST.through(model.last_line()) {
            let display = model.display_line(line, &mut |_| embed);
            let source = display.source();
            prop_assert_eq!(display.shown.runs.iter().map(|r| r.len).sum::<usize>(), display.shown.text.len());
            for offset in (0..=display.shown.text.len()).filter(|o| display.shown.text.is_char_boundary(*o)) {
                let mapped = display.to_source(ScreenOffset::new(offset));
                prop_assert!(source.start <= mapped && mapped <= source.end);
                prop_assert!(model.text().is_char_boundary(mapped.get()));
                prop_assert_eq!(display.to_display(mapped), ScreenOffset::new(offset));
            }
        }
    }
}
