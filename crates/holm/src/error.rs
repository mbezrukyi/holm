//! The app error. It wraps the library errors, and each case has one line of UI text.

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
                holm_core::Error::Changed { .. } => text::CHANGED_ON_DISK,
                holm_core::Error::BadName { problem, .. } => match problem {
                    NameProblem::Empty => text::NAME_EMPTY,
                    NameProblem::Slash => text::NAME_SLASH,
                    NameProblem::Dot => text::NAME_DOT,
                    NameProblem::Attachments => text::NAME_ATTACHMENTS,
                    NameProblem::TooLong => text::NAME_TOO_LONG,
                    NameProblem::ZeroByte => text::NAME_ZERO_BYTE,
                    _ => text::BAD_PATH,
                },
                _ => text::UNKNOWN,
            },
            Self::Markdown(_) | Self::Editor(_) => text::UNKNOWN,
            Self::NoHome => text::NO_HOME,
            Self::Window(_) => text::WINDOW,
        }
    }

    /// Whether the core refused a save after a change from another tool.
    pub(crate) fn is_outside_change(&self) -> bool {
        matches!(self, Self::Core(holm_core::Error::Changed { .. }))
    }

    pub(crate) fn is_refused_name(&self) -> bool {
        matches!(
            self,
            Self::Core(holm_core::Error::NameTaken { .. } | holm_core::Error::BadName { .. })
        )
    }
}
