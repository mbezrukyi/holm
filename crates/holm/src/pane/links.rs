//! The links of the open note.
//!
//! It opens the links of the editor: a note, a heading in a note, a file in its system app, or
//! a web address. It copies pasted files into `attachments/`. After each change in the notes
//! folder, it reads the backlinks again and tells the editor which note links are broken.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, PathPromptOptions, Subscription, Task,
};
use holm_core::model::LinkTarget;
use holm_editor::{Editor, OpenFile, OpenLink, PasteFiles, PastedFile};
use holm_markdown::Target;

use crate::backlinks::Backlinks;
use crate::error::AppError;
use crate::notes_folder::{FolderEvent, NotesFolder};
use crate::text;

/// What the links tell the pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum LinksEvent {
    /// Open another note, at `heading` when it has one.
    Open {
        path: PathBuf,
        heading: Option<String>,
    },
    /// An answer for the message line.
    Error(&'static str),
}

/// What the system picker picks. One picker can't pick files and folders together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    Files,
    Folders,
}

pub(super) struct Links {
    notes_folder: Entity<NotesFolder>,
    editor: Entity<Editor>,
    backlinks: Entity<Backlinks>,
    /// The path of the open note, relative to the notes folder.
    note: Option<PathBuf>,
    reading: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Links {
    /// [`Self::show`] reads the links, and puts the backlinks in `backlinks`.
    pub(super) fn new(
        notes_folder: Entity<NotesFolder>,
        editor: Entity<Editor>,
        backlinks: Entity<Backlinks>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.subscribe(&notes_folder, |links, _, event: &FolderEvent, cx| {
                if !event.changes.is_empty() {
                    links.read(cx);
                }
            }),
            cx.subscribe(&editor, |links, _, event: &OpenLink, cx| {
                links.open_link(event.target.clone(), cx);
            }),
            cx.subscribe(&editor, |_, _, event: &OpenFile, cx| {
                cx.open_with_system(&event.path);
            }),
            cx.subscribe(&editor, |links, _, event: &PasteFiles, cx| {
                links.paste_files(event.files.clone(), cx);
            }),
        ];
        Self {
            notes_folder,
            editor,
            backlinks,
            note: None,
            reading: None,
            _subscriptions: subscriptions,
        }
    }

    /// Follows the open note to `note`, and reads its links. With no note, it reads nothing.
    pub(super) fn show(&mut self, note: Option<PathBuf>, cx: &mut Context<Self>) {
        self.note = note;
        self.read(cx);
    }

    /// Opens the system picker for `pick`, and embeds each picked path at the cursor, with no
    /// copy. It shows as an image, a file chip or a folder chip, like a pasted path.
    pub(super) fn pick(&mut self, pick: Pick, cx: &mut Context<Self>) {
        let Some(note) = self.note.clone() else {
            return;
        };
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: pick == Pick::Files,
            directories: pick == Pick::Folders,
            multiple: true,
            prompt: Some(text::LINK_FILE.into()),
        });
        cx.spawn(async move |links, cx| {
            let files = match picked.await {
                Ok(Ok(Some(files))) => files,
                Ok(Err(error)) => {
                    tracing::warn!(%error, "the file picker failed");
                    return;
                }
                _ => return,
            };
            let names = files.iter().map(|file| file_name(file)).collect();
            let Ok(paths) = links.update(cx, |links, cx| {
                links.notes_folder.read(cx).file_links(note, files, cx)
            }) else {
                return;
            };
            let paths = paths.await;
            links
                .update(cx, |links, cx| links.insert_file_links(names, paths, cx))
                .ok();
        })
        .detach();
    }

    /// Reads the backlinks and the broken note links of the open note, and shows them.
    fn read(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.note.clone() else {
            self.reading = None;
            self.backlinks
                .update(cx, |list, cx| list.show(Vec::new(), cx));
            return;
        };
        let notes_folder = self.notes_folder.read(cx);
        let backlinks = notes_folder.snapshot().backlinks(&path);
        let broken = notes_folder.broken_links(path.clone(), cx);
        let job = cx.background_spawn(async move {
            Ok::<_, holm_core::Error>((path, backlinks, broken.await?))
        });
        self.reading = Some(cx.spawn(async move |links, cx| {
            let result = job.await;
            links
                .update(cx, |links, cx| match result {
                    Ok((path, backlinks, broken)) if links.note.as_ref() == Some(&path) => {
                        links
                            .backlinks
                            .update(cx, |list, cx| list.show(backlinks, cx));
                        let broken: HashSet<Target> =
                            broken.into_iter().filter_map(target).collect();
                        links
                            .editor
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
        match self.notes_folder.read(cx).snapshot().note_with_id(&id) {
            Some(note) => self.open_at(note.path.clone(), heading, cx),
            None => cx.emit(LinksEvent::Error(text::NOTE_GONE)),
        }
    }

    /// Opens the note at `path` at `heading`, or at the top when the note has no such heading.
    fn open_at(&mut self, path: PathBuf, heading: Option<String>, cx: &mut Context<Self>) {
        if self.note.as_ref() != Some(&path) {
            cx.emit(LinksEvent::Open { path, heading });
        } else if let Some(heading) = heading {
            self.editor
                .update(cx, |editor, cx| editor.show_heading(&heading, cx));
        }
    }

    /// Copies pasted or dropped files into `attachments/`, and puts a link to each copy at the
    /// cursor.
    fn paste_files(&mut self, files: Vec<PastedFile>, cx: &mut Context<Self>) {
        let Some(note) = self.note.clone() else {
            return;
        };
        let names = files
            .iter()
            .map(|file| match file {
                PastedFile::Path(path) => file_name(path),
                PastedFile::Image { extension, .. } => {
                    format!("{}.{extension}", text::PASTED_IMAGE)
                }
            })
            .collect();
        let links = cx.weak_entity();
        self.notes_folder.update(cx, |notes_folder, cx| {
            notes_folder.attach(
                note,
                files,
                move |copies, cx| {
                    links
                        .update(cx, |links, cx| links.insert_file_links(names, copies, cx))
                        .ok();
                },
                cx,
            );
        });
    }

    /// Embeds a link to each file at the cursor, as `![name](..)`. It shows an image or a file
    /// chip. An error shows in the message line.
    fn insert_file_links(
        &mut self,
        names: Vec<String>,
        files: Vec<holm_core::Result<PathBuf>>,
        cx: &mut Context<Self>,
    ) {
        for (name, file) in names.into_iter().zip(files) {
            match file {
                Ok(path) => {
                    let target = Target::File(path.to_string_lossy().into_owned());
                    self.editor.update(cx, |editor, cx| {
                        editor.insert_link(&name, &target, true, cx);
                    });
                }
                Err(error) => {
                    let error = AppError::from(error);
                    tracing::warn!(error = %crate::chain(&error), "cannot link a file");
                    cx.emit(LinksEvent::Error(error.message()));
                }
            }
        }
    }
}

impl EventEmitter<LinksEvent> for Links {}

/// The file name of `path`, for the text of a link to it.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
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
