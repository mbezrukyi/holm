//! The links of the open note.
//!
//! It opens the links of the editor: a note, a heading in a note, a file in its system app, or
//! a web address. It copies pasted files into `attachments/`. After each change in the notes
//! folder, it reads the backlinks again and tells the editor which note links are broken.

use std::collections::HashSet;
use std::path::PathBuf;

use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, PathPromptOptions, Subscription, Task,
};
use holm_core::model::{LinkTarget, Order};
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
    /// A message for the quiet bar.
    Error(&'static str),
}

pub(super) struct Links {
    notes_folder: Entity<NotesFolder>,
    editor: Entity<Editor>,
    backlinks: Entity<Backlinks>,
    /// The path of the open note, relative to the notes folder.
    note: Option<PathBuf>,
    /// The revision of the snapshot that the links come from.
    revision: u64,
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
            cx.subscribe(&notes_folder, |links, notes_folder, event, cx| {
                let revision = notes_folder.read(cx).snapshot().revision();
                if matches!(event, FolderEvent::Changed(_)) && revision != links.revision {
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
            revision: 0,
            reading: None,
            _subscriptions: subscriptions,
        }
    }

    /// Follows the open note to `note`, and reads its links. With no note, it reads nothing.
    pub(super) fn show(&mut self, note: Option<PathBuf>, cx: &mut Context<Self>) {
        self.note = note;
        self.read(cx);
    }

    /// Opens the system file picker, and embeds each picked file at the cursor, with no copy.
    /// It shows as an image or a file chip, like a pasted file.
    pub(super) fn pick_files(&mut self, cx: &mut Context<Self>) {
        let Some(note) = self.note.clone() else {
            return;
        };
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
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
            let Ok(paths) = links.update(cx, |links, cx| {
                links.notes_folder.read(cx).file_links(note, files, cx)
            }) else {
                return;
            };
            let paths = paths.await;
            links
                .update(cx, |links, cx| links.insert_file_links(paths, cx))
                .ok();
        })
        .detach();
    }

    /// Reads the backlinks and the broken note links of the open note, and shows them.
    fn read(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.note.clone() else {
            return;
        };
        let notes_folder = self.notes_folder.read(cx);
        self.revision = notes_folder.snapshot().revision();
        let backlinks = notes_folder.snapshot().backlinks(&path, Order::LastChange);
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
        let links = cx.weak_entity();
        self.notes_folder.update(cx, |notes_folder, cx| {
            notes_folder.attach(
                note,
                files,
                move |copies, cx| {
                    links
                        .update(cx, |links, cx| links.insert_file_links(copies, cx))
                        .ok();
                },
                cx,
            );
        });
    }

    /// Embeds a link to each file at the cursor, as `![..](..)`. It shows an image or a file
    /// chip. An error shows in a quiet bar.
    fn insert_file_links(
        &mut self,
        files: Vec<holm_core::Result<PathBuf>>,
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
