//! The main window: the notes folder screen, the notes view, and the message line under them.
//!
//! The first start asks for the notes folder. A notes folder that Holm can't open shows
//! a screen that offers to pick another one.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, Entity, InteractiveElement as _,
    IntoElement, ParentElement, PathPromptOptions, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, TestSupportExt as _, TitlebarOptions,
    Window, WindowBounds, WindowDecorations, WindowOptions, div, px, size,
};
use holm_core::Core;

use crate::config::{Config, Settings, Theme};
use crate::error::AppError;
use crate::message_line::{Answer, MessageLine};
use crate::notes::NotesView;
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
    /// The log, when the last run ended in a panic.
    pub(crate) crash_log: Option<PathBuf>,
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
    index_dir: PathBuf,
    default_folder: PathBuf,
    stage: Stage,
    line: Entity<MessageLine>,
    _appearance: Subscription,
}

enum Stage {
    /// The first start: no notes folder yet.
    Ask,
    Opening,
    /// Holm can't open the notes folder.
    Failed(SharedString),
    Open(Entity<NotesView>),
}

impl HolmWindow {
    pub(crate) fn new(startup: Startup, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let Startup {
            open_core,
            config,
            index_dir,
            default_folder,
            crash_log,
        } = startup;
        theme::apply(config.settings().theme, window, cx);
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            if this.config.read(cx).settings().theme == Theme::System {
                theme::apply(Theme::System, window, cx);
            }
        });
        let line = cx.new(MessageLine::new);
        if let Some(log) = crash_log {
            line.update(cx, |line, cx| line.show(text::crashed(&log), cx));
        }
        let folder = config.settings().notes_folder.clone();
        let mut this = Self {
            open_core,
            config: cx.new(|_| config),
            index_dir,
            default_folder,
            stage: Stage::Ask,
            line,
            _appearance: appearance,
        };
        if let Some(folder) = folder {
            this.open_folder(folder, false, window, cx);
        }
        this
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
        let job = cx.background_spawn({
            let folder = folder.clone();
            async move { notes_folder::open(&*open, &folder, &index_dir, create, last.as_deref()) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            this.update_in(cx, |this, window, cx| {
                this.stage = match result {
                    Ok((core, note)) => {
                        let folder = folder.clone();
                        let config = this.config.clone();
                        Stage::Open(cx.new(|cx| {
                            NotesView::new(core, folder, note, config, line, window, cx)
                        }))
                    }
                    Err(error) => {
                        tracing::warn!(error = %crate::chain(&error), "cannot open the notes folder");
                        Stage::Failed(match error {
                            AppError::Core(holm_core::Error::NotesFolder { .. }) => {
                                text::folder_missing(&folder).into()
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
        if let Err(error) = self.config.update(cx, |config, _| config.save(settings)) {
            tracing::warn!(error = %crate::chain(&error), "cannot save the settings");
            self.line
                .update(cx, |line, cx| line.show(error.message(), cx));
        }
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

    /// The text under the title when the config file is broken. The path is a code chip.
    /// The words wrap one by one, as plain text has no place for a chip with a padding.
    fn config_broken(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let path = text::home_path(self.config.read(cx).path());
        let word = |word: &str| div().child(word.to_owned()).into_any_element();
        let (mark, after) = mark_after_chip(text::CONFIG_BROKEN_AFTER);
        // A path wider than the text wraps inside its chip.
        let chip = h_flex()
            .max_w_full()
            .min_w_0()
            .items_baseline()
            .child(
                div()
                    .min_w_0()
                    .px(px(5.))
                    .py(px(1.))
                    .rounded(px(4.))
                    .bg(theme.accent)
                    .font_family(theme.mono_font_family.clone())
                    .text_size(px(12.5))
                    .child(path),
            )
            .child(word(mark))
            .into_any_element();
        h_flex()
            .flex_wrap()
            .items_baseline()
            .gap_x(px(4.))
            .text_color(theme.muted_foreground)
            .children(text::CONFIG_BROKEN_BEFORE.split_whitespace().map(word))
            .child(chip)
            .children(after.split_whitespace().map(word))
            .into_any_element()
    }

    fn folder_screen(&self, cx: &mut Context<Self>) -> AnyElement {
        let pick = Button::new("pick-folder")
            .label(text::PICK_FOLDER)
            .on_click(cx.listener(|_, _, window, cx| Self::pick(window, cx)));
        // A broken config file lost the notes folder, so the screen tells why it asks.
        let (title, about) = if self.config.read(cx).is_broken() {
            (text::CONFIG_BROKEN, self.config_broken(cx))
        } else {
            (
                text::ASK_FOLDER,
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(text::ABOUT_FOLDER)
                    .into_any_element(),
            )
        };
        let (message, content) = match &self.stage {
            Stage::Ask => (
                SharedString::from(title),
                v_flex()
                    .gap_2()
                    .child(div().text_lg().child(title))
                    .child(about)
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
                message.clone(),
                v_flex()
                    .gap_2()
                    .child(message.clone())
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
            .child(div().max_w(px(480.)).child(content))
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
            .on_action(cx.listener(|this, Answer(message), _, cx| {
                this.line.update(cx, |line, cx| line.show(*message, cx));
            }))
            .child(self.folder_screen(cx))
            .child(self.line.clone())
    }
}

/// Splits the text after a chip into the mark that stays on the chip, like a comma, and the
/// rest. The mark keeps no space before it.
fn mark_after_chip(after: &str) -> (&str, &str) {
    after.split_at(after.find(char::is_whitespace).unwrap_or(after.len()))
}

#[cfg(test)]
#[path = "window_tests.rs"]
mod tests;
