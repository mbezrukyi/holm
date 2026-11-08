//! The undo stack.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Typing after a pause this long starts a new undo step.
const PAUSE: Duration = Duration::from_secs(1);

/// The oldest steps go when the stack grows past this.
const MAX_STEPS: usize = 1000;

/// One replaced range. `start` is a byte offset in the text right before this change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Change {
    pub(super) start: usize,
    pub(super) removed: String,
    pub(super) inserted: String,
}

/// The anchor and the cursor.
pub(super) type Ends = (usize, usize);

/// How a step can join the step before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Group {
    /// The step never joins another one.
    Alone,
    /// Typed text joins earlier typing, up to the end of a word.
    Typing,
    /// Removed text joins earlier removals.
    Deleting,
}

/// One undo step: the changes in the order they were made.
#[derive(Debug)]
pub(super) struct Step {
    pub(super) changes: Vec<Change>,
    pub(super) before: Ends,
    pub(super) after: Ends,
    pub(super) group: Group,
    /// When the step last grew. `None` after an undo, so a redone step never joins new typing.
    pub(super) time: Option<Instant>,
}

#[derive(Debug, Default)]
pub(super) struct History {
    undo: VecDeque<Step>,
    redo: Vec<Step>,
}

impl History {
    /// Adds a new step, or joins it to the last one. It clears the redo steps.
    pub(super) fn record(&mut self, step: Step) {
        self.redo.clear();
        if let Some(last) = self.undo.back_mut()
            && last.join(&step)
        {
            return;
        }
        self.undo.push_back(step);
        if self.undo.len() > MAX_STEPS {
            self.undo.pop_front();
        }
    }

    pub(super) fn pop_undo(&mut self) -> Option<Step> {
        self.undo.pop_back()
    }

    pub(super) fn push_redo(&mut self, mut step: Step) {
        step.time = None;
        self.redo.push(step);
    }

    pub(super) fn pop_redo(&mut self) -> Option<Step> {
        self.redo.pop()
    }

    /// Puts back a redone step, and keeps the other redo steps.
    pub(super) fn push_undo(&mut self, step: Step) {
        self.undo.push_back(step);
    }
}

impl Step {
    /// Adds `next` to this step when both are one run of typing or of removal.
    fn join(&mut self, next: &Step) -> bool {
        let (Some(time), Some(next_time)) = (self.time, next.time) else {
            return false;
        };
        if self.group != next.group
            || self.group == Group::Alone
            || next_time.saturating_duration_since(time) >= PAUSE
            || self.after != next.before
        {
            return false;
        }
        let ([last], [new]) = (&mut self.changes[..], &next.changes[..]) else {
            return false;
        };
        let joined = match self.group {
            Group::Alone => false,
            Group::Typing => {
                let new_word = last.inserted.ends_with(char::is_whitespace)
                    && !new.inserted.starts_with(char::is_whitespace);
                let joins = new.removed.is_empty()
                    && new.start == last.start + last.inserted.len()
                    && !new_word;
                if joins {
                    last.inserted.push_str(&new.inserted);
                }
                joins
            }
            Group::Deleting => {
                if !last.inserted.is_empty() || !new.inserted.is_empty() {
                    false
                } else if new.start + new.removed.len() == last.start {
                    last.start = new.start;
                    last.removed.insert_str(0, &new.removed);
                    true
                } else if new.start == last.start {
                    last.removed.push_str(&new.removed);
                    true
                } else {
                    false
                }
            }
        };
        if joined {
            self.after = next.after;
            self.time = next.time;
        }
        joined
    }
}
