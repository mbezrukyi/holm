//! The main window: the notes folder screen, the notes view, and the message line under them.
//!
//! The keys of [`Place::Anywhere`] work in each part of the window. Without a notes view, the
//! window holds the focus itself, so the keys reach it.
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
    AnyElement, App, AppContext as _, Bounds, Context, Entity, FocusHandle,
    InteractiveElement as _, IntoElement, ParentElement, PathPromptOptions, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled, Subscription, TestSupportExt as _,
    TitlebarOptions, Window, WindowBounds, WindowDecorations, WindowOptions, div, px, size,
};
use holm_core::Core;

use crate::config::{AppearanceSetting, Config};
use crate::core_door::CoreDoor;
use crate::error::AppError;
use crate::log::Log;
use crate::message_line::MessageLine;
use crate::notes::NotesView;
use crate::path_text::PathText;
use crate::shortcuts::{Command, Place, Run};
use crate::window_step::window_step;
use crate::{core_door, text, theme};

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
    /// How long a file action waits for its job before the next frame.
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
    /// The notes folder that the window shows or opens. It differs from the settings while the
    /// open folder writes its text before a change.
    folder: Option<PathBuf>,
    /// The open notes folder writes its text before a change of the folder.
    leaving: bool,
    stage: Stage,
    message_line: Entity<MessageLine>,
    /// The focus of the window itself.
    focus: FocusHandle,
    _subscriptions: [Subscription; 3],
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
        theme::set_mode(config.settings().theme, window, cx);
        window.set_zoom(config.settings().scale.factor(), cx);
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            if this.config.read(cx).settings().theme == AppearanceSetting::System {
                theme::update_mode(AppearanceSetting::System, window, cx);
            }
        });
        let message_line = cx.new(MessageLine::new);
        // Without a log file, the crash notice has no log to point to.
        if let Some(file) = log.file().filter(|_| crashed) {
            message_line.update(cx, |line, cx| line.show(text::crashed(file), cx));
        }
        let folder = config.settings().notes_folder.clone();
        let config = cx.new(|_| config);
        let config_changed = cx.observe_in(&config, window, Self::config_changed);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        // With no focus, no key reaches the window. The notes view has its own focus home.
        let focus_lost = cx.on_focus_lost(window, |this, window, cx| {
            if !matches!(this.stage, Stage::Open(_)) {
                window.focus(&this.focus, cx);
            }
        });
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
            folder: None,
            leaving: false,
            stage: Stage::Ask,
            message_line,
            focus,
            _subscriptions: [appearance, config_changed, focus_lost],
        };
        this.show_problems(cx);
        if let Some(folder) = folder {
            this.open_folder(folder, false, window, cx);
        }
        this
    }

    /// Applies the settings to the window: the problems, the theme, the scale, and the notes
    /// folder.
    fn config_changed(&mut self, _: Entity<Config>, window: &mut Window, cx: &mut Context<Self>) {
        self.show_problems(cx);
        let settings = self.config.read(cx).settings();
        let (mode, scale, folder) = (
            settings.theme,
            settings.scale,
            settings.notes_folder.clone(),
        );
        theme::update_mode(mode, window, cx);
        window.set_zoom(scale.factor(), cx);
        if let Some(folder) = folder
            && self.folder.as_ref() != Some(&folder)
        {
            self.change_folder(folder, window, cx);
        }
    }

    /// A new notes folder in the settings. The open folder writes its text first. A failed
    /// write keeps the open folder, also in the settings.
    fn change_folder(&mut self, folder: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Stage::Open(notes) = &self.stage else {
            self.choose(folder, false, window, cx);
            return;
        };
        if self.leaving {
            return;
        }
        self.leaving = true;
        let left = window_step(window, cx, |this: &mut Self, saved, window, cx| {
            this.leaving = false;
            let new = this.config.read(cx).settings().notes_folder.clone();
            match new {
                Some(new) if saved => {
                    if this.folder.as_ref() != Some(&new) {
                        this.choose(new, false, window, cx);
                    }
                }
                _ => {
                    let old = this.folder.clone();
                    this.config.update(cx, |config, cx| {
                        config.change(|settings| settings.notes_folder = old, cx);
                    });
                }
            }
        });
        notes.update(cx, |notes, cx| notes.leave(left, cx));
    }

    /// Shows the problems of the settings and the log in the message line.
    fn show_problems(&self, cx: &mut Context<Self>) {
        let problems = [self.config.read(cx).problem(), self.log.problem()]
            .into_iter()
            .flatten()
            .map(SharedString::from)
            .collect();
        self.message_line
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

    /// Runs a key of [`Place::Anywhere`]. Other commands go on to the parts under the window.
    fn run_key(&mut self, Run(command): &Run, window: &mut Window, cx: &mut Context<Self>) {
        match command {
            Command::Quit => self.quit(window, cx),
            Command::Problems => self
                .message_line
                .update(cx, |line, cx| line.toggle_problems(window, cx)),
            Command::DiskState => self
                .message_line
                .update(cx, |line, cx| line.toggle_state_menu(window, cx)),
            _ => cx.propagate(),
        }
    }

    /// Closes the window, like its close button.
    fn quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.may_close(window, cx) {
            window.remove_window();
        }
    }

    /// Opens the notes folder and its first note. `create` makes the folder first.
    fn open_folder(
        &mut self,
        folder: PathBuf,
        create: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.folder = Some(folder.clone());
        self.stage = Stage::Opening;
        cx.notify();
        let index_dir = self.index_dir.clone();
        let open = self.open_core.clone();
        let settings = self.config.read(cx).settings();
        let (last, order) = (settings.last_note.clone(), settings.note_order);
        let message_line = self.message_line.clone();
        let budget = self.write_budget;
        let job =
            cx.background_spawn({
                let folder = folder.clone();
                async move {
                    core_door::open(&*open, &folder, &index_dir, create, last.as_ref(), order)
                }
            });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            this.update_in(cx, |this, window, cx| {
                // A later open wins.
                if this.folder.as_ref() != Some(&folder) {
                    return;
                }
                this.stage = match result {
                    Ok((core, note)) => {
                        let folder = folder.clone();
                        let config = this.config.clone();
                        let core_door =
                            cx.new(|cx| CoreDoor::new(core, folder, budget, window, cx));
                        Stage::Open(cx.new(|cx| {
                            NotesView::new(core_door, note, config, message_line, window, cx)
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
        self.config.update(cx, |config, cx| {
            config.change(
                |settings| {
                    settings.notes_folder = Some(folder.clone());
                    settings.last_note = None;
                },
                cx,
            );
        });
        self.open_folder(folder, create, window, cx);
    }

    fn pick(window: &mut Window, cx: &mut Context<Self>) {
        let picked = pick_folder(cx);
        cx.spawn_in(window, async move |this, cx| {
            if let Some(folder) = picked.await {
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

/// Asks for a folder in the folder picker. It gives `None` when the user cancels or the picker
/// fails.
pub(crate) fn pick_folder(cx: &App) -> impl Future<Output = Option<PathBuf>> + 'static {
    let paths = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some(text::PICK_PROMPT.into()),
    });
    async move {
        match paths.await {
            Ok(Ok(Some(paths))) => paths.into_iter().next(),
            Ok(Err(error)) => {
                tracing::warn!(%error, "the folder picker failed");
                None
            }
            _ => None,
        }
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
            .key_context(Place::Anywhere.key_context())
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::run_key))
            .child(self.folder_screen(cx))
            .map(|root| {
                if matches!(self.stage, Stage::Open(_)) {
                    root.child(self.message_line.clone())
                } else {
                    // The question stays in the middle of the window, under the message line.
                    root.child(
                        div()
                            .absolute()
                            .bottom_0()
                            .w_full()
                            .child(self.message_line.clone()),
                    )
                }
            })
    }
}

#[cfg(test)]
#[path = "window_tests.rs"]
mod tests;
