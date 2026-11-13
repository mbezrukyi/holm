//! Quick open: a small window that finds a note by its title.
//!
//! It matches every title that holds the typed text, and shows the first [`MAX_ROWS`] of them.
//! Each row shows the folder of its note on the right.
//! `↑` and `↓` pick a note, `Enter` opens it, and `Esc` closes the window.

use std::path::PathBuf;

use gpui_kit::base::actions::Cancel;
use gpui_kit::component::IndexPath;
use gpui_kit::component::command::{Command, CommandState};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement, Render, Role, StatefulInteractiveElement as _, TestSupportExt as _, Window, px,
};
use holm_core::model::NoteInfo;

use crate::{popup, text};

/// The answer of the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum QuickOpenEvent {
    Open(PathBuf),
    Close,
}

/// The most rows the window shows. The list lays out each row it has, so more rows cost time
/// with each key.
const MAX_ROWS: usize = 50;

pub(crate) struct QuickOpen {
    notes: Vec<NoteInfo>,
    state: Entity<CommandState>,
    /// The place in `notes` of each row.
    rows: Vec<usize>,
}

impl QuickOpen {
    /// Quick open over `notes`, in their order. The field gets the focus.
    pub(crate) fn new(notes: Vec<NoteInfo>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|cx| CommandState::new(window, cx));
        state.update(cx, |state, cx| state.focus(window, cx));
        let rows = rows("", &notes);
        Self { notes, state, rows }
    }

    fn open(&mut self, at: IndexPath, cx: &mut Context<Self>) {
        if let Some(note) = self.rows.get(at.row).and_then(|at| self.notes.get(*at)) {
            cx.emit(QuickOpenEvent::Open(note.path.clone()));
        }
    }

    fn query_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.state.read(cx).query(cx);
        self.rows = rows(&query, &self.notes);
        cx.notify();
    }
}

/// The place in `notes` of each title that holds `query`, with no regard to capital letters.
/// Only the first [`MAX_ROWS`] show.
fn rows(query: &str, notes: &[NoteInfo]) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    notes
        .iter()
        .enumerate()
        .filter(|(_, note)| note.title.to_lowercase().contains(&query))
        .map(|(at, _)| at)
        .take(MAX_ROWS)
        .collect()
}

impl EventEmitter<QuickOpenEvent> for QuickOpen {}

impl Render for QuickOpen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let items = self
            .rows
            .iter()
            .filter_map(|at| self.notes.get(*at))
            .map(|note| popup::note_item(note, cx));
        let command = Command::new(&self.state)
            .items(items.collect::<Vec<_>>())
            .placeholder(text::QUICK_OPEN)
            .bordered(false)
            .filterable(false)
            .empty(|_, _, cx| popup::empty(text::NO_MATCH, cx))
            .on_confirm({
                let this = this.clone();
                move |at, _, cx| {
                    this.update(cx, |this, cx| this.open(at, cx)).ok();
                }
            })
            .on_query(move |_, _, cx| {
                this.update(cx, QuickOpen::query_changed).ok();
            });
        popup::layer(cx)
            .id("quick-open")
            .role(Role::Dialog)
            .aria_label(text::QUICK_OPEN)
            .test_support()
            // The palette clears its text on the first `Esc`. The window closes at once.
            .capture_action(cx.listener(|_, _: &Cancel, _, cx| {
                cx.emit(QuickOpenEvent::Close);
                cx.stop_propagation();
            }))
            .child(
                popup::panel(px(480.), cx)
                    .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(QuickOpenEvent::Close)))
                    .child(command),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::SystemTime;

    use holm_core::model::NoteInfo;

    use super::{MAX_ROWS, rows};

    fn notes(titles: &[String]) -> Vec<NoteInfo> {
        titles
            .iter()
            .map(|title| NoteInfo {
                path: PathBuf::from(format!("{title}.md")),
                id: None,
                title: title.clone(),
                preview: String::new(),
                tags: Vec::new(),
                pinned: false,
                modified: SystemTime::UNIX_EPOCH,
            })
            .collect()
    }

    #[test]
    fn titles_match_with_no_regard_to_case() {
        let list = notes(&["Plan".into(), "Goals".into(), "Top".into()]);
        assert_eq!(rows("", &list), [0, 1, 2]);
        assert_eq!(rows(" O ", &list), [1, 2]);
        assert_eq!(rows("milk", &list), Vec::<usize>::new());
    }

    #[test]
    fn only_the_first_titles_show() {
        let many: Vec<String> = (0..MAX_ROWS + 10).map(|n| format!("Note {n}")).collect();
        let list = notes(&many);
        assert_eq!(rows("", &list), (0..MAX_ROWS).collect::<Vec<_>>());
        assert_eq!(
            rows("note 5", &list),
            [5, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59]
        );
    }
}
