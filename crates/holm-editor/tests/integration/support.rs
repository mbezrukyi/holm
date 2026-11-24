use std::path::PathBuf;

/// The folder of the files that the test notes link to.
pub(crate) fn data_folder() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/test_data/images"))
}
