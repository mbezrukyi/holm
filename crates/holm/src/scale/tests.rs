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
    while let Some(next) = scale.step(Size::Bigger) {
        scale = next;
        up.push(scale.percent());
    }
    assert_eq!(up, [110, 125, 150, 175, 200]);
    assert!(scale.at_end(Size::Bigger));
    assert_eq!(Scale::from_percent(90).step(Size::Smaller), Some(Scale(80)));
    assert_eq!(Scale::from_percent(80).step(Size::Smaller), None);
    assert!(Scale::from_percent(80).at_end(Size::Smaller));
    assert!((Scale::from_percent(125).factor() - 1.25).abs() < f32::EPSILON);
}
