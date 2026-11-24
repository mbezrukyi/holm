//! The log: one file per day in the state folder, kept 7 days.
//!
//! `holm.log` links to the file of today. The level is `info`. `HOLM_LOG` changes it. The log never holds note text,
//! only paths and ids. When Holm can't write the log file, it logs to the terminal, and the log has a problem.

use std::error::Error;
use std::path::{Path, PathBuf};

use tracing::level_filters::LevelFilter;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::MakeWriter;

use crate::text;

const LINK: &str = "holm.log";
const DAYS: usize = 7;

pub(crate) struct Log {
    /// `holm.log`, or `None` when the log goes to the terminal.
    file: Option<PathBuf>,
}

impl Log {
    pub(crate) fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    /// Holm can't write the log file.
    pub(crate) fn problem(&self) -> Option<&'static str> {
        self.file.is_none().then_some(text::LOG)
    }
}

pub(crate) fn start(dir: &Path) -> Log {
    let value = std::env::var("HOLM_LOG").ok();
    let level = value.as_deref().and_then(level);
    let max = level.unwrap_or(LevelFilter::INFO);
    let file = match to_file(dir, max) {
        Ok(()) => Some(dir.join(LINK)),
        Err(error) => {
            init(std::io::stderr, max).ok();
            tracing::warn!(%error, path = %dir.display(), "cannot write the log file, so Holm logs to the terminal");
            None
        }
    };
    if let (Some(value), None) = (&value, level) {
        tracing::warn!(value, "HOLM_LOG is not a log level, so the log uses info");
    }
    Log { file }
}

fn to_file(dir: &Path, max: LevelFilter) -> Result<(), Box<dyn Error + Send + Sync>> {
    std::fs::create_dir_all(dir)?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("holm")
        .filename_suffix("log")
        .max_log_files(DAYS)
        .latest_symlink(LINK)
        .build(dir)?;
    init(appender, max)
}

fn init<W>(writer: W, max: LevelFilter) -> Result<(), Box<dyn Error + Send + Sync>>
where
    W: for<'w> MakeWriter<'w> + Send + Sync + 'static,
{
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_max_level(max)
        .try_init()
}

fn level(value: &str) -> Option<LevelFilter> {
    value.trim().parse().ok()
}

#[cfg(test)]
impl Log {
    pub(crate) fn at(file: PathBuf) -> Self {
        Self { file: Some(file) }
    }
}

#[cfg(test)]
mod tests;
