//! The main window: the notes folder screen, the notes view, and the message line under them.
//!
//! The first start asks for the notes folder. A notes folder that Holm can't open shows
//! a screen that offers to pick another one.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, Entity, InteractiveElement as _,
    IntoElement, ParentElement, PathPromptOptions, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, TestSupportExt as _, TitlebarOptions,
    Window, WindowBounds, WindowDecorations, WindowOptions, div, px, size,
};
use holm_core::Core;

use crate::config::{Config, Settings, Theme};
use crate::error::AppError;
use crate::log::Log;
use crate::message_line::MessageLine;
use crate::notes::NotesView;
use crate::notes_folder::NotesFolder;
use crate::path_text::PathText;
use crate::{notes_folder, text, theme};

pub(crate) const APP_ID: &str = "io.github.mbezrukyi.Holm";

/// Opens the core on a notes folder and an index folder. The app uses [`Core::open`].
pub(crate) type OpenCore = Arc<dyn Fn(&Path, &Path) -> holm_core::Result<Core> + Send + Sync>;

/// What the window needs from the start of the app.
pub(crate) struct Startup {
    pub(crate) open_core: OpenCore,
    pub(crate) config: Config,
    pub(crate) index_dir: PathBuf,
    /// `~/Notes`.
    pub(crate) default_folder: PathBuf,
    /// Whether the last run ended in a panic.
    pub(crate) crashed: bool,
    pub(crate) log: Log,
    /// How long a file action waits for its write before the next frame.
    pub(crate) write_budget: Duration,
}

/// Opens the main window. The app quits when it closes.
pub(crate) fn open(startup: Startup, cx: &mut App) -> Result<(), AppError> {
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(text::TITLE.into()),
            ..TitlebarOptions::default()
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1000.), px(800.)),
            cx,
        ))),
        app_id: Some(APP_ID.to_owned()),
        window_decorations: Some(WindowDecorations::Server),
        ..WindowOptions::default()
    };
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| HolmWindow::new(startup, window, cx))
    })
    .map_err(|error| AppError::Window(error.to_string()))?;
    cx.on_window_closed(|cx, _| cx.quit()).detach();
    Ok(())
}

pub(crate) struct HolmWindow {
    open_core: OpenCore,
    config: Entity<Config>,
    log: Log,
    index_dir: PathBuf,
    default_folder: PathBuf,
    write_budget: Duration,
    stage: Stage,
    line: Entity<MessageLine>,
    _subscriptions: [Subscription; 2],
}

enum Stage {
    /// The first start: no notes folder yet.
    Ask,
    Opening,
    /// Holm can't open the notes folder.
    Failed(PathText),
    Open(Entity<NotesView>),
}

