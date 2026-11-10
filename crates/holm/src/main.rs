//! Holm: a calm, keyboard-first notes app for Linux.
//!
//! The binary joins the core and the editor. It owns windows, settings and themes.

mod bar;
mod changes;
mod config;
mod crash;
mod dialog;
mod error;
mod icons;
mod log;
mod notes;
mod pane;
mod picker;
mod popup;
mod sidebar;
mod text;
mod theme;
mod window;

use std::cell::Cell;
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use config::Config;
use error::AppError;
use window::Startup;

fn main() -> ExitCode {
    let startup = match start() {
        Ok(startup) => startup,
        Err(error) => return fail(&error),
    };
    let failed = Rc::new(Cell::new(false));
    gpui_kit::application().with_assets(icons::Icons).run({
        let failed = failed.clone();
        move |cx| {
            gpui_kit::init(cx);
            theme::init(cx);
            if let Err(error) = window::open(startup, cx) {
                fail(&error);
                failed.set(true);
                cx.quit();
            }
        }
    });
    if failed.get() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn fail(error: &AppError) -> ExitCode {
    tracing::error!(error = %chain(error), "Holm cannot start");
    eprintln!("{}", error.message());
    ExitCode::FAILURE
}

/// Starts the log and the panic hook, and reads the config.
fn start() -> Result<Startup, AppError> {
    let state = folder("XDG_STATE_HOME", ".local/state")?;
    let log = log::start(&state)?;
    crash::install(&state);
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Holm starts");

    let crash_log = crash::take_mark(&state).then_some(log);
    let config = Config::load(folder("XDG_CONFIG_HOME", ".config")?.join("config.toml"));
    tracing::debug!(settings = ?config.settings(), "settings");
    Ok(Startup {
        open_core: Arc::new(|folder, index_dir| holm_core::Core::open(folder, index_dir)),
        config,
        index_dir: folder("XDG_CACHE_HOME", ".cache")?,
        default_folder: home()?.join("Notes"),
        crash_log,
    })
}

fn home() -> Result<PathBuf, AppError> {
    env::home_dir()
        .filter(|home| home.is_absolute())
        .ok_or(AppError::NoHome)
}

/// The `holm` folder in an XDG base folder: `$var`, or `fallback` in the home folder.
fn folder(var: &str, fallback: &str) -> Result<PathBuf, AppError> {
    let base = match env::var_os(var).map(PathBuf::from) {
        Some(base) if base.is_absolute() => base,
        _ => home()?.join(fallback),
    };
    Ok(base.join("holm"))
}

/// The error and each of its sources, on one line.
pub(crate) fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(error) = source {
        text = format!("{text}: {error}");
        source = error.source();
    }
    text
}
