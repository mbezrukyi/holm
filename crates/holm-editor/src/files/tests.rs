use std::fs;

use super::*;

fn data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("test_data").join(name)
}

#[test]
fn the_kind_of_a_path_comes_from_the_disk() {
    assert_eq!(FileKind::of(&data("images")), FileKind::Folder);
    let path = data("notes/chunk.md");
    let size = path.metadata().map_or_default(|meta| meta.len());
    assert_eq!(FileKind::of(&path), FileKind::File(size));
    assert_eq!(FileKind::of(&data("images/gone.pdf")), FileKind::Missing);
    assert!(matches!(FileKind::of(&data("images/cat.png")), FileKind::Image(_)));
}

#[test]
fn a_relative_target_starts_at_the_folder() {
    let mut files = LinkedFiles::default();
    assert_eq!(files.path("docs/a.pdf"), Path::new("docs/a.pdf"));
    files.set_folder(Some(data("images")));
    assert_eq!(files.path("cat.png"), data("images/cat.png"));
    assert!(matches!(files.kind("cat.png"), FileKind::Image(_)));
    assert_eq!(files.path("/abs/a.pdf"), Path::new("/abs/a.pdf"));
}

#[test]
fn the_disk_is_read_once_for_each_path() -> std::io::Result<()> {
    let folder = std::env::temp_dir().join(format!("holm-linked-{}", std::process::id()));
    fs::create_dir_all(&folder)?;
    fs::write(folder.join("a.txt"), "abc")?;
    let mut files = LinkedFiles::default();
    files.set_folder(Some(folder.clone()));
    assert_eq!(files.kind("a.txt"), FileKind::File(3));
    fs::remove_dir_all(&folder)?;
    assert_eq!(files.kind("a.txt"), FileKind::File(3));
    files.set_folder(Some(folder));
    assert_eq!(files.kind("a.txt"), FileKind::Missing);
    Ok(())
}
