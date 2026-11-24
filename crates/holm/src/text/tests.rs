use super::*;

#[test]
fn a_path_in_the_home_folder_starts_with_a_tilde() {
    let home = std::env::home_dir().expect("a home folder");
    assert_eq!(
        home_path(&home.join(".config/holm/config.toml")),
        "~/.config/holm/config.toml"
    );
    assert_eq!(home_path(Path::new("/etc/holm.toml")), "/etc/holm.toml");
}

#[test]
fn the_root_folder_has_a_name() {
    assert_eq!(folder_label(&FolderPath::root()), "Notes folder");
    let folder = FolderPath::new("work/holm").expect("a plain path");
    assert_eq!(folder_label(&folder), "work/holm");
}