impl HolmWindow {
    pub(crate) fn new(startup: Startup, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let Startup {
            open_core,
            config,
            index_dir,
            default_folder,
            crashed,
            log,
            write_budget,
        } = startup;
        theme::apply(config.settings().theme, window, cx);
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            if this.config.read(cx).settings().theme == Theme::System {
                theme::apply(Theme::System, window, cx);
            }
        });
        let line = cx.new(MessageLine::new);
        // Without a log file, the crash notice has no log to point to.
        if let Some(file) = log.file().filter(|_| crashed) {
            line.update(cx, |line, cx| line.show(text::crashed(file), cx));
        }
        let folder = config.settings().notes_folder.clone();
        let config = cx.new(|_| config);
        let config_changed = cx.observe(&config, |this, _, cx| this.show_problems(cx));
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            this.update(cx, |this, cx| this.may_close(window, cx))
                .unwrap_or(true)
        });
        let mut this = Self {
            open_core,
            config,
            log,
            index_dir,
            default_folder,
            write_budget,
            stage: Stage::Ask,
            line,
            _subscriptions: [appearance, config_changed],
        };
        this.show_problems(cx);
        if let Some(folder) = folder {
            this.open_folder(folder, false, window, cx);
        }
        this
    }

    /// Shows the problems of the settings and the log in the message line.
    fn show_problems(&self, cx: &mut Context<Self>) {
        let problems = [self.config.read(cx).problem(), self.log.problem()]
            .into_iter()
            .flatten()
            .map(SharedString::from)
            .collect();
        self.line
            .update(cx, |line, cx| line.set_problems(problems, cx));
    }

    /// A close of the window: gives whether it may close now. The open notes folder may keep it
    /// open for unsaved text.
    fn may_close(&self, window: &Window, cx: &mut Context<Self>) -> bool {
        match &self.stage {
            Stage::Open(notes) => notes.update(cx, |notes, cx| notes.may_close(window, cx)),
            _ => true,
        }
    }

    /// Opens the notes folder and its last changed note. `create` makes the folder first.
    fn open_folder(
        &mut self,
        folder: PathBuf,
        create: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stage = Stage::Opening;
        cx.notify();
        let index_dir = self.index_dir.clone();
        let open = self.open_core.clone();
        let last = self.config.read(cx).settings().last_note.clone();
        let line = self.line.clone();
        let budget = self.write_budget;
        let job = cx.background_spawn({
            let folder = folder.clone();
            async move { notes_folder::open(&*open, &folder, &index_dir, create, last.as_ref()) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            this.update_in(cx, |this, window, cx| {
                this.stage = match result {
                    Ok((core, note)) => {
                        let folder = folder.clone();
                        let config = this.config.clone();
                        let notes_folder = cx.new(|cx| NotesFolder::new(core, budget, window, cx));
                        Stage::Open(cx.new(|cx| {
                            NotesView::new(notes_folder, folder, note, config, line, window, cx)
                        }))
                    }
                    Err(error) => {
                        tracing::warn!(error = %crate::chain(&error), "cannot open the notes folder");
                        Stage::Failed(match error {
                            AppError::Core(holm_core::Error::NotesFolder { .. }) => {
                                text::folder_missing(&folder)
                            }
                            error => error.message().into(),
                        })
                    }
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Keeps `folder` in the settings and opens it.
    fn choose(
        &mut self,
        folder: PathBuf,
        create: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = Settings {
            notes_folder: Some(folder.clone()),
            last_note: None,
            ..self.config.read(cx).settings().clone()
        };
        self.config.update(cx, |config, cx| {
            config.save(settings);
            cx.notify();
        });
        self.open_folder(folder, create, window, cx);
    }

    fn pick(window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(text::PICK_PROMPT.into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let folder = match paths.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Err(error)) => {
                    tracing::warn!(%error, "the folder picker failed");
                    None
                }
                _ => None,
            };
            if let Some(folder) = folder {
                this.update_in(cx, |this, window, cx| {
                    this.choose(folder, false, window, cx);
                })
                .ok();
            }
        })
        .detach();
    }

    fn folder_screen(&self, cx: &mut Context<Self>) -> AnyElement {
        let pick = Button::new("pick-folder")
            .label(text::PICK_FOLDER)
            .on_click(cx.listener(|_, _, window, cx| Self::pick(window, cx)));
        let (message, content) = match &self.stage {
            Stage::Ask => (
                SharedString::from(text::ASK_FOLDER),
                v_flex()
                    .gap_2()
                    .child(div().text_lg().child(text::ASK_FOLDER))
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child(text::ABOUT_FOLDER),
                    )
                    .child(
                        h_flex()
                            .pt_4()
                            .gap_2()
                            .child(
                                Button::new("use-default")
                                    .primary()
                                    .label(text::USE_DEFAULT)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        let folder = this.default_folder.clone();
                                        this.choose(folder, true, window, cx);
                                    })),
                            )
                            .child(pick),
                    ),
            ),
            Stage::Failed(message) => (
                message.plain_text(),
                v_flex()
                    .gap_2()
                    .child(message.wrapped())
                    .child(h_flex().pt_4().child(pick.primary())),
            ),
            Stage::Opening => (
                SharedString::from(text::OPENING),
                v_flex()
                    .text_color(cx.theme().muted_foreground)
                    .child(text::OPENING),
            ),
            Stage::Open(notes) => {
                return div()
                    .flex_1()
                    .min_h_0()
                    .child(notes.clone())
                    .into_any_element();
            }
        };
        div()
            .id("folder-screen")
            .role(Role::Group)
            .aria_label(message)
            .test_support()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("folder-question")
                    .test_support()
                    .max_w(px(480.))
                    .child(content),
            )
            .into_any_element()
    }
}

impl Render for HolmWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .text_size(theme.font_size)
            .relative()
            .child(self.folder_screen(cx))
            .map(|root| {
                if matches!(self.stage, Stage::Open(_)) {
                    root.child(self.line.clone())
                } else {
                    // The question stays in the middle of the window, under the line.
                    root.child(
                        div()
                            .absolute()
                            .bottom_0()
                            .w_full()
                            .child(self.line.clone()),
                    )
                }
            })
    }
}

#[cfg(test)]
#[path = "window_tests.rs"]
mod tests;
