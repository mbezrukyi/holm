use super::*;

#[test]
fn a_note_keeps_its_line_breaks() {
    assert_eq!(LineEnding::of("a\nb\n"), LineEnding::Lf);
    assert_eq!(LineEnding::of("a\r\nb\r\n"), LineEnding::CrLf);
    assert_eq!(LineEnding::of("a\r\nb\nc\n"), LineEnding::Lf);
    assert_eq!(LineEnding::of(""), LineEnding::Lf);
    assert_eq!(LineEnding::CrLf.to_disk("a\nb\n"), "a\r\nb\r\n");
    assert_eq!(LineEnding::Lf.to_disk("a\nb\n"), "a\nb\n");
}
