//! All UI text, in English. Other modules take their text from here.

use std::path::Path;

pub(crate) const NOTES_FOLDER: &str = "Holm can't open the notes folder.";
pub(crate) const READ: &str = "Holm can't read a file in the notes folder.";
pub(crate) const NOT_UTF8: &str = "This file is not plain text, so Holm can't open it.";
pub(crate) const WRITE: &str =
    "Holm can't save the change. Check the free disk space and the folder permissions.";
pub(crate) const INDEX: &str = "Holm can't use its search index.";
pub(crate) const BAD_PATH: &str = "Holm can't use this name or place.";
pub(crate) const BAD_TAG: &str = "This is not a tag name.";
pub(crate) const UNKNOWN: &str = "Something went wrong. The log has the details.";
pub(crate) const CONFIG_WRITE: &str = "Holm can't save the settings.";
pub(crate) const LOG: &str = "Holm can't write its log.";
pub(crate) const NO_HOME: &str = "Holm can't find your home folder.";
pub(crate) const CONFIG_BROKEN: &str =
    "The config file has an error. Holm uses the default settings and does not change the file.";

pub(crate) fn crashed(log: &Path) -> String {
    format!(
        "Holm closed after an error. The log is at {}.",
        log.display()
    )
}
