use super::*;

#[test]
fn the_notices_lie_in_the_doc_folder_beside_the_binary_folder() {
    assert_eq!(
        notices_beside(Path::new("/tmp/.mount_holm/usr/bin/holm")).as_deref(),
        Some(Path::new("/tmp/.mount_holm/usr/share/doc/holm/THIRD-PARTY-NOTICES.txt"))
    );
}
