//! The main window: the notes folder screen, the notes view, and the bars from the start.
//!
//! The first start asks for the notes folder. A notes folder that Holm can't open shows
//! a screen that offers to pick another one.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, ElementId, Entity, InteractiveElement as _,
    IntoElement, ParentElement, PathPromptOptions, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, TestSupportExt as _, TitlebarOptions,
    Window, WindowBounds, WindowDecorations, WindowOptions, div, px, size,
};
use holm_core::Core;
use holm_core::model::Note;

use crate::bar::{self, Bar};
use crate::config::{Config, Settings, Theme};
use crate::error::AppError;
use crate::notes::{self, NotesView};
use crate::{text, theme};

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
    notices: Vec<Notice>,
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

/// A bar from the start that the user closes.
struct Notice {
    id: &'static str,
    text: SharedString,
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
        let mut notices = Vec::new();
        if let Some(log) = crash_log {
            notices.push(Notice {
                id: "crashed",
                text: text::crashed(&log).into(),
            });
        }
        if config.is_broken() {
            notices.push(Notice {
                id: "config-broken",
                text: text::CONFIG_BROKEN.into(),
            });
        }
        let folder = config.settings().notes_folder.clone();
        let mut this = Self {
            open_core,
            config: cx.new(|_| config),
            index_dir,
            default_folder,
            stage: Stage::Ask,
            notices,
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
        let job = cx.background_spawn({
            let folder = folder.clone();
            async move { open_notes(&*open, &folder, &index_dir, create, last.as_deref()) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            this.update_in(cx, |this, window, cx| {
                this.stage = match result {
                    Ok((core, note)) => {
                        let folder = folder.clone();
                        let config = this.config.clone();
                        Stage::Open(
                            cx.new(|cx| NotesView::new(core, folder, note, config, window, cx)),
                        )
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
            self.notices.retain(|notice| notice.id != "config-write");
            self.notices.push(Notice {
                id: "config-write",
                text: error.message().into(),
            });
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
            Stage::Open(pane) => return pane.clone().into_any_element(),
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

/// Opens the core and gives the note to show first, if any. `create` makes the folder first.
fn open_notes(
    open: &dyn Fn(&Path, &Path) -> holm_core::Result<Core>,
    folder: &Path,
    index_dir: &Path,
    create: bool,
    last: Option<&Path>,
) -> Result<(Core, Option<Note>), AppError> {
    if create && let Err(error) = std::fs::create_dir_all(folder) {
        tracing::warn!(path = %folder.display(), %error, "cannot make the notes folder");
    }
    let core = open(folder, index_dir)?;
    let note = notes::first_note(&core, last)?;
    Ok((core, note))
}

impl Render for HolmWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let notices = self.notices.iter().map(|notice| {
            let id = notice.id;
            Bar::new(id, notice.text.clone()).button(
                bar::button(ElementId::Name(format!("{id}-close").into()), text::CLOSE).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.notices.retain(|notice| notice.id != id);
                        cx.notify();
                    }),
                ),
            )
        });
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .text_size(theme.font_size)
            .children(notices.collect::<Vec<_>>())
            .child(self.folder_screen(cx))
    }
}

#[cfg(test)]
#[path = "window_tests.rs"]
mod tests;
