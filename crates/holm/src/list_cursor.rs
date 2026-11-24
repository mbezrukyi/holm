//! The step of a list cursor, for every list and the options of a settings row.

/// Which item [`step`] goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Toward {
    Next,
    Previous,
}

/// The item after `at`, or before it toward [`Toward::Previous`], among the `len` items. It wraps at the ends, and
/// skips each item where `works` is false. With no cursor, next goes to the first item that works, previous to the
/// last. It is `None` only when no item works.
pub(crate) fn step(at: Option<usize>, len: usize, toward: Toward, works: impl Fn(usize) -> bool) -> Option<usize> {
    let start = match at.filter(|at| *at < len) {
        Some(at) => at,
        // One step from the item before the first, or after the last.
        None if toward == Toward::Next => len.checked_sub(1)?,
        None => 0,
    };
    (1..=len)
        .map(|distance| match toward {
            Toward::Next => (start + distance) % len,
            Toward::Previous => (start + len - distance) % len,
        })
        .find(|&next| works(next))
}

#[cfg(test)]
mod tests;
