//! Holm: a calm, keyboard-first notes app for Linux.
//!
//! The binary joins the core and the editor. It owns windows, settings and themes.

mod config;
mod crash;
mod error;
mod log;
mod text;

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use config::Config;
use error::AppError;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = %chain(&error), "Holm cannot start");
            eprintln!("{}", error.message());
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), AppError> {
    let state = folder("XDG_STATE_HOME", ".local/state")?;
    let log = log::start(&state)?;
    crash::install(&state);
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Holm starts");

    if crash::take_mark(&state) {
        eprintln!("{}", text::crashed(&log));
    }
    let config = Config::load(folder("XDG_CONFIG_HOME", ".config")?.join("config.toml"));
    if config.is_broken() {
        eprintln!("{}", text::CONFIG_BROKEN);
    }
    tracing::debug!(settings = ?config.settings(), "settings");
    Ok(())
}

/// The `holm` folder in an XDG base folder: `$var`, or `fallback` in the home folder.
fn folder(var: &str, fallback: &str) -> Result<PathBuf, AppError> {
    let base = match env::var_os(var).map(PathBuf::from) {
        Some(base) if base.is_absolute() => base,
        _ => env::home_dir()
            .filter(|home| home.is_absolute())
            .ok_or(AppError::NoHome)?
            .join(fallback),
    };
    Ok(base.join("holm"))
}

/// The error and each of its sources, on one line.
fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(error) = source {
        text = format!("{text}: {error}");
        source = error.source();
    }
    text
}
