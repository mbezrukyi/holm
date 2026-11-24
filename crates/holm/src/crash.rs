//! The panic hook and the crash mark.
//!
//! The hook writes the panic and a backtrace to the log, and leaves a mark file. The next start takes the mark and
//! tells the user.

use std::backtrace::Backtrace;
use std::path::Path;
use std::{fs, io, panic};

const MARK: &str = "crashed";

/// The hook leaves the mark in `dir`, then runs the hook it replaces.
pub(crate) fn install(dir: &Path) {
    let mark = dir.join(MARK);
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let backtrace = Backtrace::force_capture();
        tracing::error!("{info}\n{backtrace}");
        if let Err(error) = fs::write(&mark, "") {
            tracing::error!(path = %mark.display(), %error, "cannot write the crash mark");
        }
        previous(info);
    }));
}

/// Whether the last run ended in a panic. It removes the mark.
pub(crate) fn take_mark(dir: &Path) -> bool {
    let mark = dir.join(MARK);
    match fs::remove_file(&mark) {
        Ok(()) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => {
            tracing::warn!(path = %mark.display(), %error, "cannot remove the crash mark");
            true
        }
    }
}

#[cfg(test)]
mod tests;
