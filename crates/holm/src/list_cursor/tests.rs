use proptest::prelude::*;

use super::*;

#[test]
fn it_wraps_at_both_ends() {
    assert_eq!(step(Some(2), 3, Toward::Next, |_| true), Some(0));
    assert_eq!(step(Some(0), 3, Toward::Previous, |_| true), Some(2));
    assert_eq!(step(Some(1), 3, Toward::Next, |_| true), Some(2));
}

#[test]
fn with_no_cursor_next_goes_to_the_first_and_previous_to_the_last() {
    assert_eq!(step(None, 3, Toward::Next, |_| true), Some(0));
    assert_eq!(step(None, 3, Toward::Previous, |_| true), Some(2));
    assert_eq!(step(None, 3, Toward::Next, |at| at != 0), Some(1));
    assert_eq!(step(None, 3, Toward::Previous, |at| at != 2), Some(1));
}

#[test]
fn it_skips_the_items_that_do_not_work() {
    assert_eq!(step(Some(1), 4, Toward::Next, |at| at != 2), Some(3));
    assert_eq!(step(Some(3), 4, Toward::Next, |at| at != 0), Some(1));
    assert_eq!(step(Some(1), 3, Toward::Next, |at| at == 1), Some(1));
}

#[test]
fn it_is_none_only_when_no_item_works() {
    assert_eq!(step(None, 0, Toward::Next, |_| true), None);
    assert_eq!(step(Some(0), 0, Toward::Previous, |_| true), None);
    assert_eq!(step(Some(1), 3, Toward::Next, |_| false), None);
}

#[test]
fn a_cursor_past_the_end_counts_as_no_cursor() {
    assert_eq!(step(Some(5), 3, Toward::Next, |_| true), Some(0));
    assert_eq!(step(Some(5), 3, Toward::Previous, |_| true), Some(2));
}

proptest! {
    #[test]
    fn it_lands_on_the_nearest_item_that_works_in_its_way(
        works in prop::collection::vec(any::<bool>(), 0..12),
        at in prop::option::of(0..14usize),
        toward in prop_oneof![Just(Toward::Next), Just(Toward::Previous)],
    ) {
        let len = works.len();
        let next = step(at, len, toward, |at| works[at]);
        prop_assert_eq!(next.is_some(), works.contains(&true));
        if let Some(next) = next {
            prop_assert!(works[next]);
            let walk_order: Vec<usize> = match at.filter(|at| *at < len) {
                Some(at) if toward == Toward::Next => (at + 1..len).chain(0..=at).collect(),
                Some(at) => (0..at).rev().chain((at..len).rev()).collect(),
                None if toward == Toward::Next => (0..len).collect(),
                None => (0..len).rev().collect(),
            };
            let nearest_working = walk_order.into_iter().find(|&at| works[at]);
            prop_assert_eq!(nearest_working, Some(next));
        }
    }
}
