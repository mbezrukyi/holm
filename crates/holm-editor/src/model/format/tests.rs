use std::time::Instant;

use super::*;
use crate::model::{Selection, Step};

fn model_with(text: &str, selection: Range<usize>) -> Model {
    let mut model = Model::new(text);
    model.select(SourceOffset::range(selection));
    model
}

fn toggled(text: &str, selection: Range<usize>, format: Format) -> (String, Range<usize>) {
    let mut model = model_with(text, selection);
    model.toggle_format(format);
    (model.text().to_owned(), model.selection().bytes())
}

#[test]
fn a_format_key_wraps_the_selection_as_one_undo_step() {
    assert_eq!(toggled("a word", 2..6, Format::Bold), ("a **word**".into(), 4..8));
    let mut model = Model::new("a word");
    model.set_cursor(SourceOffset::new(6), Selection::Collapse);
    model.set_cursor(SourceOffset::new(2), Selection::Extend);
    model.toggle_format(Format::Underline);
    assert_eq!(model.text(), "a <u>word</u>");
    assert_eq!(
        (model.cursor(), model.selection()),
        (SourceOffset::new(5), SourceOffset::range(5..9))
    );
    model.undo();
    assert_eq!(model.text(), "a word");
}

#[test]
fn format_keys_keep_nested_styles_valid() {
    assert_eq!(
        toggled("**==Writ==**==e the report==", 4..8, Format::Bold).0,
        "==Write the report=="
    );
    assert_eq!(
        toggled("A **tip with strong** text", 0..5, Format::Italic).0,
        "*A* ***t*ip with strong** text"
    );
}

#[test]
fn a_format_key_with_no_selection_takes_the_word_or_adds_a_pair() {
    let mut model = model_with("one two", 1..1);
    model.toggle_format(Format::Bold);
    assert_eq!((model.text(), model.cursor()), ("**one** two", SourceOffset::new(3)));
    let mut model = model_with("one ", 4..4);
    model.toggle_format(Format::Highlight);
    assert_eq!((model.text(), model.cursor()), ("one ====", SourceOffset::new(6)));
}

#[test]
fn a_heading_key_sets_and_removes_the_level() {
    let mut model = model_with("Plans", 2..2);
    model.toggle_heading(2);
    assert_eq!(model.text(), "## Plans");
    assert_eq!(model.cursor(), SourceOffset::new(5));
    model.toggle_heading(1);
    assert_eq!(model.text(), "# Plans");
    model.toggle_heading(1);
    assert_eq!(model.text(), "Plans");
    assert_eq!(model.cursor(), SourceOffset::new(2));
    let mut model = model_with("a\n\n# b", 0..6);
    model.toggle_heading(3);
    assert_eq!(model.text(), "### a\n\n### b");
}

fn enter(text: &str, at: usize) -> (String, usize) {
    let mut model = model_with(text, at..at);
    model.newline(Instant::now());
    (model.text().to_owned(), model.cursor().get())
}

#[test]
fn enter_continues_a_list() {
    assert_eq!(enter("- one", 5), ("- one\n- ".into(), 8));
    assert_eq!(enter("9. one", 6), ("9. one\n10. ".into(), 11));
    assert_eq!(enter("- [x] done", 10), ("- [x] done\n- [ ] ".into(), 17));
    assert_eq!(enter("> * quote", 9), ("> * quote\n> * ".into(), 14));
    assert_eq!(enter("- a\n  - b", 9), ("- a\n  - b\n  - ".into(), 14));
    assert_eq!(enter("- onetwo", 5), ("- one\n- two".into(), 8));
}

#[test]
fn enter_on_an_empty_item_ends_the_list() {
    assert_eq!(enter("- one\n- ", 8), ("- one\n\n".into(), 7));
    assert_eq!(enter("> - a\n> - ", 10), ("> - a\n>\n> ".into(), 10));
    assert_eq!(enter("- a\n  - b\n  - ", 14), ("- a\n  - b\n\n".into(), 11));
}

#[test]
fn enter_outside_a_list_is_a_line_break() {
    assert_eq!(enter("text", 4), ("text\n".into(), 5));
    assert_eq!(enter("```\n- a\n```", 7), ("```\n- a\n\n```".into(), 8));
    assert_eq!(enter("- one", 1), ("-\n one".into(), 2));
}

