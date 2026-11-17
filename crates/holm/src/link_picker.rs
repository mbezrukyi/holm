//! The link picker: a small window that finds the target of a note link.
//!
//! It matches titles with fuzzy matching, and lists the headings of a note after `#`. Tab fills
//! in the title of the selected row and adds `#`. When no title is the typed text, the last row
//! creates a note. `Enter` picks a row, and `Esc` closes the window.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use gpui_kit::component::input::IndentInline;
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement, Render, Role, StatefulInteractiveElement as _, Subscription,
    TestSupportExt as _, Window, px,
};
use holm_core::Snapshot;
use holm_core::model::{NoteInfo, NoteSet};

use crate::link_rows::{self, Query, Row};
use crate::notes_folder::NotesFolder;
use crate::pick_notes::PickNotes;
use crate::popup::{self, PickEvent, PickList, PickRow};
use crate::text;

/// What the link picker starts with.
pub(crate) struct LinkStart {
    notes: Vec<NoteInfo>,
    open_note: PathBuf,
    query: String,
}

impl LinkStart {
    /// The picker lists each note of `snapshot`, the newest change first. The path breaks a tie.
    /// The link goes into `open_note`. The field starts with `query`.
    pub(crate) fn new(snapshot: &Snapshot, open_note: PathBuf, query: String) -> Self {
        let mut notes: Vec<&NoteInfo> = snapshot.matching(NoteSet::All).collect();
        notes.sort_unstable_by(|a, b| {
            b.modified
                .cmp(&a.modified)
                .then_with(|| a.path.cmp(&b.path))
        });
        Self {
            notes: notes.into_iter().cloned().collect(),
            open_note,
            query,
        }
    }
}

/// The answer of the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LinkPickerEvent {
    Pick(LinkChoice),
    Close,
}

/// The pick of the link picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LinkChoice {
    /// A note, or a heading in it.
    Note {
        path: PathBuf,
        title: String,
        heading: Option<String>,
    },
    /// A heading in the open note.
    SameNote(String),
    /// A new note with this title.
    Create(String),
}

pub(crate) struct LinkPicker {
    notes_folder: Entity<NotesFolder>,
    notes: PickNotes,
    open_note: PathBuf,
    list: Entity<PickList>,
    rows: Vec<Row>,
    /// The note of the heading rows, or `None` for the open note.
    heading_note: Option<usize>,
    /// The headings of each note that the field named.
    headings: HashMap<PathBuf, Vec<String>>,
    loading: HashSet<PathBuf>,
    _list: Subscription,
}

impl LinkPicker {
    /// The field gets the focus.
    pub(crate) fn new(
        notes_folder: Entity<NotesFolder>,
        start: LinkStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let list = cx.new(|cx| {
            let list = PickList::new(text::LINK_TO_NOTE, text::NO_HEADING, window, cx);
            list.set_query(start.query, window, cx);
            list
        });
        let subscription = cx.subscribe(&list, |this, _, event: &PickEvent, cx| match event {
            PickEvent::Query => this.query_changed(cx),
            PickEvent::Pick(at) => this.pick(*at, cx),
            PickEvent::Close => cx.emit(LinkPickerEvent::Close),
        });
        let mut this = Self {
            notes_folder,
            notes: PickNotes::new(start.notes),
            open_note: start.open_note,
            list,
            rows: Vec::new(),
            heading_note: None,
            headings: HashMap::new(),
            loading: HashSet::new(),
            _list: subscription,
        };
        this.query_changed(cx);
        this
    }

    fn pick(&mut self, at: usize, cx: &mut Context<Self>) {
        if let Some(choice) = self.choice(at) {
            cx.emit(LinkPickerEvent::Pick(choice));
        }
    }

    fn choice(&self, row: usize) -> Option<LinkChoice> {
        let note = |at: usize| self.notes.get(at);
        Some(match self.rows.get(row)? {
            Row::Note(at) => {
                let note = note(*at)?;
                LinkChoice::Note {
                    path: note.path.clone(),
                    title: note.title.clone(),
                    heading: None,
                }
            }
            Row::Heading(heading) => match self.heading_note {
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
            Row::Create(title) => LinkChoice::Create(title.clone()),
        })
    }

    /// Finds the rows for the new text. It loads the headings of a note the first time.
    fn query_changed(&mut self, cx: &mut Context<Self>) {
        let text = self.list.read(cx).query(cx).to_string();
        let query = Query::parse(&text, &self.notes);
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
        self.rows = link_rows::rows(query, &self.notes, shown);
        let shown = self.shown_rows();
        self.list.update(cx, |list, cx| list.set_rows(shown, cx));
    }

    fn headings_loaded(&mut self, path: PathBuf, headings: Vec<String>, cx: &mut Context<Self>) {
        self.loading.remove(&path);
        self.headings.insert(path, headings);
        self.query_changed(cx);
    }

    /// Tab fills in the title of the selected note and adds `#`.
    fn fill_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let selected = self.list.read(cx).selected();
        let Some(Row::Note(at)) = selected.and_then(|at| self.rows.get(at)) else {
            return;
        };
        let query = format!("{}#", self.notes[*at].title);
        self.list
            .update(cx, |list, cx| list.set_query(query, window, cx));
        self.query_changed(cx);
    }

    fn shown_rows(&self) -> Vec<PickRow> {
        self.rows
            .iter()
            .filter_map(|row| {
                Some(match row {
                    Row::Note(at) => PickRow::note(self.notes.get(*at)?),
                    Row::Heading(heading) => PickRow::new(heading.clone()),
                    Row::Create(title) => PickRow::new(text::create_note(title)),
                })
            })
            .collect()
    }

    fn load_headings(&self, path: PathBuf, cx: &mut Context<Self>) {
        let job = self.notes_folder.read(cx).headings(path.clone(), cx);
        cx.spawn(async move |this, cx| {
            let headings = job.await.unwrap_or_else(|error| {
                let error = crate::error::AppError::from(error);
                tracing::warn!(error = %crate::chain(&error), "cannot read the headings of a note");
                Vec::new()
            });
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
            .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                this.fill_title(window, cx);
            }))
            .child(
                popup::panel(px(480.), cx)
                    .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(LinkPickerEvent::Close)))
                    .child(self.list.clone()),
            )
    }
}
