use super::*;

#[test]
fn a_width_out_of_the_range_changes_nothing() {
    let mut width = TextWidthSetting::default();
    assert_eq!(width.chars(), 80);
    for bad in [39, 201, 0, -40, i64::from(u16::MAX) + 40] {
        assert!(!width.set_chars(bad), "{bad}");
    }
    assert_eq!(width.chars(), 80);
    assert!(width.set_chars(40) && width.set_chars(200));
    assert_eq!(width.chars(), 200);
}

#[test]
fn full_keeps_the_width_for_the_editor_and_the_next_limited() {
    let mut width = TextWidthSetting::default();
    width.set_chars(120);
    assert_eq!(width.text_width(), TextWidth::Full);
    width.limited = true;
    assert_eq!(width.text_width(), TextWidth::Limited(120));
}
