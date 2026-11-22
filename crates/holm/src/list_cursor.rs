//! The step of a list cursor, for every list and the options of a settings row.

/// The item after `at`, or before it when not `forward`, among the `len` items. It wraps at
/// the ends, and skips each item where `works` is false. With no cursor, forward goes to the
/// first item that works, back to the last. It is `None` only when no item works.
pub(crate) fn step(at: Option<usize>, len: usize, forward: bool, works: impl Fn(usize) -> bool) -> Option<usize> {
    let start = match at.filter(|at| *at < len) {
        Some(at) => at,
        // One step from the item before the first, or after the last.
        None if forward => len.checked_sub(1)?,
        None => 0,
    };
    (1..=len)
        .map(|distance| {
            if forward {
                (start + distance) % len
            } else {
                (start + len - distance) % len
            }
        })
        .find(|&next| works(next))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn it_wraps_at_both_ends() {
        assert_eq!(step(Some(2), 3, true, |_| true), Some(0));
        assert_eq!(step(Some(0), 3, false, |_| true), Some(2));
        assert_eq!(step(Some(1), 3, true, |_| true), Some(2));
    }

    #[test]
    fn with_no_cursor_forward_goes_to_the_first_and_back_to_the_last() {
        assert_eq!(step(None, 3, true, |_| true), Some(0));
        assert_eq!(step(None, 3, false, |_| true), Some(2));
        assert_eq!(step(None, 3, true, |at| at != 0), Some(1));
        assert_eq!(step(None, 3, false, |at| at != 2), Some(1));
    }

    #[test]
    fn it_skips_the_items_that_do_not_work() {
        assert_eq!(step(Some(1), 4, true, |at| at != 2), Some(3));
        assert_eq!(step(Some(3), 4, true, |at| at != 0), Some(1));
        assert_eq!(step(Some(1), 3, true, |at| at == 1), Some(1));
    }

    #[test]
    fn it_is_none_only_when_no_item_works() {
        assert_eq!(step(None, 0, true, |_| true), None);
        assert_eq!(step(Some(0), 0, false, |_| true), None);
        assert_eq!(step(Some(1), 3, true, |_| false), None);
    }

    #[test]
    fn a_cursor_past_the_end_counts_as_no_cursor() {
        assert_eq!(step(Some(5), 3, true, |_| true), Some(0));
        assert_eq!(step(Some(5), 3, false, |_| true), Some(2));
    }

    proptest! {
        #[test]
        fn it_lands_on_the_nearest_item_that_works_in_its_way(
            works in prop::collection::vec(any::<bool>(), 0..12),
            at in prop::option::of(0..14usize),
            forward in any::<bool>(),
        ) {
            let len = works.len();
            let next = step(at, len, forward, |at| works[at]);
            prop_assert_eq!(next.is_some(), works.contains(&true));
            if let Some(next) = next {
                prop_assert!(works[next]);
                // Each item between the start and `next` does not work.
                let order: Vec<usize> = match at.filter(|at| *at < len) {
                    Some(at) if forward => (at + 1..len).chain(0..=at).collect(),
                    Some(at) => (0..at).rev().chain((at..len).rev()).collect(),
                    None if forward => (0..len).collect(),
                    None => (0..len).rev().collect(),
                };
                let first = order.into_iter().find(|&at| works[at]);
                prop_assert_eq!(first, Some(next));
            }
        }
    }
}
