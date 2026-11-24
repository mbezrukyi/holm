use super::*;

#[test]
fn a_panic_leaves_a_mark_for_the_next_start() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!take_mark(dir.path()));

    install(dir.path());
    let result = std::thread::spawn(|| panic!("test panic")).join();
    drop(panic::take_hook());

    assert!(result.is_err());
    assert!(take_mark(dir.path()));
    assert!(!take_mark(dir.path()));
}
