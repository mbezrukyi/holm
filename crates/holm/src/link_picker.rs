//! The link picker: a small window that finds the target of a note link.
//!
//! It matches titles with fuzzy matching, and lists the headings of a note after `#`. A `#` typed into a field with no
//! `#` yet fills in the title of the note under the cursor. When no title is the typed text, the last row creates a
//! note. `Enter` picks a row, and `Esc` closes the window.

use std::collections::{HashMap, HashSet};

use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, ParentElement, Render, Role,
    StatefulInteractiveElement as _, Subscription, TestSupportExt as _, Window, px,
};
use holm_core::model::{NoteId, NoteInfo, NoteSet};
use holm_core::{NotePath, Snapshot};
use holm_markdown::Target;

use crate::core_door::CoreDoor;
use crate::link_rows::{self, LinkRow, Query};
use crate::pick_notes::PickNotes;
use crate::popup::{self, PickEvent, PickList, PickRow};
use crate::text;

pub(crate) struct LinkStart {
    notes: Vec<NoteInfo>,
    open_note: NotePath,
    query: String,
}

impl LinkStart {
    /// The picker lists the notes of `snapshot` with the newest change first, as [`Snapshot::newest`] gives them. It
    /// does not take the note order of the sidebar: a link most often goes to a note of late. The link goes into
    /// `open_note`. The field starts with `target`, the link at the cursor, when it is a note or a heading.
    pub(crate) fn new(snapshot: &Snapshot, open_note: NotePath, target: Option<&Target>) -> Self {
        let query = match target {
            Some(Target::Note { id, heading }) => snapshot
                .note_with_id(&NoteId::from_field(id.clone()))
                .map(|note| Query::text(Some(&note.title), heading.as_deref()))
                .unwrap_or_default(),
            Some(Target::Heading(heading)) => Query::text(None, Some(heading)),
            _ => String::new(),
        };
        Self {
            notes: snapshot.newest(NoteSet::All),
            open_note,
            query,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LinkPickerEvent {
    Pick(LinkChoice),
    Close,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LinkChoice {
    Note {
        path: NotePath,
        title: String,
        heading: Option<String>,
    },
    /// A heading in the open note.
    SameNote(String),
    /// A new note with this title.
    Create(String),
}

pub(crate) struct LinkPicker {
    core_door: Entity<CoreDoor>,
    notes: PickNotes,
    open_note: NotePath,
    list: Entity<PickList<LinkChoice>>,
    /// The note of the heading rows, or `None` for the open note.
    heading_note: Option<usize>,
    /// The headings of each note that the field named.
    headings: HashMap<NotePath, Vec<String>>,
    loading: HashSet<NotePath>,
    /// The text of the field that the rows come from.
    text: String,
    _list: Subscription,
}

impl LinkPicker {
    /// The field gets the focus.
    pub(crate) fn new(
        core_door: Entity<CoreDoor>,
        start: LinkStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let list = cx.new(|cx| {
            let list = PickList::new(text::LINK_TO_NOTE, text::NO_HEADING, window, cx);
            list.set_query(start.query, window, cx);
            list
        });
        let subscription =
            cx.subscribe_in(
                &list,
                window,
                |this, _, event: &PickEvent<LinkChoice>, window, cx| match event {
                    PickEvent::Query => this.typed(window, cx),
                    PickEvent::Pick(choice) => cx.emit(LinkPickerEvent::Pick(choice.clone())),
                    PickEvent::Close => cx.emit(LinkPickerEvent::Close),
                },
            );
        let mut this = Self {
            core_door,
            notes: PickNotes::new(start.notes),
            open_note: start.open_note,
            list,
            heading_note: None,
            headings: HashMap::new(),
            loading: HashSet::new(),
            text: String::new(),
            _list: subscription,
        };
        this.query_changed(cx);
        this
    }

    fn choice(&self, row: &LinkRow) -> Option<LinkChoice> {
        let note = |at: usize| self.notes.get(at);
        Some(match row {
            LinkRow::Note(at) => {
                let note = note(*at)?;
                LinkChoice::Note {
                    path: note.path.clone(),
                    title: note.title.clone(),
                    heading: None,
                }
            }
            LinkRow::Heading(heading) => match self.heading_note {
                Some(at) => {
                    let note = note(at)?;
                    LinkChoice::Note {
                        path: note.path.clone(),
                        title: note.title.clone(),
                        heading: Some(heading.clone()),
                    }
                }
                None => LinkChoice::SameNote(heading.clone()),
            },
            LinkRow::Create(title) => LinkChoice::Create(title.clone()),
        })
    }

    /// It loads the headings of a note the first time.
    fn query_changed(&mut self, cx: &mut Context<Self>) {
        self.text = self.list.read(cx).query(cx).to_string();
        let query = Query::parse(&self.text, &self.notes);
        let headings = match query {
            Query::Titles(_) => None,
            Query::Headings { note, .. } => {
                self.heading_note = note;
                Some(match note {
                    Some(at) => self.notes[at].path.clone(),
                    None => self.open_note.clone(),
                })
            }
        };
        let shown = match &headings {
            Some(path) => {
                if !self.headings.contains_key(path) && self.loading.insert(path.clone()) {
                    self.load_headings(path.clone(), cx);
                }
                self.headings.get(path).map_or(&[][..], Vec::as_slice)
            }
            None => &[],
        };
        let shown = self.shown_rows(&link_rows::rows(query, &self.notes, shown));
        self.list.update(cx, |list, cx| list.set_rows(shown, cx));
    }

    fn headings_loaded(&mut self, path: NotePath, headings: Vec<String>, cx: &mut Context<Self>) {
        self.loading.remove(&path);
        self.headings.insert(path, headings);
        self.query_changed(cx);
    }

    /// A `#` typed into a text with no `#` yet fills in the title of the note under the cursor.
    fn typed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.list.read(cx).query(cx).to_string();
        let one_hash = text.matches('#').count() == 1 && !self.text.contains('#');
        if one_hash && text.replacen('#', "", 1) == self.text {
            let query = match self.list.read(cx).cursor() {
                Some(LinkChoice::Note {
                    title, heading: None, ..
                }) => Some(Query::text(Some(title), Some(""))),
                _ => None,
            };
            if let Some(query) = query {
                self.list.update(cx, |list, cx| list.set_query(query, window, cx));
            }
        }
        self.query_changed(cx);
    }

    fn shown_rows(&self, rows: &[LinkRow]) -> Vec<(PickRow, LinkChoice)> {
        rows.iter()
            .filter_map(|row| {
                let shown = match row {
                    LinkRow::Note(at) => PickRow::note(self.notes.get(*at)?),
                    LinkRow::Heading(heading) => PickRow::new(heading.clone()),
                    LinkRow::Create(title) => PickRow::new(text::create_note(title)),
                };
                Some((shown, self.choice(row)?))
            })
            .collect()
    }

    fn load_headings(&self, path: NotePath, cx: &mut Context<Self>) {
        let job = self.core_door.read(cx).headings(path.clone(), cx);
        cx.spawn(async move |this, cx| {
            let headings = job.await;
            this.update(cx, |this, cx| this.headings_loaded(path, headings, cx))
                .ok();
        })
        .detach();
    }
}

impl EventEmitter<LinkPickerEvent> for LinkPicker {}

impl Render for LinkPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        popup::layer(cx)
            .id("link-picker")
            .role(Role::Dialog)
            .aria_label(text::LINK_TO_NOTE)
            .test_support()
            .child(
                popup::panel(px(480.), cx)
                    .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(LinkPickerEvent::Close)))
                    .child(self.list.clone()),
            )
    }
}
