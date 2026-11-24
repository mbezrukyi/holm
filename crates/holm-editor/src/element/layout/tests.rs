use super::*;

#[test]
fn file_sizes_have_a_unit_and_at_most_one_decimal() {
    assert_eq!(file_size(512), "512\u{a0}B");
    assert_eq!(file_size(48_000), "48\u{a0}KB");
    assert_eq!(file_size(2_140_000), "2.1\u{a0}MB");
    assert_eq!(file_size(999_999), "1.0\u{a0}MB");
    assert_eq!(file_size(9_960_000), "10\u{a0}MB");
}

#[test]
fn table_columns_get_their_widest_widths_when_they_fit() {
    let columns = [(px(20.), px(50.)), (px(30.), px(40.))];
    assert_eq!(split(&columns, px(100.), px(8.)), [px(50.), px(40.)]);
    assert_eq!(split(&columns, px(90.), px(8.)), [px(50.), px(40.)]);
}

#[test]
fn table_columns_share_the_room_past_their_narrowest_widths() {
    // 30 of room left, for differences of 30 and 10 and 0.
    let columns = [(px(20.), px(50.)), (px(30.), px(40.)), (px(10.), px(10.))];
    let widths = split(&columns, px(90.), px(8.));
    let expected = [px(20. + 22.5), px(30. + 7.5), px(10.)];
    for (width, expected) in widths.iter().zip(expected) {
        assert!((*width - expected).abs() < px(0.001), "{widths:?}");
    }
    assert_eq!(split(&columns, px(60.), px(8.)), [px(20.), px(30.), px(10.)]);
}

#[test]
fn table_columns_shrink_alike_when_not_even_their_narrowest_widths_fit() {
    let columns = [(px(40.), px(50.)), (px(20.), px(40.)), (px(4.), px(4.))];
    assert_eq!(split(&columns, px(32.), px(3.)), [px(20.), px(10.), px(3.)]);
}
