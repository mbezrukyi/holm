use super::*;

#[test]
fn a_log_with_no_file_has_a_problem() {
    assert_eq!(Log { file: None }.problem(), Some(text::LOG));
    let file = Log {
        file: Some(PathBuf::from("/state/holm.log")),
    };
    assert_eq!(file.problem(), None);
}

#[test]
fn holm_log_sets_the_level() {
    assert_eq!(level("debug"), Some(LevelFilter::DEBUG));
    assert_eq!(level("loud"), None);
}
