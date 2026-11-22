use std::ops::Range;

use holm_markdown::list::{Move, relist};
use holm_markdown::{Body, OffsetRange, SourceOffset};

/// The text and the selection after `key`. The selection is given and returned with `|` for the cursor, or two `|`
/// around the selected text.
fn key(marked: &str, key: Move) -> String {
    let (text, selection) = unmark(marked);
    let Some(edit) = relist(&text, &Body::parse(&text), SourceOffset::range(selection), key) else {
        return NO_EDIT.to_owned();
    };
    let mut new = text;
    new.replace_range(edit.range.bytes(), &edit.text);
    mark(&new, &edit.selection.bytes())
}

const NO_EDIT: &str = "no edit";

fn unmark(marked: &str) -> (String, Range<usize>) {
    let ends: Vec<usize> = marked.match_indices('|').map(|(at, _)| at).collect();
    let text = marked.replace('|', "");
    let selection = match ends[..] {
        [at] => at..at,
        [start, end] => start..end - 1,
        _ => panic!("one or two | in {marked:?}"),
    };
    (text, selection)
}

fn mark(text: &str, selection: &Range<usize>) -> String {
    let mut out = text.to_owned();
    out.insert(selection.end, '|');
    if !selection.is_empty() {
        out.insert(selection.start, '|');
    }
    out
}

fn tab(marked: &str) -> String {
    key(marked, Move::In)
}

fn shift_tab(marked: &str) -> String {
    key(marked, Move::Out)
}

fn enter(marked: &str) -> String {
    key(marked, Move::Next)
}

#[test]
fn tab_on_the_last_item_starts_a_list_at_1() {
    assert_eq!(tab("1. a\n2. b\n3. c|"), "1. a\n2. b\n   1. c|");
}

#[test]
fn the_cursor_stays_on_its_item_below_other_text() {
    assert_eq!(
        tab("# Lists\n\n1. Milk\n2. Eggs\n3. |"),
        "# Lists\n\n1. Milk\n2. Eggs\n   1. |"
    );
    assert_eq!(
        shift_tab("# Lists\n\n1. Milk\n   1. Eg|gs"),
        "# Lists\n\n1. Milk\n2. Eg|gs"
    );
}

#[test]
fn tab_in_the_middle_counts_the_rest_again() {
    assert_eq!(tab("1. Milk\n2. Eg|gs\n3. Bread"), "1. Milk\n   1. Eg|gs\n2. Bread");
}

#[test]
fn tab_joins_the_sub_list_of_the_item_above() {
    assert_eq!(tab("1. a\n   1. b\n2. |c\n3. d"), "1. a\n   1. b\n   2. |c\n2. d");
}

#[test]
fn a_moved_item_takes_the_delimiter_of_the_list_it_joins() {
    assert_eq!(tab("1. a\n   1) b\n2. |c"), "1. a\n   1) b\n   2) |c");
    assert_eq!(tab("- a\n  * b\n- |c"), "- a\n  * b\n  * |c");
}

#[test]
fn shift_tab_takes_the_next_number_of_the_list_above() {
    assert_eq!(shift_tab("1. a\n   1. b\n   2. |c\n2. d"), "1. a\n   1. b\n2. |c\n3. d");
}

#[test]
fn shift_tab_makes_the_items_below_children_of_the_moved_item() {
    assert_eq!(
        shift_tab("1. A\n   1. B\n   2. |C\n   3. D"),
        "1. A\n   1. B\n2. |C\n   1. D"
    );
}

#[test]
fn enter_counts_the_rest_of_the_list_again() {
    assert_eq!(enter("1. a|\n2. b"), "1. a\n2. |\n3. b");
    assert_eq!(enter("1. one|two\n2. b"), "1. one\n2. |two\n3. b");
}

#[test]
fn a_list_keeps_its_first_number() {
    assert_eq!(enter("5. a|\n6. b"), "5. a\n6. |\n7. b");
    assert_eq!(tab("5. a\n6. |b\n7. c"), "5. a\n   1. |b\n6. c");
}