fn indented(text: &str, selection: Range<usize>, key: Move) -> Option<String> {
    let mut model = model_with(text, selection);
    model.relist(key)?;
    Some(model.text().to_owned())
}

#[test]
fn tab_makes_an_item_a_child_of_the_item_above() {
    assert_eq!(indented("- a\n- b", 6..6, Move::In), Some("- a\n  - b".into()));
    assert_eq!(indented("10. a\n11. b", 8..8, Move::In), Some("10. a\n    1. b".into()));
    assert_eq!(
        indented("- a\n  - b\n- c", 12..12, Move::In),
        Some("- a\n  - b\n  - c".into())
    );
}

#[test]
fn shift_tab_moves_an_item_to_its_parent_level() {
    assert_eq!(indented("- a\n  - b", 9..9, Move::Out), Some("- a\n- b".into()));
    assert_eq!(indented("- a", 1..1, Move::Out), Some("- a".into()));
}

#[test]
fn tab_moves_each_selected_item() {
    assert_eq!(
        indented("- a\n- b\n- c", 4..11, Move::In),
        Some("- a\n  - b\n  - c".into())
    );
}

#[test]
fn tab_and_shift_tab_count_each_level() {
    let mut model = model_with("1. Milk\n2. Eggs\n3. Bread", 13..13);
    model.relist(Move::In);
    assert_eq!(model.text(), "1. Milk\n   1. Eggs\n2. Bread");
    assert_eq!(model.cursor(), SourceOffset::new(16));
    model.relist(Move::Out);
    assert_eq!(model.text(), "1. Milk\n2. Eggs\n3. Bread");
    assert_eq!(model.cursor(), SourceOffset::new(13));
}

#[test]
fn enter_counts_the_list_again() {
    assert_eq!(enter("1. a\n2. b\n3. c", 9), ("1. a\n2. b\n3. \n4. c".into(), 13));
}

#[test]
fn a_list_key_is_one_undo_step() {
    let text = "1. a\n2. b\n3. c\n4. d";
    let mut model = model_with(text, 9..9);
    model.relist(Move::In);
    assert_eq!(model.text(), "1. a\n   1. b\n2. c\n3. d");
    model.undo();
    assert_eq!((model.text(), model.cursor()), (text, SourceOffset::new(9)));
    model.newline(Instant::now());
    assert_eq!(model.text(), "1. a\n2. b\n3. \n4. c\n5. d");
    model.undo();
    assert_eq!((model.text(), model.cursor()), (text, SourceOffset::new(9)));
}

#[test]
fn the_cursor_stays_on_its_item_when_the_number_grows() {
    let mut model = model_with("9. a\n   1. bc", 12..12);
    model.relist(Move::Out);
    assert_eq!((model.text(), model.cursor()), ("9. a\n10. bc", SourceOffset::new(10)));
}

#[test]
fn typing_deleting_and_pasting_do_not_count_a_list_again() {
    let mut model = model_with("1. a\n2. b\n3. c", 4..9);
    model.backspace(Step::Char, Instant::now());
    assert_eq!(model.text(), "1. a\n3. c");
    model.insert("\n7. x");
    assert_eq!(model.text(), "1. a\n7. x\n3. c");
    model.type_text("\n2. y", Instant::now());
    assert_eq!(model.text(), "1. a\n7. x\n2. y\n3. c");
}

#[test]
fn tab_outside_a_list_is_not_an_indent() {
    assert_eq!(indented("text", 0..0, Move::In), None);
}

const STYLES: [Format; 5] = [
    Format::Bold,
    Format::Italic,
    Format::Underline,
    Format::Strikethrough,
    Format::Highlight,
];

proptest::proptest! {
    #[test]
    fn keys_pressed_twice_in_any_order_give_back_the_text(
        words in proptest::collection::vec("[a-z]{1,5}", 1..5),
        keys in proptest::collection::vec(0..STYLES.len(), 1..6),
    ) {
        let text = words.join(" ");
        let mut model = model_with(&text, 0..text.len());
        for key in keys.iter().chain(&keys) {
            model.toggle_format(STYLES[*key]);
        }
        proptest::prop_assert_eq!(model.text(), text.as_str());
    }
}
