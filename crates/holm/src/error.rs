//! The app error. It wraps the library errors, and each case has one line of UI text.

use std::io;
use std::path::PathBuf;

use holm_core::NameProblem;

use crate::text;

#[derive(Debug, thiserror::Error)]
pub(crate) enum AppError {
    #[error(transparent)]
    Core(#[from] holm_core::Error),
    #[error(transparent)]
    Markdown(#[from] holm_markdown::Error),
    #[error(transparent)]
    Editor(#[from] holm_editor::Error),
    #[error("cannot write the config file {path}")]
    ConfigWrite { path: PathBuf, source: io::Error },
    #[error("cannot start the log in {path}")]
    Log {
        path: PathBuf,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("cannot find the home folder")]
    NoHome,
    #[error("cannot open the window: {0}")]
    Window(String),
}

impl AppError {
    /// The line the UI shows for this error.
    pub(crate) fn message(&self) -> &'static str {
        match self {
            Self::Core(error) => match error {
                holm_core::Error::NotesFolder { .. } => text::NOTES_FOLDER,
                holm_core::Error::Read { .. } => text::READ,
                holm_core::Error::NotUtf8 { .. } => text::NOT_UTF8,
                holm_core::Error::Write { .. } => text::WRITE,
                holm_core::Error::BadPath { .. } => text::BAD_PATH,
                holm_core::Error::BadTag { .. } => text::BAD_TAG,
                holm_core::Error::NameTaken { .. } => text::NAME_TAKEN,
                holm_core::Error::BadName { problem, .. } => match problem {
                    NameProblem::Empty => text::NAME_EMPTY,
                    NameProblem::Slash => text::NAME_SLASH,
                    NameProblem::Dot => text::NAME_DOT,
                    NameProblem::Attachments => text::NAME_ATTACHMENTS,
                    NameProblem::TooLong => text::NAME_TOO_LONG,
                    _ => text::BAD_PATH,
                },
                _ => text::UNKNOWN,
            },
            Self::Markdown(_) | Self::Editor(_) => text::UNKNOWN,
            Self::ConfigWrite { .. } => text::CONFIG_WRITE,
            Self::Log { .. } => text::LOG,
            Self::NoHome => text::NO_HOME,
            Self::Window(_) => text::WINDOW,
        }
    }
}
