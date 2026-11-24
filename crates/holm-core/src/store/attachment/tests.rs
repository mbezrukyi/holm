use super::*;

fn folder_path(path: &str) -> FolderPath {
    FolderPath::new(path).unwrap()
}

fn name(name: &str) -> Name {
    Name::typed(name).unwrap()
}

#[test]
fn file_stems() {
    assert_eq!(file_stem("Plans for 2026"), "Plans for 2026");
    assert_eq!(file_stem("a/b/c"), "a-b-c");
    assert_eq!(file_stem("..dots"), "-.dots");
    assert_eq!(file_stem("Ünïcode: ok?"), "Ünïcode: ok?");
}

#[test]
fn a_long_file_stem_stops_at_a_char() {
    let text = "é".repeat(200);
    let stem = file_stem(&text);
    assert_eq!(stem.len(), MAX_STEM_BYTES);
    assert!(text.starts_with(&stem));
}

#[test]
fn a_new_name_takes_the_folder_in_any_case_only_at_the_root() {
    let root = FolderPath::root();
    assert!(reserves(&root, &name("attachments")));
    assert!(reserves(&root, &name("Attachments")));
    assert!(reserves(&root, &name("ATTACHMENTS")));
    assert!(!reserves(&root, &name("attachments2")));
    assert!(!reserves(&folder_path("work"), &name("attachments")));
}

#[test]
fn only_the_exact_folder_holds_attachments() {
    assert_eq!(folder(), Some(folder_path("attachments")));
    assert!(holds(&folder_path("attachments")));
    assert!(holds(&folder_path("attachments/2026")));
    assert!(!holds(&folder_path("Attachments")));
    assert!(!holds(&folder_path("attachments2")));
    assert!(!holds(&folder_path("work/attachments")));
    assert!(!holds(&FolderPath::root()));
}

#[test]
fn only_the_exact_folder_is_the_attachments_folder() {
    assert!(is(&folder_path("attachments")));
    assert!(!is(&folder_path("Attachments")));
    assert!(!is(&folder_path("attachments/2026")));
    assert!(!is(&FolderPath::root()));
}
