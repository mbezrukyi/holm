//! Quick open and the link picker: one small window that finds a note by its title.
//!
//! Quick open matches every title that holds the typed text. `↑` and `↓` pick a note,
//! `Enter` opens it, and `Esc` closes the window.
//!
//! The link picker matches titles with fuzzy matching, and lists the headings of a note after
//! `#`. Tab fills in the title of the selected row and adds `#`. When no title is the typed
//! text, the last row creates a note.

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

/// The answer of the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum QuickOpenEvent {
    Open(PathBuf),
    Link(LinkChoice),
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

/// The link picker part.
struct Linking {
    core: Core,
    open_note: PathBuf,
    rows: Vec<Row>,
    /// The note of the heading rows, or `None` for the open note.
    heading_note: Option<usize>,
    /// The headings of each note that the field named, without the title.
    headings: HashMap<PathBuf, Vec<String>>,
    loading: HashSet<PathBuf>,
}

pub(crate) struct QuickOpen {
    notes: Vec<NoteInfo>,
    state: Entity<CommandState>,
    linking: Option<Linking>,
}

impl QuickOpen {
    /// Quick open over `notes`, in their order. The field gets the focus.
    pub(crate) fn new(notes: Vec<NoteInfo>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|cx| CommandState::new(window, cx));
        state.update(cx, |state, cx| state.focus(window, cx));
        Self {
            notes,
            state,
            linking: None,
        }
    }

    /// The link picker over `notes`, in their order, for a link in `open_note`. The field starts
    /// with `query`.
    pub(crate) fn link(
        core: Core,
        notes: Vec<NoteInfo>,
        open_note: PathBuf,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::new(notes, window, cx);
        this.linking = Some(Linking {
            core,
            open_note,
            rows: Vec::new(),
            heading_note: None,
            headings: HashMap::new(),
            loading: HashSet::new(),
        });
        let query = query.to_owned();
        this.state
            .update(cx, |state, cx| state.set_query(query, window, cx));
        this.query_changed(cx);
        this
    }

    fn open(&mut self, at: IndexPath, cx: &mut Context<Self>) {
        if self.linking.is_some() {
            if let Some(choice) = self.choice(at.row) {
                cx.emit(QuickOpenEvent::Link(choice));
            }
        } else if let Some(note) = self.notes.get(at.row) {
            cx.emit(QuickOpenEvent::Open(note.path.clone()));
        }
    }

    fn choice(&self, row: usize) -> Option<LinkChoice> {
        let linking = self.linking.as_ref()?;
        let note = |at: usize| self.notes.get(at);
        Some(match linking.rows.get(row)? {
            Row::Note(at) => {
                let note = note(*at)?;
                LinkChoice::Note {
                    path: note.path.clone(),
                    title: note.title.clone(),
                    heading: None,
                }
            }
            Row::Heading(heading) => match linking.heading_note {
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

    fn query_text(&self, cx: &Context<Self>) -> String {
        self.state.read(cx).query(cx).to_string()
    }

    /// Finds the rows for the new text. It loads the headings of a note the first time.
    fn query_changed(&mut self, cx: &mut Context<Self>) {
        let text = self.query_text(cx);
        let Some(linking) = &mut self.linking else {
            return;
        };
        let query = Query::parse(&text, &self.notes);
        let headings = match query {
            Query::Titles(_) => None,
            Query::Headings { note, .. } => {
                linking.heading_note = note;
                Some(match note {
                    Some(at) => self.notes[at].path.clone(),
                    None => linking.open_note.clone(),
                })
            }
        };
        let shown = match &headings {
            Some(path) => {
                if !linking.headings.contains_key(path) && linking.loading.insert(path.clone()) {
                    load_headings(linking.core.clone(), path.clone(), cx);
                }
                linking.headings.get(path).map_or(&[][..], Vec::as_slice)
            }
            None => &[],
        };
        linking.rows = link_rows::rows(query, &self.notes, shown);
        cx.notify();
    }

    fn headings_loaded(&mut self, path: PathBuf, headings: Vec<String>, cx: &mut Context<Self>) {
        if let Some(linking) = &mut self.linking {
            linking.loading.remove(&path);
            linking.headings.insert(path, headings);
            self.query_changed(cx);
        }
    }

    /// Tab fills in the title of the selected note and adds `#`.
    fn fill_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(linking) = &self.linking else {
            return;
        };
        cx.stop_propagation();
        let selected = self.state.read(cx).selected_index();
        let Some(Row::Note(at)) = selected.and_then(|at| linking.rows.get(at.row)) else {
            return;
        };
        let query = format!("{}#", self.notes[*at].title);
        self.state
            .update(cx, |state, cx| state.set_query(query, window, cx));
        self.query_changed(cx);
    }

    fn items(&self, cx: &Context<Self>) -> Vec<CommandItem> {
        let Some(linking) = &self.linking else {
            return self
                .notes
                .iter()
                .map(|note| CommandItem::new().label(note.title.clone()))
                .collect();
        };
        let muted = cx.theme().muted_foreground;
        linking
            .rows
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

fn load_headings(core: Core, path: PathBuf, cx: &mut Context<QuickOpen>) {
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

impl EventEmitter<QuickOpenEvent> for QuickOpen {}

impl Render for QuickOpen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let linking = self.linking.is_some();
        let (name, empty) = if linking {
            (text::LINK_TO_NOTE, text::NO_HEADING)
        } else {
            (text::QUICK_OPEN, text::NO_MATCH)
        };
        let mut command = Command::new(&self.state)
            .items(self.items(cx))
            .placeholder(name)
            .bordered(false)
            .empty(move |_, _, cx| {
                div()
                    .px_3()
                    .py_2()
                    .text_color(cx.theme().muted_foreground)
                    .child(empty)
            })
            .on_confirm({
                let this = this.clone();
                move |at, _, cx| {
                    this.update(cx, |this, cx| this.open(at, cx)).ok();
                }
            });
        if linking {
            command = command.filterable(false).on_query(move |_, _, cx| {
                this.update(cx, QuickOpen::query_changed).ok();
            });
        }
        popup::layer(cx)
            .id("quick-open")
            .role(Role::Dialog)
            .aria_label(name)
            .test_support()
            // The palette clears its text on the first `Esc`. The window closes at once.
            .capture_action(cx.listener(|_, _: &Cancel, _, cx| {
                cx.emit(QuickOpenEvent::Close);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                this.fill_title(window, cx);
            }))
            .child(popup::panel(px(480.), cx).child(command))
    }
}
