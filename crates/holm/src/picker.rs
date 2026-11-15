//! The folder picker for a move. It shows the folders whose path holds the typed text.
//! `↑` and `↓` pick a folder, `Enter` moves there, `Esc` cancels.

use std::path::{Path, PathBuf};

use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement, Render, Role, StatefulInteractiveElement as _, Subscription,
    TestSupportExt as _, Window, px,
};

use crate::icons::Icon;
use crate::popup::{self, PickEvent, PickList, PickRow};
use crate::sidebar::Target;
use crate::text;

/// The answer of the picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PickerEvent {
    Move { target: Target, folder: PathBuf },
    Cancel,
}

pub(crate) struct FolderPicker {
    target: Target,
    /// The root folder first, as the empty path.
    folders: Vec<PathBuf>,
    list: Entity<PickList>,
    /// The place in `folders` of each row.
    rows: Vec<usize>,
    _list: Subscription,
}

impl FolderPicker {
    /// A picker for `target`, over `folders`. It leaves out the folder of the target.
    /// For a folder move, it also leaves out that folder and its children. The field gets the
    /// focus.
    pub(crate) fn new(
        target: Target,
        folders: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (path, inside) = match &target {
            Target::Note(path) => (path.as_path(), None),
            Target::Folder(path) => (path.as_path(), Some(path.as_path())),
        };
        let home = path.parent().unwrap_or(Path::new(""));
        let folders = std::iter::once(PathBuf::new())
            .chain(folders.iter().cloned())
            .filter(|folder| {
                folder != home && inside.is_none_or(|inside| !folder.starts_with(inside))
            })
            .collect();
        let name = match &target {
            Target::Note(path) => path.file_stem(),
            Target::Folder(path) => path.file_name(),
        }
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
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
        let query = self.list.read(cx).query(cx).trim().to_lowercase();
        self.rows = self
            .folders
            .iter()
            .enumerate()
            .filter(|(_, folder)| name(folder).to_lowercase().contains(&query))
            .map(|(at, _)| at)
            .collect();
        let shown = self
            .rows
            .iter()
            .map(|at| PickRow::new(name(&self.folders[*at])).icon(Icon::Folder))
            .collect();
        self.list.update(cx, |list, cx| list.set_rows(shown, cx));
    }
}

/// The name that a row shows for `folder`: its path, or a name for the root folder.
fn name(folder: &Path) -> String {
    if folder.as_os_str().is_empty() {
        text::ROOT_FOLDER.to_owned()
    } else {
        folder.display().to_string()
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
