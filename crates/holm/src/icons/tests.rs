use super::*;

#[test]
fn every_icon_loads_by_its_path() {
    for icon in Icon::EVERY {
        let file = Icons.load(icon.file().0).ok().flatten();
        assert!(file.is_some_and(|bytes| bytes.starts_with(b"<svg")), "{icon:?}");
    }
    assert_eq!(Icons.list("icons/").unwrap().len(), Icon::EVERY.len());
}
