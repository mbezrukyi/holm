/// The cursor offset, with what Up, Down, Home and End know about its screen row.
///
/// [`super::Model::set_cursor`] makes a plain caret. So any other cursor change drops the row start and the goals.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Caret {
    offset: usize,
    /// A wrap inside a word puts the end of a row and the start of the next row at the same offset. This picks the
    /// start.
    row_start: bool,
    /// The x that Up and Down keep, in pixels.
    goal_x: Option<f32>,
    goal_row: Option<GoalRow>,
}

/// After Up or Down into a line that showed no text: the row to put the cursor on at the goal x, once the next frame
/// shows the source of that line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GoalRow {
    /// The first row after Down, the last row after Up.
    pub(crate) first: bool,
    pub(crate) extend: bool,
}

impl Caret {
    pub(crate) fn at(offset: usize) -> Self {
        Self {
            offset,
            ..Self::default()
        }
    }

    pub(crate) fn with_row_start(self, row_start: bool) -> Self {
        Self { row_start, ..self }
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

    /// Whether the cursor shows at the start of its row, not at the end of the row above.
    pub(crate) fn row_start(&self) -> bool {
        self.row_start
    }

    pub(crate) fn goal_x(&self) -> Option<f32> {
        self.goal_x
    }

    pub(crate) fn goal_row(&self) -> Option<GoalRow> {
        self.goal_row
    }
}
