use super::*;

#[test]
fn the_first_digit_replaces_the_width_and_the_next_ones_add_to_it() {
    let mut draft = CharsDraft::new(80);
    draft.type_digit('1');
    assert_eq!(draft.chars(), Some(1));
    for digit in ['2', '0', '5'] {
        draft.type_digit(digit);
    }
    assert_eq!(draft.text, "120", "3 digits at most");
    draft.backspace();
    draft.backspace();
    draft.backspace();
    assert_eq!(draft.chars(), None);
}

#[test]
fn backspace_on_a_fresh_width_edits_it() {
    let mut draft = CharsDraft::new(80);
    draft.backspace();
    draft.type_digit('5');
    assert_eq!(draft.text, "85");
}
