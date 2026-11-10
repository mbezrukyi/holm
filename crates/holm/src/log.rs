//! The log: one file per day in the state folder, kept 7 days.
//!
//! `holm.log` links to the file of today. The level is `info`. `HOLM_LOG` changes it.
//! The log never holds note text, only paths and ids.

use std::path::{Path, PathBuf};

use tracing::level_filters::LevelFilter;
use tracing_appender::rolling::{RollingFileAppender, Rotation};

use crate::error::AppError;

const LINK: &str = "holm.log";
const DAYS: usize = 7;

/// Starts the log in `dir` and returns the path of `holm.log`.
pub(crate) fn start(dir: &Path) -> Result<PathBuf, AppError> {
    let error = |source: Box<dyn std::error::Error + Send + Sync>| AppError::Log {
        path: dir.to_path_buf(),
        source,
    };
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("holm")
        .filename_suffix("log")
        .max_log_files(DAYS)
        .latest_symlink(LINK)
        .build(dir)
        .map_err(|e| error(e.into()))?;
    let value = std::env::var("HOLM_LOG").ok();
    let level = value.as_deref().and_then(level);
    tracing_subscriber::fmt()
        .with_writer(appender)
        .with_max_level(level.unwrap_or(LevelFilter::INFO))
        .try_init()
        .map_err(error)?;
    if let (Some(value), None) = (&value, level) {
        tracing::warn!(value, "HOLM_LOG is not a log level, so the log uses info");
    }
    Ok(dir.join(LINK))
}

fn level(value: &str) -> Option<LevelFilter> {
    value.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holm_log_sets_the_level() {
        assert_eq!(level("debug"), Some(LevelFilter::DEBUG));
        assert_eq!(level("loud"), None);
    }
}
