//! The editor pane: one note in the editor, with autosave.
//!
//! The pane saves a short time after the user stops typing, and when the app quits.
//! An outside change of the note reloads it quietly when the pane has no unsaved
//! changes. Else a quiet bar asks to reload or to keep the text in the editor.
//! When a save fails, a quiet bar shows the error, and the text stays in the editor.

use std::borrow::Cow;
use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::{Theme as Ui, v_flex};
use gpui_kit::{
    AppContext as _, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    Styled, Subscription, Task, Window, div,
};
use holm_core::Core;
use holm_core::model::{Change, Note};
use holm_editor::{Changed, Editor};

use crate::bar::{self, Bar};
use crate::error::AppError;
use crate::{changes, text, theme};

/// How long the pane waits after the last change before it saves.
const AUTOSAVE: Duration = Duration::from_millis(500);

pub(crate) struct NotePane {
    core: Core,
    notes_folder: PathBuf,
    editor: Entity<Editor>,
    /// The note as it is on disk, after the last read or save.
    note: Note,
    ending: LineEnding,
    /// The editor text that matches the file on disk.
    saved: String,
    autosave: Option<Task<()>>,
    saving: Option<Task<()>>,
    /// The note changed on disk while the editor had unsaved changes.
    conflict: bool,
    error: Option<&'static str>,
    _changes: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl NotePane {
    pub(crate) fn new(
        core: Core,
        notes_folder: PathBuf,
        note: Note,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(Editor::new);
        let changes = changes::watch(&core, cx, Self::outside_change);
        let subscriptions = vec![
            cx.subscribe(&editor, |pane, _, _: &Changed, cx| pane.schedule_save(cx)),
            cx.observe_global::<Ui>(|pane, cx| {
                let appearance = theme::appearance(cx);
                pane.editor
                    .update(cx, |editor, cx| editor.set_appearance(appearance, cx));
            }),
            cx.on_app_quit(|pane, cx| {
                pane.autosave = None;
                // Unsaved text wins over an outside change at quit.
                let save = if pane.is_dirty(cx) {
                    pane.save_task(true, cx)
                } else {
                    None
                };
                async move {
                    if let Some(save) = save {
                        save.await;
                    }
                }
            }),
        ];
        let appearance = theme::appearance(cx);
        editor.update(cx, |editor, cx| editor.set_appearance(appearance, cx));
        window.focus(&editor.focus_handle(cx), cx);
        let mut pane = Self {
            core,
            notes_folder,
            editor,
            note: note.clone(),
            ending: LineEnding::Lf,
            saved: String::new(),
            autosave: None,
            saving: None,
            conflict: false,
            error: None,
            _changes: changes,
            _subscriptions: subscriptions,
        };
        pane.show(note, false, cx);
        pane
    }

    /// Puts `note` from the disk into the editor. A reload keeps the view and is one undo step.
    fn show(&mut self, note: Note, reload: bool, cx: &mut Context<Self>) {
        self.ending = LineEnding::of(&note.body);
        let folder = note
            .info
            .path
            .parent()
            .map(|parent| self.notes_folder.join(parent));
        self.saved = self.editor.update(cx, |editor, cx| {
            if reload {
                editor.reload_text(&note.body, cx);
            } else {
                editor.set_text(&note.body, cx);
                editor.set_folder(folder, cx);
            }
            editor.text().to_owned()
        });
        self.note = note;
        self.conflict = false;
        self.error = None;
        cx.notify();
    }

    fn is_dirty(&self, cx: &Context<Self>) -> bool {
        self.editor.read(cx).text() != self.saved
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.autosave = Some(cx.spawn(async move |pane, cx| {
            cx.background_executor().timer(AUTOSAVE).await;
            pane.update(cx, |pane, cx| pane.save(false, cx)).ok();
        }));
    }

    /// Saves the editor text. With no `overwrite`, it saves only unsaved changes with no conflict.
    fn save(&mut self, overwrite: bool, cx: &mut Context<Self>) {
        if self.saving.is_none() {
            self.saving = self.save_task(overwrite, cx);
        }
    }

    fn save_task(&mut self, overwrite: bool, cx: &mut Context<Self>) -> Option<Task<()>> {
        if !overwrite && (self.conflict || !self.is_dirty(cx)) {
            return None;
        }
        let text = self.editor.read(cx).text().to_owned();
        let mut note = self.note.clone();
        note.body = self.ending.to_disk(&text).into_owned();
        let core = self.core.clone();
        let job = cx.background_spawn(async move { core.save(&note) });
        Some(cx.spawn(async move |pane, cx| {
            let result = job.await;
            pane.update(cx, |pane, cx| pane.saved(text, result, cx))
                .ok();
        }))
    }

    fn saved(&mut self, text: String, result: holm_core::Result<Note>, cx: &mut Context<Self>) {
        self.saving = None;
        match result {
            Ok(note) => {
                self.note = note;
                self.saved = text;
                self.conflict = false;
                self.error = None;
                if self.is_dirty(cx) {
                    self.schedule_save(cx);
                }
            }
            Err(error) => {
                let error = AppError::from(error);
                tracing::warn!(error = %crate::chain(&error), "cannot save the note");
                self.error = Some(error.message());
            }
        }
        cx.notify();
    }

    fn outside_change(&mut self, change: Change, cx: &mut Context<Self>) {
        match change {
            Change::Changed(info) if info.path == self.note.info.path => {
                if self.is_dirty(cx) || self.saving.is_some() {
                    self.conflict = true;
                    cx.notify();
                } else {
                    self.reload(cx);
                }
            }
            Change::Moved { from, note } if from == self.note.info.path => {
                self.note.info.path = note.path;
            }
            _ => {}
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let path = self.note.info.path.clone();
        let job = cx.background_spawn(async move { core.read(path) });
        cx.spawn(async move |pane, cx| {
            let result = job.await;
            pane.update(cx, |pane, cx| match result {
                Ok(note) => pane.show(note, true, cx),
                Err(error) => {
                    let error = AppError::from(error);
                    tracing::warn!(error = %crate::chain(&error), "cannot reload the note");
                    pane.error = Some(error.message());
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn keep_mine(&mut self, cx: &mut Context<Self>) {
        self.conflict = false;
        self.autosave = None;
        self.save(true, cx);
        cx.notify();
    }
}

#[cfg(test)]
impl NotePane {
    pub(crate) fn text(&self, cx: &gpui_kit::App) -> String {
        self.editor.read(cx).text().to_owned()
    }

    pub(crate) fn has_conflict(&self) -> bool {
        self.conflict
    }
}

impl Focusable for NotePane {
    fn focus_handle(&self, cx: &gpui_kit::App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Render for NotePane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let conflict = self.conflict.then(|| {
            Bar::new("changed-on-disk", text::CHANGED_ON_DISK)
                .button(
                    bar::button("reload", text::RELOAD)
                        .on_click(cx.listener(|pane, _, _, cx| pane.reload(cx))),
                )
                .button(
                    bar::button("keep-mine", text::KEEP_MINE)
                        .on_click(cx.listener(|pane, _, _, cx| pane.keep_mine(cx))),
                )
        });
        let error = self.error.map(|message| Bar::new("save-error", message));
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .children(conflict)
            .children(error)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .font_family(theme::NOTE_FONT)
                    .text_size(theme::NOTE_SIZE)
                    .line_height(theme::NOTE_LINE)
                    .child(self.editor.clone()),
            )
    }
}

/// The line break of a note. The editor works with LF only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    /// The line break of most lines in `body`.
    fn of(body: &str) -> Self {
        let crlf = body.matches("\r\n").count();
        if crlf > 0 && crlf * 2 >= body.matches('\n').count() {
            Self::CrLf
        } else {
            Self::Lf
        }
    }

    /// `text` from the editor with this line break.
    fn to_disk(self, text: &str) -> Cow<'_, str> {
        match self {
            Self::Lf => Cow::Borrowed(text),
            Self::CrLf => Cow::Owned(text.replace('\n', "\r\n")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_keeps_its_line_breaks() {
        assert_eq!(LineEnding::of("a\nb\n"), LineEnding::Lf);
        assert_eq!(LineEnding::of("a\r\nb\r\n"), LineEnding::CrLf);
        assert_eq!(LineEnding::of("a\r\nb\nc\n"), LineEnding::Lf);
        assert_eq!(LineEnding::of(""), LineEnding::Lf);
        assert_eq!(LineEnding::CrLf.to_disk("a\nb\n"), "a\r\nb\r\n");
        assert_eq!(LineEnding::Lf.to_disk("a\nb\n"), "a\nb\n");
    }
}
