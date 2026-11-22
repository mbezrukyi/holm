/// The cursor offset, with what Up, Down, Home and End know about its screen row.
///
/// [`super::Model::set_cursor`] makes a plain caret. So any other cursor change drops the affinity and the goals.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Caret {
    offset: usize,
    affinity: Affinity,
    /// The x that Up and Down keep, in pixels.
    goal_x: Option<f32>,
    goal_row: Option<GoalRow>,
}

/// A wrap inside a word puts the end of a row and the start of the next row at the same offset. This picks one of them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Affinity {
    /// The end of the row above.
    #[default]
    RowEnd,
    /// The start of the next row.
    RowStart,
}

/// After Up or Down into a line that showed no text: the row to put the cursor on at the goal x, once the next frame
/// shows the source of that line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GoalRow {
    pub(crate) direction: Direction,
    pub(crate) selection: Selection,
}

/// Of Up or Down. The cursor enters a line at its first row after Down, and at its last row after Up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Direction {
    Up,
    Down,
}

/// What a cursor move does to the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Selection {
    /// The anchor moves with the cursor, so nothing stays selected.
    Collapse,
    /// The anchor stays, so the selection grows or shrinks.
    Extend,
}

impl Caret {
    pub(crate) fn at(offset: usize) -> Self {
        Self {
            offset,
            ..Self::default()
        }
    }

    pub(crate) fn with_affinity(self, affinity: Affinity) -> Self {
        Self { affinity, ..self }
    }

    pub(crate) fn with_goal_x(self, goal_x: f32) -> Self {
        Self {
            goal_x: Some(goal_x),
            ..self
        }
    }

    pub(crate) fn with_goal_row(self, goal_row: Option<GoalRow>) -> Self {
        Self { goal_row, ..self }
    }

    pub(crate) fn offset(&self) -> usize {
        self.offset
    }

    pub(crate) fn affinity(&self) -> Affinity {
        self.affinity
    }

    pub(crate) fn goal_x(&self) -> Option<f32> {
        self.goal_x
    }

    pub(crate) fn goal_row(&self) -> Option<GoalRow> {
        self.goal_row
    }
}
