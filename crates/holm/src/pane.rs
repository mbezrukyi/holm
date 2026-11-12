//! The editor pane: one note in the editor, with autosave.
//!
//! The pane saves a short time after the user stops typing, and when the app quits.
//! An outside change of the note reloads it quietly when the pane has no unsaved
//! changes. Else a quiet bar asks to reload or to keep the text in the editor.
//! When a save fails, a quiet bar shows the error, and the text stays in the editor.
//!
//! When the user opens another note, the pane saves the old one. It also renames the old file
//! to match the title.
//!
//! The tag row before the first line shows the tags and the pin of the note. It scrolls with
//! the text. The pane writes their changes at once. The editor undo stack holds only the text,
//! so Ctrl+Z does not undo them.
//!
//! The pane opens the links of the editor: a note, a heading in a note, a file in its system
//! app, or a web address. It copies pasted files into `attachments/`. After each change in the
//! notes folder, it reads the backlinks again and tells the editor which note links are broken.

use std::borrow::Cow;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::{Theme as Ui, v_flex};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, PathPromptOptions, Render, Styled, Subscription, Task, Window, div,
};
use holm_core::Core;
use holm_core::model::{Change, LinkTarget, Note, NoteInfo, Order};
use holm_editor::{Changed, Editor, OpenFile, OpenLink, PasteFiles, PastedFile, UpFromStart};
use holm_markdown::Target;

use crate::backlinks::{Backlinks, OpenBacklink};
use crate::bar::{self, Bar};
use crate::error::AppError;
use crate::link_picker::{LinkChoice, LinkStart};
use crate::tags::{TagRow, TagRowEvent};
use crate::{changes, text, theme};

/// How long the pane waits after the last change before it saves.
const AUTOSAVE: Duration = Duration::from_millis(500);

/// What the pane tells the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PaneEvent {
    /// The pane shows the note at this path.
    Opened(PathBuf),
}

/// What happens to the open note when the pane opens another one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Leave {
    /// Save it and rename its file to match the title.
    Keep,
    /// The note is gone. Drop its text.
    Drop,
}

pub(crate) struct NotePane {
    core: Core,
    notes_folder: PathBuf,
    editor: Entity<Editor>,
    tag_row: Entity<TagRow>,
    backlinks: Entity<Backlinks>,
    /// The note as it is on disk, after the last read or save. Its tags and pin can be newer:
    /// a write of them may still run.
    note: Note,
    /// Grows with each change of the tags or the pin. A read of the fields that started
    /// before a change does not undo it.
    fields_version: u64,
    /// The last write of the tags or the pin. The next write waits for it.
    writing: Option<Task<()>>,
    ending: LineEnding,
    /// The editor text that matches the file on disk.
    saved: String,
    autosave: Option<Task<()>>,
    saving: Option<Task<()>>,
    /// The note changed on disk while the editor had unsaved changes.
    conflict: bool,
    error: Option<&'static str>,
    opening: Option<Task<()>>,
    /// The heading to show when the note that opens is ready.
    open_heading: Option<String>,
    /// Whether the link picker changes the link at the cursor, not inserts a new one.
    replace_link: bool,
    links: Option<Task<()>>,
    _changes: Task<()>,
    _revisions: Task<()>,
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
        let tag_row = cx.new(|cx| TagRow::new(core.clone(), window, cx));
        let backlinks = cx.new(|_| Backlinks::default());
        editor.update(cx, |editor, cx| {
            editor.set_start(Some(tag_row.clone().into()), cx);
            editor.set_end(Some(backlinks.clone().into()), cx);
        });
        let changes = changes::watch(&core, cx, Self::outside_change);
        let revisions = changes::watch_revision(&core, cx, Self::read_links);
        let subscriptions = vec![
            cx.subscribe(&editor, |pane, _, _: &Changed, cx| pane.schedule_save(cx)),
            cx.subscribe_in(&editor, window, |pane, _, _: &UpFromStart, window, cx| {
                pane.focus_tags(window, cx);
            }),
            cx.subscribe_in(&tag_row, window, Self::tag_row_event),
            cx.subscribe(&editor, |pane, _, event: &OpenLink, cx| {
                pane.open_link(event.target.clone(), cx);
            }),
            cx.subscribe(&editor, |_, _, event: &OpenFile, cx| {
                cx.open_with_system(&event.path);
            }),
            cx.subscribe(&editor, |pane, _, event: &PasteFiles, cx| {
                pane.paste_files(event.files.clone(), cx);
            }),
            cx.subscribe(&backlinks, |pane, _, event: &OpenBacklink, cx| {
                pane.open(event.0.clone(), Leave::Keep, cx);
            }),
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
            tag_row,
            backlinks,
            note: note.clone(),
            fields_version: 0,
            writing: None,
            ending: LineEnding::Lf,
            saved: String::new(),
            autosave: None,
            saving: None,
            conflict: false,
            error: None,
            opening: None,
            open_heading: None,
            replace_link: false,
            links: None,
            _changes: changes,
            _revisions: revisions,
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
        self.fields_version += 1;
        self.show_fields(cx);
        self.conflict = false;
        self.error = None;
        if !reload && let Some(heading) = self.open_heading.take() {
            self.editor
                .update(cx, |editor, cx| editor.show_heading(&heading, cx));
        }
        self.read_links(cx);
        cx.notify();
    }

