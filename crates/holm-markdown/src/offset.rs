//! Byte offsets into the three texts that a parse and an edit work on. Each kind has its own type, so that an offset
//! into one text cannot be read as an offset into another.

use std::fmt;
use std::ops::{Add, AddAssign, Range, Sub, SubAssign};

macro_rules! offset {
    ($(#[$doc:meta])* $vis:vis struct $name:ident;) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        $vis struct $name(usize);

        impl $name {
            $vis const fn new(bytes: usize) -> Self {
                Self(bytes)
            }

            $vis const fn get(self) -> usize {
                self.0
            }

            $vis fn range(bytes: Range<usize>) -> Range<Self> {
                Self(bytes.start)..Self(bytes.end)
            }
        }

        /// Writes the bare number, like a `usize`.
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Debug::fmt(&self.0, f)
            }
        }

        impl Add<usize> for $name {
            type Output = Self;

            fn add(self, bytes: usize) -> Self {
                Self(self.0 + bytes)
            }
        }

        impl AddAssign<usize> for $name {
            fn add_assign(&mut self, bytes: usize) {
                self.0 += bytes;
            }
        }

        impl Sub<usize> for $name {
            type Output = Self;

            fn sub(self, bytes: usize) -> Self {
                Self(self.0 - bytes)
            }
        }

        impl SubAssign<usize> for $name {
            fn sub_assign(&mut self, bytes: usize) {
                self.0 -= bytes;
            }
        }

        /// The bytes from `other` to `self`.
        impl Sub for $name {
            type Output = usize;

            fn sub(self, other: Self) -> usize {
                self.0 - other.0
            }
        }

        impl OffsetRange for Range<$name> {
            fn bytes(&self) -> Range<usize> {
                self.start.0..self.end.0
            }
        }
    };
}

/// A range of one kind of offset.
pub trait OffsetRange {
    /// The range as plain byte offsets, to slice the text it is in.
    fn bytes(&self) -> Range<usize>;
}

offset! {
    /// A byte offset into the parsed text, the body of a note.
    #[derive(Default)]
    pub struct SourceOffset;
}

offset! {
    /// A byte offset into the lines of a leaf block joined with `\n`, without the quote markers and indents between
    /// them.
    #[derive(Default, Hash)]
    pub(crate) struct JoinedOffset;
}

offset! {
    /// A byte offset into the text after an [`crate::inline::Edit`].
    #[derive(Default)]
    pub struct NewOffset;
}

impl NewOffset {
    /// The source before an edit is the same in the new text.
    pub(crate) fn unmoved(offset: SourceOffset) -> Self {
        Self(offset.0)
    }
}
