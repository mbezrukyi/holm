//! The folder picker for a move. It shows the folders whose path holds the typed text, with no regard to capital
//! letters. `Tab` and `Shift+Tab` pick a folder, `Enter` moves there, `Esc` cancels.

use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, ParentElement, Render, Role,
    StatefulInteractiveElement as _, Subscription, TestSupportExt as _, Window, px,
};

use holm_core::FolderPath;

use crate::icons::Icon;
use crate::lowercase::Lowercase;
use crate::popup::{self, PickEvent, PickList, PickRow};
use crate::sidebar::Target;
use crate::text;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PickerEvent {
    Move { target: Target, folder: FolderPath },
    Cancel,
}

pub(crate) struct FolderPicker {
    target: Target,
    /// The root folder first.
    folders: Vec<FolderPath>,
    list: Entity<PickList>,
    /// The place in `folders` of each row.
    rows: Vec<usize>,
    _list: Subscription,
}

impl FolderPicker {
    /// It leaves out the folder of the target. For a folder move, it also leaves out that folder and its children. The
    /// field gets the focus.
    pub(crate) fn new(target: Target, folders: &[FolderPath], window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (home, inside) = match &target {
            Target::Note(path) => (Some(path.folder().clone()), None),
            Target::Folder(path) => (path.folder(), Some(path)),
        };
        let folders = std::iter::once(FolderPath::root())
            .chain(folders.iter().cloned())
            .filter(|folder| home.as_ref() != Some(folder) && inside.is_none_or(|inside| inside.can_move_into(folder)))
            .collect();
        let name = match &target {
            Target::Note(path) => path.title().into_owned(),
            Target::Folder(path) => path.last_part().into_owned(),
        };
        let list = cx.new(|cx| PickList::new(text::move_to(&name), text::NO_FOLDER, window, cx));
        let subscription = cx.subscribe(&list, |this, _, event: &PickEvent, cx| match event {
            PickEvent::Query => this.query_changed(cx),
            PickEvent::Pick(at) => this.pick(*at, cx),
            PickEvent::Close => cx.emit(PickerEvent::Cancel),
        });
        let mut this = Self {
            target,
            folders,
            list,
            rows: Vec::new(),
            _list: subscription,
        };
        this.query_changed(cx);
        this
    }

    fn pick(&mut self, at: usize, cx: &mut Context<Self>) {
        if let Some(folder) = self.rows.get(at).and_then(|at| self.folders.get(*at)) {
            cx.emit(PickerEvent::Move {
                target: self.target.clone(),
                folder: folder.clone(),
            });
        }
    }

    fn query_changed(&mut self, cx: &mut Context<Self>) {
        let query = Lowercase::new(self.list.read(cx).query(cx).trim());
        self.rows = self
            .folders
            .iter()
            .enumerate()
            .filter(|(_, folder)| Lowercase::new(&text::folder_label(folder)).holds(&query))
            .map(|(at, _)| at)
            .collect();
        let shown = self
            .rows
            .iter()
            .map(|at| PickRow::new(text::folder_label(&self.folders[*at])).icon(Icon::Folder))
            .collect();
        self.list.update(cx, |list, cx| list.set_rows(shown, cx));
    }
}

impl EventEmitter<PickerEvent> for FolderPicker {}

impl Render for FolderPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        popup::layer(cx)
            .id("folder-picker")
            .role(Role::Dialog)
            .aria_label(text::MOVE_TO)
            .test_support()
            .child(
                popup::panel(px(480.), cx)
                    .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(PickerEvent::Cancel)))
                    .child(self.list.clone()),
            )
    }
}
