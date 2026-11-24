use std::time::Duration;

use super::*;

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn replace_writes_the_new_text_and_leaves_no_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.md");
    fs::write(&path, "old").unwrap();

    replace(&path, "new", Permissions::from_mode(0o644), None).unwrap();

    assert_eq!(fs::read_to_string(&path).unwrap(), "new");
    assert_eq!(names(dir.path()), ["a.md"]);
}

#[test]
fn replace_sets_the_permissions_and_the_modification_time() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.md");
    fs::write(&path, "old").unwrap();
    let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);

    replace(&path, "new", Permissions::from_mode(0o640), Some(time)).unwrap();

    let meta = fs::metadata(&path).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o640);
    assert_eq!(meta.modified().unwrap(), time);
}

#[test]
fn a_failed_replace_keeps_the_old_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.md");
    fs::write(&path, "old").unwrap();
    // A folder can't take the place of a file, so the rename fails.
    let target = dir.path().join("folder.md");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("x"), "").unwrap();

    assert!(replace(&target, "new", Permissions::from_mode(0o644), None).is_err());

    assert_eq!(fs::read_to_string(&path).unwrap(), "old");
    assert_eq!(names(dir.path()), ["a.md", "folder.md"]);
}
