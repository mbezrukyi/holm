//! Quick open: a small window that finds a note by its title.
//!
//! It matches every title that holds the typed text, and shows the first [`MAX_ROWS`] of them.
//! Each row shows the folder of its note on the right.
//! `↑` and `↓` pick a note, `Enter` opens it, and `Esc` closes the window.

use std::path::PathBuf;

use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement, Render, Role, StatefulInteractiveElement as _, Subscription,
    TestSupportExt as _, Window, px,
};
use holm_core::model::NoteInfo;

use crate::pick_notes::{Lowercase, PickNotes};
use crate::popup::{self, PickEvent, PickList, PickRow};
use crate::text;

/// The answer of the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum QuickOpenEvent {
    Open(PathBuf),
    Close,
}

/// The most rows the window shows.
const MAX_ROWS: usize = 50;

pub(crate) struct QuickOpen {
    notes: PickNotes,
    list: Entity<PickList>,
    /// The place in `notes` of each row.
    rows: Vec<usize>,
    _list: Subscription,
}

impl QuickOpen {
    /// Quick open over `notes`, in their order. The field gets the focus.
    pub(crate) fn new(notes: Vec<NoteInfo>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let list = cx.new(|cx| PickList::new(text::QUICK_OPEN, text::NO_MATCH, window, cx));
        let subscription = cx.subscribe(&list, |this, _, event: &PickEvent, cx| match event {
            PickEvent::Query => this.query_changed(cx),
            PickEvent::Pick(at) => this.open(*at, cx),
            PickEvent::Close => cx.emit(QuickOpenEvent::Close),
        });
        let mut this = Self {
            notes: PickNotes::new(notes),
            list,
            rows: Vec::new(),
            _list: subscription,
        };
        this.query_changed(cx);
        this
    }

    fn open(&mut self, at: usize, cx: &mut Context<Self>) {
        if let Some(note) = self.rows.get(at).and_then(|at| self.notes.get(*at)) {
            cx.emit(QuickOpenEvent::Open(note.path.clone()));
        }
    }

    fn query_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.list.read(cx).query(cx);
        self.rows = rows(&query, &self.notes);
        let shown = self
            .rows
            .iter()
            .filter_map(|at| self.notes.get(*at))
            .map(PickRow::note)
            .collect();
        self.list.update(cx, |list, cx| list.set_rows(shown, cx));
    }
}

/// The place of each title that holds `query`, with no regard to capital letters. Only the
/// first [`MAX_ROWS`] show.
fn rows(query: &str, notes: &PickNotes) -> Vec<usize> {
    let query = Lowercase::new(query.trim());
    notes
        .titles()
        .enumerate()
        .filter(|(_, title)| title.as_str().contains(query.as_str()))
        .map(|(at, _)| at)
        .take(MAX_ROWS)
        .collect()
}

impl EventEmitter<QuickOpenEvent> for QuickOpen {}

impl Render for QuickOpen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        popup::layer(cx)
            .id("quick-open")
            .role(Role::Dialog)
            .aria_label(text::QUICK_OPEN)
            .test_support()
            .child(
                popup::panel(px(480.), cx)
                    .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(QuickOpenEvent::Close)))
                    .child(self.list.clone()),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::SystemTime;

    use holm_core::model::NoteInfo;

    use super::{MAX_ROWS, PickNotes};

    fn rows(query: &str, notes: &[NoteInfo]) -> Vec<usize> {
        super::rows(query, &PickNotes::new(notes.to_vec()))
    }

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
