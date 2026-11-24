use super::*;

fn color(value: &str) -> Color {
    Color::from_hex(value).unwrap_or_else(|| panic!("{value} is a color"))
}

#[test]
fn a_used_color_goes_first_and_the_list_keeps_three() {
    let mut custom = vec![color("#111"), color("#222"), color("#333")];
    assert!(remember(&mut custom, color("#444")));
    assert_eq!(custom, [color("#444"), color("#111"), color("#222")]);
    assert!(remember(&mut custom, color("#222")));
    assert_eq!(custom, [color("#222"), color("#444"), color("#111")]);
    assert!(!remember(&mut custom, color("#222")));
}

#[test]
fn the_hex_field_takes_only_six_hex_digits() {
    let mut window = ColorWindow::new(Paint::Text);
    window.push("ab");
    assert_eq!(window.hex, None);
    window.hex = Some(String::new());
    window.push("#12x3456789");
    assert_eq!(window.hex.as_deref(), Some("123456"));
}
