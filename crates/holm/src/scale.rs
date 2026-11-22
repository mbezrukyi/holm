//! The scale of the app: how much bigger or smaller Holm draws the whole window than the desktop scale. The steps live
//! here, and nothing else knows them.

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

    /// The next step up or down, or `None` at that end.
    pub(crate) fn step(self, bigger: bool) -> Option<Self> {
        let at = STEPS.iter().position(|(step, _)| *step == self)?;
        let next = if bigger { at.checked_add(1)? } else { at.checked_sub(1)? };
        STEPS.get(next).map(|(step, _)| *step)
    }

    pub(crate) fn at_end(self, bigger: bool) -> bool {
        self.step(bigger).is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_that_is_not_a_step_gives_100() {
        assert_eq!(Scale::from_percent(125).percent(), 125);
        for bad in [0, 99, 101, 300, -100] {
            assert_eq!(Scale::from_percent(bad), Scale::default(), "{bad}");
        }
        assert_eq!(Scale::default().percent(), 100);
    }

    #[test]
    fn the_steps_go_up_and_down_and_stop_at_the_ends() {
        let mut scale = Scale::default();
        let mut up = Vec::new();
        while let Some(next) = scale.step(true) {
            scale = next;
            up.push(scale.percent());
        }
        assert_eq!(up, [110, 125, 150, 175, 200]);
        assert!(scale.at_end(true));
        assert_eq!(Scale::from_percent(90).step(false), Some(Scale(80)));
        assert_eq!(Scale::from_percent(80).step(false), None);
        assert!(Scale::from_percent(80).at_end(false));
        assert!((Scale::from_percent(125).factor() - 1.25).abs() < f32::EPSILON);
    }
}