#[test]
fn a_paren_delimiter_stays() {
    assert_eq!(tab("1) a\n2) |b"), "1) a\n   1) |b");
    assert_eq!(enter("1) a|\n2) b"), "1) a\n2) |\n3) b");
}

#[test]
fn numbered_items_under_bullets() {
    assert_eq!(tab("- a\n  1. b\n  2. |c"), "- a\n  1. b\n     1. |c");
    assert_eq!(tab("- a\n  1. b\n- |c"), "- a\n  1. b\n  - |c");
    assert_eq!(shift_tab("- a\n  1. |b\n  2. c"), "- a\n1. |b\n   1. c");
}

#[test]
fn items_in_a_quote() {
    assert_eq!(tab("> 1. a\n> 2. |b"), "> 1. a\n>    1. |b");
    assert_eq!(shift_tab("> 1. a\n>    1. |b\n> 2. c"), "> 1. a\n> 2. |b\n> 3. c");
    assert_eq!(enter("> 1. a|\n> 2. b"), "> 1. a\n> 2. |\n> 3. b");
}

#[test]
fn the_cursor_stays_on_its_item_when_the_number_grows() {
    assert_eq!(shift_tab("9. a\n   1. b|"), "9. a\n10. b|");
    assert_eq!(enter("9. a|\n10. b"), "9. a\n10. |\n11. b");
    assert_eq!(tab("8. a\n9. b\n10. c|"), "8. a\n9. b\n   1. c|");
}

#[test]
fn an_item_moves_with_its_children_and_later_paragraphs() {
    assert_eq!(tab("- a\n- |b\n  - c\n\n  more"), "- a\n  - |b\n    - c\n\n    more");
    assert_eq!(
        shift_tab("1. a\n   1. |b\n      - c\n\n      more"),
        "1. a\n2. |b\n   - c\n\n   more"
    );
}

#[test]
fn a_selection_moves_each_top_item_once() {
    assert_eq!(tab("- a\n- |b\n  - c|"), "- a\n  - |b\n    - c|");
    assert_eq!(tab("1. a\n2. |b\n3. c|"), "1. a\n   1. |b\n   2. c|");
    assert_eq!(shift_tab("1. a\n   1. |b\n   2. c|"), "1. a\n2. |b\n3. c|");
}

#[test]
fn keys_that_do_nothing_still_take_the_key() {
    assert_eq!(tab("1. |a\n2. b"), "1. |a\n2. b");
    assert_eq!(shift_tab("- a|"), "- a|");
}

#[test]
fn outside_a_list_there_is_no_edit() {
    assert_eq!(tab("text|"), NO_EDIT);
    assert_eq!(shift_tab("text|"), NO_EDIT);
    assert_eq!(enter("text|"), NO_EDIT);
    assert_eq!(tab("- a\n\n      code|"), NO_EDIT);
    assert_eq!(tab("- a\n  ```\n  code|\n  ```"), NO_EDIT);
}

#[test]
fn enter_on_an_empty_item_ends_the_list() {
    assert_eq!(enter("- one\n- |"), "- one\n\n|");
    assert_eq!(enter("> - a\n> - |"), "> - a\n>\n> |");
    assert_eq!(enter("1. a\n2. |\n3. b"), "1. a\n\n|\n3. b");
}

#[test]
fn enter_before_the_item_text_is_no_list_key() {
    assert_eq!(enter("-| one"), NO_EDIT);
    assert_eq!(enter("- [ ]| a"), NO_EDIT);
}

#[test]
fn enter_keeps_a_task_box() {
    assert_eq!(enter("- [x] done|"), "- [x] done\n- [ ] |");
}

#[test]
fn an_empty_numbered_item_starts_a_sub_list() {
    assert_eq!(tab("1. a\n2. |"), "1. a\n   1. |");
}
