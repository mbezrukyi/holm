//! The scale of the app: how much bigger or smaller Holm draws the whole window than the desktop scale. The steps live
//! here, and nothing else knows them.

/// Which way a step of the scale goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Size {
    Bigger,
    Smaller,
}

/// One of the steps of the scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Scale(u16);

/// The smallest first, with their labels in the settings.
const STEPS: [(Scale, &str); 8] = [
    (Scale(80), "80"),
    (Scale(90), "90"),
    (Scale(100), "100"),
    (Scale(110), "110"),
    (Scale(125), "125"),
    (Scale(150), "150"),
    (Scale(175), "175"),
    (Scale(200), "200"),
];

impl Default for Scale {
    fn default() -> Self {
        Self(100)
    }
}

impl Scale {
    /// A value that is not a step gives the default.
    pub(crate) fn from_percent(percent: i64) -> Self {
        STEPS
            .iter()
            .map(|(step, _)| *step)
            .find(|step| i64::from(step.0) == percent)
            .unwrap_or_default()
    }

    pub(crate) fn percent(self) -> u16 {
        self.0
    }

    /// The factor on the desktop scale, like 1.25.
    pub(crate) fn factor(self) -> f32 {
        f32::from(self.0) / 100.
    }

    /// The smallest first.
    pub(crate) fn options() -> &'static [(Self, &'static str)] {
        &STEPS
    }

    /// The next step that way, or `None` at that end.
    pub(crate) fn step(self, size: Size) -> Option<Self> {
        let at = STEPS.iter().position(|(step, _)| *step == self)?;
        let next = match size {
            Size::Bigger => at.checked_add(1)?,
            Size::Smaller => at.checked_sub(1)?,
        };
        STEPS.get(next).map(|(step, _)| *step)
    }

    pub(crate) fn at_end(self, size: Size) -> bool {
        self.step(size).is_none()
    }
}

#[cfg(test)]
mod tests;
