//! The links of the open note.
//!
//! It opens the links of the editor: a note, a heading in a note, a file in its system app, or a web address. It copies
//! pasted files into `attachments/`. After each change in the notes folder, it reads the backlinks again and tells the
//! editor which note links are broken.
//!
//! It also runs the link picker flow. Ctrl+K gives the start of the picker, and the pick puts a link at the cursor.

use std::collections::HashSet;
use std::path::Path;

use gpui_kit::{App, Context, Entity, EventEmitter, PathPromptOptions, Subscription, Task};
use holm_core::model::{LinkTarget, NoteId};
use holm_core::{LinkPath, NotePath};
use holm_editor::{Editor, OpenFile, OpenLink, PasteFiles, PastedFile};
use holm_markdown::{LinkKind, Target};

use crate::backlinks::Backlinks;
use crate::core_door::{CoreDoor, Done, DoorEvent, Then};
use crate::link_picker::{LinkChoice, LinkStart};
use crate::text;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum LinksEvent {
    /// Open another note, at `heading` when it has one.
    Open { path: NotePath, heading: Option<String> },
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
    core_door: Entity<CoreDoor>,
    editor: Entity<Editor>,
    backlinks: Entity<Backlinks>,
    open_note: Option<NotePath>,
    reading: Option<Task<()>>,
    /// Whether the pick of the link picker changes the link at the cursor, not inserts a new one.
    replace: bool,
    _subscriptions: Vec<Subscription>,
}