    /// Reads the backlinks and the broken note links of the open note, and shows them.
    fn read_links(&mut self, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let path = self.note.info.path.clone();
        let job = cx.background_spawn(async move {
            let backlinks = core.backlinks(&path, Order::Title)?;
            let broken = core.broken_links(&path)?;
            Ok::<_, holm_core::Error>((path, backlinks, broken))
        });
        self.links = Some(cx.spawn(async move |pane, cx| {
            let result = job.await;
            pane.update(cx, |pane, cx| match result {
                Ok((path, backlinks, broken)) if path == pane.note.info.path => {
                    pane.backlinks
                        .update(cx, |list, cx| list.show(backlinks, cx));
                    let broken: HashSet<Target> = broken.into_iter().filter_map(target).collect();
                    pane.editor
                        .update(cx, |editor, cx| editor.set_link_states(broken, cx));
                }
                Ok(_) => {}
                Err(error) => {
                    let error = AppError::from(error);
                    tracing::warn!(error = %crate::chain(&error), "cannot read the links of the note");
                }
            })
            .ok();
        }));
    }

    /// Opens a note link or a web link.
    fn open_link(&mut self, target: Target, cx: &mut Context<Self>) {
        let (id, heading) = match target {
            Target::Note { id, heading } => (id, heading),
            Target::Web(url) => {
                cx.open_url(&url);
                return;
            }
            Target::Heading(_) | Target::File(_) => return,
        };
        let core = self.core.clone();
        let job = cx.background_spawn(async move { core.note_with_id(&id) });
        cx.spawn(async move |pane, cx| {
            let found = job.await;
            pane.update(cx, |pane, cx| match found {
                Ok(Some(note)) => pane.open_at(note.path, heading, cx),
                Ok(None) => {
                    pane.error = Some(text::NOTE_GONE);
                    cx.notify();
                }
                Err(error) => {
                    let error = AppError::from(error);
                    tracing::warn!(error = %crate::chain(&error), "cannot find the note of a link");
                    pane.error = Some(error.message());
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Opens the note at `path` at `heading`, or at the top when the note has no such heading.
    fn open_at(&mut self, path: PathBuf, heading: Option<String>, cx: &mut Context<Self>) {
        if path == self.note.info.path {
            if let Some(heading) = heading {
                self.editor
                    .update(cx, |editor, cx| editor.show_heading(&heading, cx));
            }
            return;
        }
        self.open_heading = heading;
        self.open(path, Leave::Keep, cx);
    }

    /// Copies pasted or dropped files into `attachments/`, and puts a link to each copy at the
    /// cursor.
    fn paste_files(&mut self, files: Vec<PastedFile>, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let note = self.note.info.path.clone();
        let job = cx.background_spawn(async move {
            files
                .iter()
                .map(|file| match file {
                    PastedFile::Path(path) => core.attach_file(&note, path),
                    PastedFile::Image { extension, bytes } => {
                        core.paste_image(&note, extension, bytes)
                    }
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |pane, cx| {
            let copies = job.await;
            pane.update(cx, |pane, cx| {
                pane.insert_file_links(copies, true, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Opens the system file picker, and embeds each picked file at the cursor, with no copy.
    /// It shows as an image or a file card, like a pasted file.
    pub(crate) fn link_files(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(text::LINK_FILE.into()),
        });
        let core = self.core.clone();
        let note = self.note.info.path.clone();
        cx.spawn(async move |pane, cx| {
            let files = match picked.await {
                Ok(Ok(Some(files))) => files,
                Ok(Err(error)) => {
                    tracing::warn!(%error, "the file picker failed");
                    return;
                }
                _ => return,
            };
            let links = cx
                .background_spawn(async move {
                    files
                        .iter()
                        .map(|file| core.file_link(&note, file))
                        .collect::<Vec<_>>()
                })
                .await;
            pane.update(cx, |pane, cx| pane.insert_file_links(links, true, cx))
                .ok();
        })
        .detach();
    }

    /// Puts a link to each file at the cursor. `embedded` writes `![..](..)`, which shows an
    /// image or a file card. An error shows in a quiet bar.
    fn insert_file_links(
        &mut self,
        files: Vec<holm_core::Result<PathBuf>>,
        embedded: bool,
        cx: &mut Context<Self>,
    ) {
        for file in files {
            match file {
                Ok(path) => {
                    let name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let target = Target::File(path.to_string_lossy().into_owned());
                    self.editor.update(cx, |editor, cx| {
                        editor.insert_link(&name, &target, embedded, cx);
                    });
                }
                Err(error) => {
                    let error = AppError::from(error);
                    tracing::warn!(error = %crate::chain(&error), "cannot link a file");
                    self.error = Some(error.message());
                    cx.notify();
                }
            }
        }
    }

    /// Ctrl+K: reads what the link picker starts with. In a note link, the field starts with
    /// its target, and the pick changes that link.
    pub(crate) fn start_link(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<LinkStart>> {
        let link = self.editor.read(cx).link_at_cursor();
        self.replace_link = link.as_ref().is_some_and(|link| !link.embedded);
        let target = link.map(|link| link.target);
        let open_note = self.note.info.path.clone();
        self.run(
            move |core| {
                let mut notes = core.notes(Order::LastChange)?;
                notes.sort_by_key(|note| std::cmp::Reverse(note.modified));
                let query = match target {
                    Some(Target::Note { id, heading }) => match core.note_with_id(&id)? {
                        Some(note) => match heading {
                            Some(heading) => format!("{}#{heading}", note.title),
                            None => note.title,
                        },
                        None => String::new(),
                    },
                    Some(Target::Heading(heading)) => format!("#{heading}"),
                    _ => String::new(),
                };
                Ok(LinkStart {
                    notes,
                    open_note,
                    query,
                })
            },
            cx,
        )
    }

    /// Makes the link that the user picked in the link picker, and puts it at the cursor.
    /// A new note goes in the folder of the open note.
    pub(crate) fn link(
        &mut self,
        choice: LinkChoice,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<()>> {
        let folder = self
            .note
            .info
            .path
            .parent()
            .map(Path::to_owned)
            .unwrap_or_default();
        let make = self.run(
            move |core| match choice {
                LinkChoice::Note {
                    path,
                    title,
                    heading,
                } => {
                    let id = core.link_id(&path)?.to_string();
                    let text = heading.clone().unwrap_or(title);
                    Ok((Target::Note { id, heading }, text))
                }
                LinkChoice::SameNote(heading) => Ok((Target::Heading(heading.clone()), heading)),
                LinkChoice::Create(title) => {
                    let mut note = core.create(&folder)?;
                    note.body = format!("# {title}\n");
                    let note = core.save(&note)?;
                    let info = core.rename_to_title(&note.info.path)?;
                    let id = core.link_id(&info.path)?.to_string();
                    Ok((Target::Note { id, heading: None }, title))
                }
            },
            cx,
        );
        cx.spawn(async move |pane, cx| {
            let (target, text) = make.await?;
            pane.update(cx, |pane, cx| pane.put_link(&target, &text, cx))
                .ok();
            Ok(())
        })
    }

    /// Puts a link to `target` at the cursor, with the selection or `text` as its text.
    /// When the picker started in a link, it points that link to `target` and keeps its text.
    fn put_link(&mut self, target: &Target, text: &str, cx: &mut Context<Self>) {
        let replace = self.replace_link;
        self.editor.update(cx, |editor, cx| {
            if !(replace && editor.set_link_target(target, cx)) {
                editor.insert_link(text, target, false, cx);
            }
        });
    }

    fn show_fields(&mut self, cx: &mut Context<Self>) {
        let info = &self.note.info;
        self.tag_row
            .update(cx, |row, cx| row.show(&info.tags, info.pinned, cx));
    }

    fn tag_row_event(
        &mut self,
        _: &Entity<TagRow>,
        event: &TagRowEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TagRowEvent::Changed(tags) => {
                self.note.info.tags.clone_from(tags);
                let (path, tags) = (self.note.info.path.clone(), tags.clone());
                self.write_fields(move |core| core.set_tags(&path, &tags), cx);
            }
            TagRowEvent::Leave => window.focus(&self.editor.focus_handle(cx), cx),
        }
    }

    /// Opens the tag popup. The editor scrolls to it.
    pub(crate) fn focus_tags(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tag_row.update(cx, |row, cx| row.enter(window, cx));
    }

    /// Pins or unpins the open note.
    pub(crate) fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        let pinned = !self.note.info.pinned;
        self.note.info.pinned = pinned;
        self.show_fields(cx);
        let path = self.note.info.path.clone();
        self.write_fields(move |core| core.set_pinned(&path, pinned), cx);
    }

    /// Writes a change of the tags or the pin, after the writes before it.
    fn write_fields(
        &mut self,
        job: impl FnOnce(&Core) -> holm_core::Result<NoteInfo> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        self.fields_version += 1;
        let before = self.writing.take();
        self.writing = Some(cx.spawn(async move |pane, cx| {
            if let Some(before) = before {
                before.await;
            }
            let Ok(run) = pane.update(cx, |pane, cx| pane.run(job, cx)) else {
                return;
            };
            if let Err(error) = run.await {
                pane.update(cx, |pane, cx| {
                    let error = AppError::from(error);
                    tracing::warn!(error = %crate::chain(&error), "cannot change the tags or the pin");
                    pane.error = Some(error.message());
                    cx.notify();
                })
                .ok();
            }
        }));
    }

    /// Reads the tags and the pin of the open note again, after a file action.
    /// It keeps a change of them that came while it read.
    fn read_fields(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let core = self.core.clone();
        let path = self.note.info.path.clone();
        let version = self.fields_version;
        let job = cx.background_spawn(async move { core.read(&path) });
        cx.spawn(async move |pane, cx| {
            let Ok(note) = job.await else {
                return;
            };
            pane.update(cx, |pane, cx| {
                if pane.fields_version == version && pane.note.info.path == note.info.path {
                    pane.note.info = note.info;
                    pane.show_fields(cx);
                }
            })
            .ok();
        })
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

    /// The path of the open note, relative to the notes folder.
    pub(crate) fn path(&self) -> &Path {
        &self.note.info.path
    }

    /// Saves unsaved text now. Like at quit, the text wins over an outside change.
    /// The task ends when the file is written, or when the save failed.
    fn flush(&mut self, cx: &mut Context<Self>) -> Task<()> {
        self.autosave = None;
        let running = self.saving.take();
        cx.spawn(async move |pane, cx| {
            if let Some(running) = running {
                running.await;
            }
            let save = pane.update(cx, |pane, cx| {
                if pane.is_dirty(cx) {
                    pane.save_task(true, cx)
                } else {
                    None
                }
            });
            if let Ok(Some(save)) = save {
                save.await;
            }
        })
    }

    /// Opens the note at `path`. With [`Leave::Keep`], the pane first saves the open note and
    /// renames its file to match the title. When that save fails, the pane stays on the note.
    pub(crate) fn open(&mut self, path: PathBuf, leave: Leave, cx: &mut Context<Self>) {
        if leave == Leave::Keep && path == self.note.info.path {
            return;
        }
        let flush = (leave == Leave::Keep).then(|| self.flush(cx));
        let old = self.note.info.path.clone();
        self.opening = Some(cx.spawn(async move |pane, cx| {
            if let Some(flush) = flush {
                flush.await;
                let saved = pane.update(cx, |pane, cx| !pane.is_dirty(cx));
                if !matches!(saved, Ok(true)) {
                    return;
                }
            }
            let Ok(core) = pane.read_with(cx, |pane, _| pane.core.clone()) else {
                return;
            };
            let job = cx.background_spawn(async move {
                let renamed = (leave == Leave::Keep).then(|| core.rename_to_title(&old));
                (renamed, core.read(&path))
            });
            let (renamed, read) = job.await;
            pane.update(cx, |pane, cx| pane.opened(renamed, read, cx))
                .ok();
        }));
    }

    fn opened(
        &mut self,
        renamed: Option<holm_core::Result<holm_core::model::NoteInfo>>,
        read: holm_core::Result<Note>,
        cx: &mut Context<Self>,
    ) {
        self.opening = None;
        if let Some(Err(error)) = renamed {
            let error = AppError::from(error);
            tracing::warn!(error = %crate::chain(&error), "cannot rename the note to its title");
        }
        match read {
            Ok(note) => {
                let path = note.info.path.clone();
                self.show(note, false, cx);
                cx.emit(PaneEvent::Opened(path));
            }
            Err(error) => {
                let error = AppError::from(error);
                tracing::warn!(error = %crate::chain(&error), "cannot open the note");
                self.error = Some(error.message());
                cx.notify();
            }
        }
    }

    /// Runs a file action off the UI thread, after the pane saves the open note. So a late
    /// save never writes a file back after the action moved or deleted it. After the action,
    /// the pane reads the tags and the pin of the open note again, so its next save keeps them.
    pub(crate) fn run<T: Send + 'static>(
        &mut self,
        job: impl FnOnce(&Core) -> holm_core::Result<T> + Send + 'static,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<T>> {
        let flush = self.flush(cx);
        let core = self.core.clone();
        cx.spawn(async move |pane, cx| {
            flush.await;
            let result = cx.background_spawn(async move { job(&core) }).await;
            if let Ok(read) = pane.update(cx, Self::read_fields) {
                read.await;
            }
            result
        })
    }

    /// Like [`Self::run`], for an action that moves the note or folder at `from` and gives its
    /// new path. When that holds the open note, the pane follows it.
    pub(crate) fn run_move(
        &mut self,
        from: PathBuf,
        job: impl FnOnce(&Core) -> holm_core::Result<PathBuf> + Send + 'static,
        cx: &mut Context<Self>,
    ) -> Task<holm_core::Result<PathBuf>> {
        let run = self.run(job, cx);
        cx.spawn(async move |pane, cx| {
            let result = run.await;
            if let Ok(to) = &result {
                pane.update(cx, |pane, cx| pane.moved(&from, to, cx)).ok();
            }
            result
        })
    }

    /// Follows a move of the open note, or of a folder that holds it, and reads it again.
    /// A move can change the file links in the note.
    fn moved(&mut self, from: &Path, to: &Path, cx: &mut Context<Self>) {
        let Ok(rest) = self.note.info.path.strip_prefix(from) else {
            return;
        };
        let path = if rest.as_os_str().is_empty() {
            to.to_owned()
        } else {
            to.join(rest)
        };
        self.note.info.path.clone_from(&path);
        let folder = path.parent().map(|parent| self.notes_folder.join(parent));
        self.editor
            .update(cx, |editor, cx| editor.set_folder(folder, cx));
        self.reload(cx);
        cx.emit(PaneEvent::Opened(path));
    }

    /// Selects the title text in the editor and gives the editor the focus.
    pub(crate) fn select_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, Editor::select_title);
        window.focus(&self.editor.focus_handle(cx), cx);
    }
}

impl EventEmitter<PaneEvent> for NotePane {}

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

/// The editor target of a broken link from the core.
fn target(link: LinkTarget) -> Option<Target> {
    match link {
        LinkTarget::Note { id, heading } => Some(Target::Note {
            id: id.to_string(),
            heading,
        }),
        _ => None,
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
