//! The link picker: a small window that finds the target of a note link.
//!
//! It matches titles with fuzzy matching, and lists the headings of a note after `#`. Tab fills
//! in the title of the selected row and adds `#`. When no title is the typed text, the last row
//! creates a note. `Enter` picks a row, and `Esc` closes the window.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use gpui_kit::base::actions::Cancel;
use gpui_kit::component::command::{Command, CommandItem, CommandState};
use gpui_kit::component::input::IndentInline;
use gpui_kit::component::{ActiveTheme as _, IndexPath, h_flex};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement, Render, Role, SharedString, StatefulInteractiveElement as _, Styled,
    TestSupportExt as _, Window, div, px,
};
use holm_core::Core;
use holm_core::model::NoteInfo;

use crate::link_rows::{self, Query, Row};
use crate::{popup, text};

/// What the link picker starts with.
pub(crate) struct LinkStart {
    /// The notes to pick from, in their order.
    pub(crate) notes: Vec<NoteInfo>,
    /// The note that gets the link.
    pub(crate) open_note: PathBuf,
    /// The first text of the field.
    pub(crate) query: String,
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
    core: Core,
    notes: Vec<NoteInfo>,
    open_note: PathBuf,
    state: Entity<CommandState>,
    rows: Vec<Row>,
    /// The note of the heading rows, or `None` for the open note.
    heading_note: Option<usize>,
    /// The headings of each note that the field named, without the title.
    headings: HashMap<PathBuf, Vec<String>>,
    loading: HashSet<PathBuf>,
}

impl LinkPicker {
    /// The field gets the focus.
    pub(crate) fn new(
        core: Core,
        start: LinkStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let state = cx.new(|cx| CommandState::new(window, cx));
        state.update(cx, |state, cx| {
            state.focus(window, cx);
            state.set_query(start.query, window, cx);
        });
        let mut this = Self {
            core,
            notes: start.notes,
            open_note: start.open_note,
            state,
            rows: Vec::new(),
            heading_note: None,
            headings: HashMap::new(),
            loading: HashSet::new(),
        };
        this.query_changed(cx);
        this
    }

    fn pick(&mut self, at: IndexPath, cx: &mut Context<Self>) {
        if let Some(choice) = self.choice(at.row) {
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
        let text = self.state.read(cx).query(cx).to_string();
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
                    load_headings(self.core.clone(), path.clone(), cx);
                }
                self.headings.get(path).map_or(&[][..], Vec::as_slice)
            }
            None => &[],
        };
        self.rows = link_rows::rows(query, &self.notes, shown);
        cx.notify();
    }

    fn headings_loaded(&mut self, path: PathBuf, headings: Vec<String>, cx: &mut Context<Self>) {
        self.loading.remove(&path);
        self.headings.insert(path, headings);
        self.query_changed(cx);
    }

    /// Tab fills in the title of the selected note and adds `#`.
    fn fill_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let selected = self.state.read(cx).selected_index();
        let Some(Row::Note(at)) = selected.and_then(|at| self.rows.get(at.row)) else {
            return;
        };
        let query = format!("{}#", self.notes[*at].title);
        self.state
            .update(cx, |state, cx| state.set_query(query, window, cx));
        self.query_changed(cx);
    }

    fn items(&self, cx: &Context<Self>) -> Vec<CommandItem> {
        let muted = cx.theme().muted_foreground;
        self.rows
            .iter()
            .filter_map(|row| {
                let (label, folder): (SharedString, Option<SharedString>) = match row {
                    Row::Note(at) => {
                        let note = self.notes.get(*at)?;
                        (note.title.clone().into(), folder(&note.path))
                    }
                    Row::Heading(heading) => (heading.clone().into(), None),
                    Row::Create(title) => (text::create_note(title).into(), None),
                };
                let shown = label.clone();
                Some(CommandItem::new().label(label).child(move |_, _| {
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_2()
                        .child(div().truncate().child(shown.clone()))
                        .children(folder.clone().map(|folder| {
                            div().ml_auto().flex_none().text_color(muted).child(folder)
                        }))
                }))
            })
            .collect()
    }
}

/// The folder of a note for its row, or `None` in the root folder.
fn folder(path: &Path) -> Option<SharedString> {
    path.parent()
        .filter(|folder| !folder.as_os_str().is_empty())
        .map(|folder| folder.display().to_string().into())
}

fn load_headings(core: Core, path: PathBuf, cx: &mut Context<LinkPicker>) {
    let read = path.clone();
    let job = cx.background_spawn(async move { core.headings(read) });
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

impl EventEmitter<LinkPickerEvent> for LinkPicker {}

impl Render for LinkPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let command = Command::new(&self.state)
            .items(self.items(cx))
            .placeholder(text::LINK_TO_NOTE)
            .bordered(false)
            .filterable(false)
            .empty(|_, _, cx| popup::empty(text::NO_HEADING, cx))
            .on_confirm({
                let this = this.clone();
                move |at, _, cx| {
                    this.update(cx, |this, cx| this.pick(at, cx)).ok();
                }
            })
            .on_query(move |_, _, cx| {
                this.update(cx, LinkPicker::query_changed).ok();
            });
        popup::layer(cx)
            .id("link-picker")
            .role(Role::Dialog)
            .aria_label(text::LINK_TO_NOTE)
            .test_support()
            // The palette clears its text on the first `Esc`. The window closes at once.
            .capture_action(cx.listener(|_, _: &Cancel, _, cx| {
                cx.emit(LinkPickerEvent::Close);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                this.fill_title(window, cx);
            }))
            .child(popup::panel(px(480.), cx).child(command))
    }
}
