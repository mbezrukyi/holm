use super::*;

/// 10 px for each char, in both fonts.
fn measure(text: &str, _kind: Kind) -> Pixels {
    px(10.) * text.chars().count()
}

fn rows(text: &PathText, mode: Mode, width: f32) -> Vec<Vec<Piece>> {
    lay_out(&text.parts, mode, Some(px(width)), &measure)
        .into_iter()
        .map(|row| row.pieces)
        .collect()
}

fn piece(x: f32, text: &str, kind: Kind) -> Piece {
    Piece {
        x: px(x),
        text: text.to_owned(),
        kind,
    }
}

fn missing() -> PathText {
    PathText::default()
        .plain("Not at ")
        .path("~/Gone".to_owned())
        .plain(". Pick one.")
}

#[test]
fn the_plain_text_holds_the_paths() {
    assert_eq!(missing().plain_text(), "Not at ~/Gone. Pick one.");
    assert_eq!(PathText::from("a").plain("b"), PathText::from("ab"));
}

#[test]
fn a_text_that_fits_stays_in_one_row() {
    assert_eq!(
        rows(&missing(), Mode::Wrap, 1000.),
        vec![vec![
            piece(0., "Not at ", Kind::Plain),
            piece(70., "~/Gone", Kind::Path),
            piece(140., ". Pick one.", Kind::Plain),
        ]]
    );
}

#[test]
fn the_mark_after_a_path_wraps_with_the_chip() {
    // "Not at " and the chip fit, but the "." does not.
    assert_eq!(
        rows(&missing(), Mode::Wrap, 145.),
        vec![
            vec![piece(0., "Not at ", Kind::Plain)],
            vec![piece(0., "~/Gone", Kind::Path), piece(70., ". Pick ", Kind::Plain)],
            vec![piece(0., "one.", Kind::Plain)],
        ]
    );
}

#[test]
fn a_path_wider_than_the_row_ends_in_an_ellipsis() {
    assert_eq!(
        rows(&missing(), Mode::Wrap, 60.),
        vec![
            vec![piece(0., "Not at ", Kind::Plain)],
            vec![piece(0., "~/Go…", Kind::Path), piece(60., ". ", Kind::Plain)],
            vec![piece(0., "Pick ", Kind::Plain)],
            vec![piece(0., "one.", Kind::Plain)],
        ]
    );
}

#[test]
fn one_line_cuts_the_plain_text() {
    assert_eq!(
        rows(&missing(), Mode::OneLine, 200.),
        vec![vec![
            piece(0., "Not at ", Kind::Plain),
            piece(70., "~/Gone", Kind::Path),
            piece(140., ". Pic…", Kind::Plain),
        ]]
    );
}

#[test]
fn a_cut_inside_the_path_keeps_the_chip() {
    assert_eq!(
        rows(&missing(), Mode::OneLine, 120.),
        vec![vec![piece(0., "Not at ", Kind::Plain), piece(70., "~/G…", Kind::Path)]]
    );
}