impl Links {
    /// [`Self::show`] reads the links, and puts the backlinks in `backlinks`.
    pub(super) fn new(
        core_door: Entity<CoreDoor>,
        editor: Entity<Editor>,
        backlinks: Entity<Backlinks>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.subscribe(&core_door, |links, _, event: &DoorEvent, cx| {
                if !event.is_empty() {
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
            core_door,
            editor,
            backlinks,
            open_note: None,
            reading: None,
            replace: false,
            _subscriptions: subscriptions,
        }
    }

    /// Follows the open note to `note`, and reads its links. With no note, it reads nothing.
    pub(super) fn show(&mut self, note: Option<NotePath>, cx: &mut Context<Self>) {
        self.open_note = note;
        self.read(cx);
    }

    /// Opens the system picker for `pick`, and embeds each picked path at the cursor, with no copy. It shows as an
    /// image, a file chip or a folder chip, like a pasted path.
    pub(super) fn pick(&mut self, pick: Pick, cx: &mut Context<Self>) {
        let Some(note) = self.open_note.clone() else {
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
                links
                    .core_door
                    .update(cx, |core_door, cx| core_door.file_links(note, files, cx))
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

    /// Gives `done` what the link picker starts with, after the write line writes the unsaved text. In a note link, the
    /// field starts with its target, and the pick changes that link. With no open note, it does nothing.
    pub(super) fn start(&mut self, done: impl FnOnce(LinkStart, &mut App) + 'static, cx: &mut Context<Self>) {
        let Some(open_note) = self.open_note.clone() else {
            return;
        };
        let link = self.editor.read(cx).link_at_cursor();
        self.replace = link.as_ref().is_some_and(|link| link.kind == LinkKind::Plain);
        let target = link.map(|link| link.target);
        let links = cx.weak_entity();
        let done = Done::value(move |(), cx| {
            let start = links.read_with(cx, |links, cx| {
                LinkStart::new(links.core_door.read(cx).snapshot(), open_note, target.as_ref())
            });
            if let Ok(start) = start {
                done(start, cx);
            }
        });
        self.core_door.update(cx, |core_door, cx| core_door.wait(done, cx));
    }

    /// Makes the link of `choice`, and puts it at the cursor. A new note goes in the folder of the open note. The link
    /// goes only into the note that was open at the call. `done` runs at the end, also with no link.
    pub(super) fn link(&mut self, choice: LinkChoice, done: Then<()>, cx: &mut Context<Self>) {
        let Some(note) = self.open_note.clone() else {
            cx.defer(move |cx| done((), cx));
            return;
        };
        let folder = note.folder().clone();
        let links = cx.weak_entity();
        let into = note.clone();
        let put = move |target: Target, text: String, cx: &mut App| {
            links.update(cx, |links, cx| links.put(&into, &target, &text, cx)).ok();
        };
        match choice {
            LinkChoice::Note { path, title, heading } => self.core_door.update(cx, |core_door, cx| {
                core_door.link_id(
                    path,
                    Done::value(move |id, cx| {
                        let text = heading.clone().unwrap_or(title);
                        put(Target::Note { id, heading }, text, cx);
                        done((), cx);
                    }),
                    cx,
                );
            }),
            LinkChoice::SameNote(heading) => {
                self.put(&note, &Target::Heading(heading.clone()), &heading, cx);
                cx.defer(move |cx| done((), cx));
            }
            LinkChoice::Create(title) => self.core_door.update(cx, |core_door, cx| {
                core_door.create_linked_note(
                    folder,
                    title.clone(),
                    Done::value(move |id, cx| {
                        put(Target::Note { id, heading: None }, title, cx);
                        done((), cx);
                    }),
                    cx,
                );
            }),
        }
    }

    /// Puts a link to `target` at the cursor, when `note` is still the open note.
    fn put(&mut self, note: &NotePath, target: &Target, text: &str, cx: &mut Context<Self>) {
        if self.open_note.as_ref() != Some(note) {
            return;
        }
        let replace = self.replace;
        self.editor.update(cx, |editor, cx| {
            if !(replace && editor.set_link_target(target, cx)) {
                editor.insert_link(text, target, LinkKind::Plain, cx);
            }
        });
    }

    /// Reads the backlinks and the broken note links of the open note, and shows them.
    fn read(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.open_note.clone() else {
            self.reading = None;
            self.backlinks.update(cx, |list, cx| list.show(Vec::new(), cx));
            return;
        };
        let core_door = self.core_door.read(cx);
        let backlinks = core_door.snapshot().backlinks(&path);
        let broken = core_door.broken_links(path.clone(), cx);
        self.reading = Some(cx.spawn(async move |links, cx| {
            let broken = broken.await;
            links
                .update(cx, |links, cx| {
                    if links.open_note.as_ref() != Some(&path) {
                        return;
                    }
                    links.backlinks.update(cx, |list, cx| list.show(backlinks, cx));
                    let broken: HashSet<Target> = broken.into_iter().filter_map(target).collect();
                    links.editor.update(cx, |editor, cx| editor.set_link_states(broken, cx));
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
        match self.core_door.read(cx).snapshot().note_with_id(&NoteId::from_field(id)) {
            Some(note) => self.open_at(note.path.clone(), heading, cx),
            None => cx.emit(LinksEvent::Error(text::NOTE_GONE)),
        }
    }

    /// Opens the note at `path` at `heading`, or at the top when the note has no such heading.
    fn open_at(&mut self, path: NotePath, heading: Option<String>, cx: &mut Context<Self>) {
        if self.open_note.as_ref() != Some(&path) {
            cx.emit(LinksEvent::Open { path, heading });
        } else if let Some(heading) = heading {
            self.editor.update(cx, |editor, cx| editor.show_heading(&heading, cx));
        }
    }

    /// Copies pasted or dropped files into `attachments/`, and puts a link to each copy at the cursor.
    fn paste_files(&mut self, files: Vec<PastedFile>, cx: &mut Context<Self>) {
        let Some(note) = self.open_note.clone() else {
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
        self.core_door.update(cx, |core_door, cx| {
            core_door.attach(
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

    /// Embeds a link to each file at the cursor, as `![name](..)`. It shows an image or a file chip. A file with no
    /// link gets none.
    fn insert_file_links(&mut self, names: Vec<String>, files: Vec<Option<LinkPath>>, cx: &mut Context<Self>) {
        for (name, path) in names.into_iter().zip(files) {
            let Some(path) = path else {
                continue;
            };
            let target = Target::File(path.as_ref().to_string_lossy().into_owned());
            self.editor.update(cx, |editor, cx| {
                editor.insert_link(&name, &target, LinkKind::Embedded, cx);
            });
        }
    }
}

impl EventEmitter<LinksEvent> for Links {}

/// For the text of a link to it.
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
