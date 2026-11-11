use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;

use std::collections::HashSet;

use gpui_kit::{
    AnyView, App, Bounds, ClipboardEntry, ClipboardItem, Context, CursorStyle, EntityInputHandler,
    EventEmitter, ExternalPaths, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyDownEvent, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, Pixels, Point, Render, ScrollEvent, Styled, TestSupportExt as _, UTF16Selection,
    Window, div, px,
};
use holm_markdown::{Color, Target};

use crate::color::{self, ColorWindow, Paint, Step};
use crate::element::{EditorElement, Hit, ImageSizes, Layout, Scroll, Theme};
use crate::find::Find;
use crate::highlight::{self, Highlighter};
use crate::ink;
use crate::model::{Format, Model};

/// Spaces that Tab types outside a list.
const TAB: &str = "    ";
/// How far the mouse moves with the button down before a click becomes a drag.
const DRAG_START: Pixels = px(4.);

/// The live preview editor for one note body.
///
/// The text is the note body only, with LF line breaks. The editor turns other line breaks
/// into LF. Markdown syntax shows on the lines with the cursor or the selection, and hides on
/// all other lines. A code block or a table shows its whole source while the cursor or the
/// selection is in it. Embedded images show below their line, and other embedded files show
/// as file cards.
///
/// The editor takes its font, size and line height from the text style of its parent element.
/// It draws its own background and centers a text column of a fixed width. The app sets light
/// or dark mode with [`Editor::set_appearance`].
///
/// The editor handles its keys itself, so the app binds none of them:
///
/// - Arrows move by char, and with Ctrl by word. Home and End go to the row ends, and with
///   Ctrl to the note ends. Page Up and Page Down move by one view. Shift selects.
/// - Ctrl+A selects all. Ctrl+C, Ctrl+X and Ctrl+V copy, cut and paste the Markdown source.
/// - Ctrl+Z undoes, Ctrl+Y redoes. Ctrl+Backspace and Ctrl+Delete remove a word.
/// - Format keys: Ctrl+B bold, Ctrl+I italic, Ctrl+U underline, Ctrl+D strikethrough,
///   Ctrl+H highlight, Ctrl+E code, Ctrl+1 to Ctrl+6 a heading, Ctrl+0 a plain line.
/// - Enter continues a list. Tab and Shift+Tab move a list item in and out.
/// - Ctrl+L checks or clears the task boxes on the selected lines. A click on a task box does too.
/// - A click places the cursor, also on a link. Ctrl+click opens the link under the mouse, and
///   Ctrl+Enter the link at the cursor. A link to a heading in this note moves the cursor
///   there. Other links send [`OpenLink`] or [`OpenFile`].
/// - Ctrl+V with files or an image on the clipboard sends [`PasteFiles`], and so does a drop of
///   files.
/// - Ctrl+T opens the color window for the text color, and Ctrl+M for the highlight color.
///   Keys 1 to 6 pick a palette color, 7 to 9 one of the last custom colors, and 0 removes the
///   color. `+` opens a hex field, and Enter takes its value. Esc closes the window. A new
///   custom color sends [`CustomColors`].
/// - Up with the cursor at the very start of the text sends [`UpFromStart`].
/// - Ctrl+F opens find. Typed text goes into its query. Enter and Shift+Enter select the next
///   and the match before. Esc or a click in the text closes it.
///
/// [`WritingHelp`] turns off the format keys, the color window and the list keys.
pub struct Editor {
    pub(crate) model: Model,
    pub(crate) folder: Option<PathBuf>,
    pub(crate) focus: FocusHandle,
    pub(crate) scroll: Scroll,
    /// Whether the next layout moves the view to the cursor.
    pub(crate) autoscroll: bool,
    /// The count of text changes, to know when [`Self::layout`] is out of date.
    pub(crate) version: u64,
    pub(crate) layout: Option<Rc<Layout>>,
    pub(crate) image_sizes: ImageSizes,
    pub(crate) highlighter: Highlighter,
    pub(crate) theme: Theme,
    goal_x: Option<Pixels>,
    /// The cursor offset that shows at the start of its row, not at the end of the row above.
    /// A wrap inside a word puts both places at the same offset. Only Up, Down, Home and End
    /// set it.
    row_start: Option<usize>,
    /// After Up or Down into a line that showed no text: the row to put the cursor on at the
    /// goal x, once the next frame shows the source of that line.
    pub(crate) goal_row: Option<GoalRow>,
    drag: Option<Drag>,
    help: WritingHelp,
    pub(crate) find: Option<Find>,
    color: Option<ColorWindow>,
    /// The last custom colors, the newest first.
    custom_colors: Vec<Color>,
    pub(crate) end: Option<AnyView>,
}

/// The row for the cursor after Up or Down into a line that showed no text.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GoalRow {
    /// The cursor that Up or Down set. Any other cursor drops the goal row.
    cursor: usize,
    /// The first row after Down, the last row after Up.
    first: bool,
    extend: bool,
}

/// A mouse drag after a click. The selection grows by `unit` from `origin`.
///
/// A click can show the syntax of its line and so move the text under the mouse. So the
/// drag waits until the mouse is [`DRAG_START`] away from `down`.
#[derive(Clone, Debug)]
struct Drag {
    unit: Unit,
    origin: Range<usize>,
    down: Point<Pixels>,
    moving: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unit {
    Char,
    Word,
    Line,
}

/// The writing helps that the user can turn off. All are on by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct WritingHelp {
    /// The keys that add or remove Markdown syntax, like Ctrl+B, and the color window.
    pub format_keys: bool,
    /// Enter in a list item starts the next item.
    pub continue_lists: bool,
    /// Tab and Shift+Tab move a list item in and out.
    pub indent_lists: bool,
}

impl Default for WritingHelp {
    fn default() -> Self {
        Self {
            format_keys: true,
            continue_lists: true,
            indent_lists: true,
        }
    }
}

/// Light or dark mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    /// Dark text on a light background.
    #[default]
    Light,
    /// Light text on a dark background.
    Dark,
}

/// The user changed the text. [`Editor::set_text`] does not send it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Changed;

impl EventEmitter<Changed> for Editor {}

/// The user pressed Up with the cursor at the very start of the text, and no selection.
/// The app can move the focus to a part above the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpFromStart;

impl EventEmitter<UpFromStart> for Editor {}

/// The user asked to open a file: with the "Open" button of a file card, or a file link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenFile {
    /// The path of the file. A relative link starts at the folder of the note.
    pub path: PathBuf,
}

impl EventEmitter<OpenFile> for Editor {}

/// The user asked to open a link to a note or a web address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenLink {
    /// A [`Target::Note`] or a [`Target::Web`].
    pub target: Target,
}

impl EventEmitter<OpenLink> for Editor {}

/// The user pasted or dropped files or an image. The app stores them and calls
/// [`Editor::insert_link`] for each one. A drop first moves the cursor to the drop point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasteFiles {
    /// The files, in order.
    pub files: Vec<PastedFile>,
}

impl EventEmitter<PasteFiles> for Editor {}

/// One pasted or dropped file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PastedFile {
    /// A file on the disk.
    Path(PathBuf),
    /// Image data from the clipboard, with no file yet.
    Image {
        /// The file extension of its format, like `png`.
        extension: &'static str,
        /// The bytes of the image file.
        bytes: Vec<u8>,
    },
}

/// The last custom colors changed, after the user picked a color that is not in the palette.
/// The app stores them and gives them back with [`Editor::set_custom_colors`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomColors {
    /// The colors, the newest first. There are at most 3.
    pub colors: Vec<Color>,
}

impl EventEmitter<CustomColors> for Editor {}

/// The link at the cursor, from [`Editor::link_at_cursor`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorLink {
    /// Where the link points.
    pub target: Target,
    /// Whether it is `![..](..)`: an image or a file card.
    pub embedded: bool,
}

impl Editor {
    /// An empty editor in light mode.
    pub fn new(cx: &mut Context<Self>) -> Self {
        cx.background_executor()
            .spawn(async {
                highlight::warm_up();
                ink::warm_up();
            })
            .detach();
        Self {
            model: Model::new(""),
            folder: None,
            focus: cx.focus_handle(),
            scroll: Scroll::default(),
            autoscroll: false,
            version: 0,
            layout: None,
            image_sizes: ImageSizes::default(),
            highlighter: Highlighter::default(),
            theme: Theme::new(Appearance::Light),
            goal_x: None,
            row_start: None,
            goal_row: None,
            drag: None,
            help: WritingHelp::default(),
            find: None,
            color: None,
            custom_colors: Vec::new(),
            end: None,
        }
    }

    /// Shows `text` from its start, with the cursor at the start and an empty undo stack.
    /// Use it to open a note.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let broken = self.model.take_broken();
        self.model = Model::new(text);
        self.model.set_broken(broken);
        self.find = None;
        self.color = None;
        self.scroll = Scroll::default();
        self.goal_x = None;
        self.version += 1;
        cx.notify();
    }

    /// Puts in `text` from the disk in place of the current text, as one undo step.
    /// The view and the cursor stay near their place. It sends no [`Changed`].
    pub fn reload_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.model.reload(text) {
            self.version += 1;
            self.goal_x = None;
            if let Some(find) = &mut self.find {
                find.refresh(&mut self.model);
            }
            cx.notify();
        }
    }

    /// The text.
    pub fn text(&self) -> &str {
        self.model.text()
    }

    /// Sets the writing helps, from the settings.
    pub fn set_writing_help(&mut self, help: WritingHelp) {
        self.help = help;
    }

    /// Sets the last custom colors, the newest first, from the settings. The color window shows
    /// the first 3 on keys 7 to 9.
    pub fn set_custom_colors(&mut self, colors: &[Color], cx: &mut Context<Self>) {
        self.custom_colors = colors.iter().copied().take(color::CUSTOM_COUNT).collect();
        cx.notify();
    }

    /// Sets light or dark mode, from the app theme.
    pub fn set_appearance(&mut self, appearance: Appearance, cx: &mut Context<Self>) {
        self.theme = Theme::new(appearance);
        cx.notify();
    }

    /// Sets the folder of the note. Relative paths of embedded files start there.
    pub fn set_folder(&mut self, folder: Option<PathBuf>, cx: &mut Context<Self>) {
        self.folder = folder;
        self.image_sizes.clear();
        cx.notify();
    }

    /// Sets the link targets that are missing, like a note link to a heading that the note does
    /// not have. Their links get a quiet "broken" style. The editor itself checks links to
    /// headings in this note. The set stays for the next notes, until the next call.
    pub fn set_link_states(&mut self, broken: HashSet<Target>, cx: &mut Context<Self>) {
        self.model.set_broken(broken);
        cx.notify();
    }

    /// The selected source text. It is empty with no selection.
    pub fn selected_text(&self) -> &str {
        self.model.selected_text()
    }

    /// The link that holds the cursor, or ends at it.
    pub fn link_at_cursor(&self) -> Option<CursorLink> {
        self.model
            .link_at(self.model.cursor())
            .map(|link| CursorLink {
                target: link.target.clone(),
                embedded: link.embedded,
            })
    }

    /// Puts a link to `target` in place of the selection, as one undo step, and the cursor
    /// after it. The selection becomes the link text. With no selection, `text` does, written
    /// so that it shows as it is. `embedded` writes `![..](..)` on a line of its own, which shows
    /// an image or a file card. It sends [`Changed`].
    pub fn insert_link(
        &mut self,
        text: &str,
        target: &Target,
        embedded: bool,
        cx: &mut Context<Self>,
    ) {
        let changed = self.model.insert_link(text, target, embedded);
        self.finish(changed, cx);
    }

    /// Points the link at the cursor to `target`, as one undo step. The link text stays.
    /// It returns whether the text changed. It changes nothing with no link at the cursor, or
    /// for a reference link like `[text][name]`. It sends [`Changed`] when the text changes.
    pub fn set_link_target(&mut self, target: &Target, cx: &mut Context<Self>) -> bool {
        let changed = self.model.set_link_target(target);
        self.finish(changed, cx);
        changed
    }

    /// Puts the cursor at the first heading with the text `heading`, and shows that heading at
    /// the top of the view. It returns whether the text has such a heading.
    pub fn show_heading(&mut self, heading: &str, cx: &mut Context<Self>) -> bool {
        let Some(start) = self.model.heading_start(heading) else {
            return false;
        };
        self.model.set_cursor(start, false);
        self.goal_x = None;
        self.scroll = Scroll {
            line: self.model.line_of(start),
            offset: px(0.),
        };
        cx.notify();
        true
    }

    /// Sets a view that shows after the last line of the text, in the text column, or none.
    /// It scrolls with the text.
    pub fn set_end(&mut self, end: Option<AnyView>, cx: &mut Context<Self>) {
        self.end = end;
        cx.notify();
    }

    /// Selects the text of the title, so typing replaces it. The title is a `# heading` that is
    /// the first block. With no title, the cursor goes to the start of the text.
    pub fn select_title(&mut self, cx: &mut Context<Self>) {
        match holm_markdown::title_range(self.model.text()) {
            Some(range) => self.model.select(range),
            None => self.model.set_cursor(0, false),
        }
        self.goal_x = None;
        self.moved(cx);
    }

    /// Opens `target`: moves to a heading in this note, or tells the app.
    fn open_link(&mut self, target: Target, cx: &mut Context<Self>) {
        match target {
            Target::Heading(name) => {
                if let Some(start) = self.model.heading_start(&name) {
                    self.model.set_cursor(start, false);
                    self.goal_x = None;
                    self.moved(cx);
                }
            }
            Target::File(path) => {
                let path = match &self.folder {
                    Some(folder) => folder.join(path),
                    None => PathBuf::from(path),
                };
                cx.emit(OpenFile { path });
            }
            target => cx.emit(OpenLink { target }),
        }
    }

    /// Puts the cursor on its goal row after Up or Down, now that `layout` shows its line.
    pub(crate) fn reach_goal_row(&mut self, layout: &Layout) {
        let Some(goal) = self.goal_row.take() else {
            return;
        };
        let cursor = self.model.cursor();
        let Some(x) = self.goal_x.filter(|_| cursor == goal.cursor) else {
            return;
        };
        if let Some((offset, row_start)) = layout.row_at(cursor, x, goal.first) {
            self.model.set_cursor(offset, goal.extend);
            self.row_start = row_start.then_some(offset);
        }
    }

    /// Whether the cursor shows at the start of its row. See [`Self::row_start`].
    pub(crate) fn row_start(&self) -> bool {
        self.row_start == Some(self.model.cursor())
    }

    /// The last layout, when it still shows the current text.
    fn current_layout(&self) -> Option<&Layout> {
        self.layout
            .as_deref()
            .filter(|layout| layout.version == self.version)
    }

    fn edited(&mut self, cx: &mut Context<Self>) {
        self.version += 1;
        self.row_start = None;
        self.goal_x = None;
        self.autoscroll = true;
        cx.emit(Changed);
        cx.notify();
    }

    fn moved(&mut self, cx: &mut Context<Self>) {
        self.autoscroll = true;
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.alt || modifiers.platform || modifiers.function {
            return;
        }
        let key = event.keystroke.key.as_str();
        // These keys set the row start again. Any other key drops it.
        if !matches!(key, "up" | "down" | "home" | "end") {
            self.row_start = None;
        }
        if self.color.is_some() {
            if self.color_key(event, cx) {
                cx.stop_propagation();
            }
            return;
        }
        if self.find.is_some() {
            if self.find_key(key, modifiers, cx) {
                cx.stop_propagation();
            }
            return;
        }
        let handled = if modifiers.control {
            self.control_key(key, modifiers.shift, cx)
        } else {
            self.plain_key(key, modifiers.shift, cx)
        };
        if handled {
            cx.stop_propagation();
        }
    }

    /// A key with Ctrl. Ctrl+Shift with a letter belongs to the app.
    fn control_key(&mut self, key: &str, shift: bool, cx: &mut Context<Self>) -> bool {
        let now = cx.background_executor().now();
        let format = match key {
            "b" => Some(Format::Bold),
            "i" => Some(Format::Italic),
            "u" => Some(Format::Underline),
            "d" => Some(Format::Strikethrough),
            "h" => Some(Format::Highlight),
            "e" => Some(Format::Code),
            _ => None,
        };
        let heading = key
            .parse::<u8>()
            .ok()
            .filter(|level| *level <= 6 && key.len() == 1);
        match key {
            "left" => self.model.move_word_left(shift),
            "right" => self.model.move_word_right(shift),
            "home" => self.model.set_cursor(0, shift),
            "end" => self.model.set_cursor(self.model.text().len(), shift),
            "enter" if !shift => {
                let Some(link) = self.model.link_at(self.model.cursor()) else {
                    return false;
                };
                self.open_link(link.target.clone(), cx);
                return true;
            }
            "backspace" | "delete" if !shift => {
                let changed = if key == "backspace" {
                    self.model.backspace(true, now)
                } else {
                    self.model.delete(true, now)
                };
                self.finish(changed, cx);
                return true;
            }
            _ if shift => return false,
            "a" => self.model.select_all(),
            "l" => {
                let changed = self.model.toggle_tasks();
                self.finish(changed, cx);
                return true;
            }
            "c" => {
                self.copy(cx);
                return true;
            }
            "f" => {
                self.open_find(cx);
                return true;
            }
            "x" => {
                self.copy(cx);
                let changed = self.model.remove_selection();
                self.finish(changed, cx);
                return true;
            }
            "v" => {
                self.paste(cx);
                return true;
            }
            "z" | "y" => {
                let changed = if key == "z" {
                    self.model.undo()
                } else {
                    self.model.redo()
                };
                self.finish(changed, cx);
                return true;
            }
            _ if !self.help.format_keys => return false,
            "t" | "m" => {
                let paint = if key == "t" {
                    Paint::Text
                } else {
                    Paint::Highlight
                };
                self.color = Some(ColorWindow::new(paint));
                cx.notify();
                return true;
            }
            _ if format.is_some() || heading.is_some() => {
                let changed = match (format, heading) {
                    (Some(format), _) => self.model.toggle_format(format),
                    (None, Some(level)) => self.model.toggle_heading(level),
                    (None, None) => false,
                };
                self.finish(changed, cx);
                return true;
            }
            _ => return false,
        }
        self.goal_x = None;
        self.moved(cx);
        true
    }

    /// A key while the color window is open. It returns whether the key stops here. Ctrl+V
    /// pastes into the hex field. Other keys with Ctrl do nothing.
    fn color_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(window) = &mut self.color else {
            return false;
        };
        let keystroke = &event.keystroke;
        if keystroke.modifiers.control {
            if keystroke.key == "v"
                && let Some(text) = cx.read_from_clipboard().and_then(|item| item.text())
            {
                window.push(&text);
                cx.notify();
            }
            return true;
        }
        match window.key(keystroke, &self.custom_colors) {
            Step::Type => return false,
            Step::Stay => {}
            Step::Close => self.color = None,
            Step::Apply(color) => {
                let paint = window.paint;
                self.color = None;
                let changed = self.model.toggle_format(paint.format(color));
                self.finish(changed, cx);
                if let Some(color) = color.filter(|c| paint.is_custom(*c))
                    && color::remember(&mut self.custom_colors, color)
                {
                    cx.emit(CustomColors {
                        colors: self.custom_colors.clone(),
                    });
                }
            }
        }
        cx.notify();
        true
    }

    /// Opens find. A selection on one line becomes the query.
    fn open_find(&mut self, cx: &mut Context<Self>) {
        let selected = self.model.selected_text();
        let query = if selected.contains('\n') {
            String::new()
        } else {
            selected.to_owned()
        };
        let mut find = Find::new(query, self.model.selection().start);
        find.refresh(&mut self.model);
        self.find = Some(find);
        self.moved(cx);
    }

    /// Pastes text, or sends [`PasteFiles`] for files or an image.
    fn paste(&mut self, cx: &mut Context<Self>) {
        let item = cx.read_from_clipboard();
        let files = item.as_ref().map(pasted_files).unwrap_or_default();
        if !files.is_empty() {
            cx.emit(PasteFiles { files });
            return;
        }
        let text = item.and_then(|item| item.text());
        let changed = text.is_some_and(|text| self.model.insert(&text));
        self.finish(changed, cx);
    }

    /// A key while find is open. It returns whether the key stops here.
    /// Typed chars go on to the text input, which adds them to the query.
    /// Other keys with Ctrl go on to the app. Other named keys do nothing.
    fn find_key(&mut self, key: &str, modifiers: Modifiers, cx: &mut Context<Self>) -> bool {
        let Some(find) = &mut self.find else {
            return false;
        };
        match (modifiers.control, key) {
            (false, "escape") => self.find = None,
            (false, "enter") => find.step(modifiers.shift, &mut self.model),
            (_, "backspace") => {
                find.pop(modifiers.control);
                find.refresh(&mut self.model);
            }
            (true, "f") if !modifiers.shift => {}
            (true, "v") => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    find.push(&text);
                    find.refresh(&mut self.model);
                }
            }
            (true, _) => return false,
            (false, _) => return key.chars().count() > 1 && key != "space",
        }
        self.goal_x = None;
        self.moved(cx);
        true
    }

    /// A key with no Ctrl. Shift extends the selection.
    fn plain_key(&mut self, key: &str, shift: bool, cx: &mut Context<Self>) -> bool {
        let now = cx.background_executor().now();
        match key {
            "left" => self.model.move_left(shift),
            "right" => self.model.move_right(shift),
            "up" if !shift && self.model.selection() == (0..0) => {
                cx.emit(UpFromStart);
                return true;
            }
            "up" | "down" => {
                self.move_vertical(key == "down", shift);
                self.moved(cx);
                return true;
            }
            "pageup" | "pagedown" => {
                self.move_page(key == "pagedown", shift);
                self.moved(cx);
                return true;
            }
            "home" | "end" => {
                let cursor = self.model.cursor();
                let end = key == "end";
                let row_start = self.row_start();
                let (offset, row_start) = self
                    .current_layout()
                    .and_then(|layout| layout.row_edge(cursor, row_start, end))
                    .unwrap_or_else(|| {
                        let line = self.model.line_range(self.model.line_of(cursor));
                        (if end { line.end } else { line.start }, false)
                    });
                self.model.set_cursor(offset, shift);
                self.row_start = row_start.then_some(offset);
            }
            "backspace" | "delete" if !shift || key == "backspace" => {
                let changed = if key == "backspace" {
                    self.model.backspace(false, now)
                } else {
                    self.model.delete(false, now)
                };
                self.finish(changed, cx);
                return true;
            }
            "enter" => {
                let changed = self.model.newline(self.help.continue_lists, now);
                self.finish(changed, cx);
                return true;
            }
            "tab" => {
                let indented = self
                    .help
                    .indent_lists
                    .then(|| self.model.indent(shift))
                    .flatten();
                let changed = match indented {
                    Some(changed) => changed,
                    None if shift => false,
                    None => self.model.type_text(TAB, now),
                };
                self.finish(changed, cx);
                return true;
            }
            _ => return false,
        }
        self.goal_x = None;
        self.moved(cx);
        true
    }

    /// Tells the app and the view about an edit, when there was one.
    fn finish(&mut self, changed: bool, cx: &mut Context<Self>) {
        if changed {
            self.edited(cx);
        }
    }

    fn copy(&self, cx: &mut Context<Self>) {
        let text = self.model.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
        }
    }

    /// Moves the cursor by the rows that fit in the view.
    fn move_page(&mut self, down: bool, extend: bool) {
        let rows = self.current_layout().map_or(1, Layout::rows_in_view);
        for _ in 0..rows {
            self.move_vertical(down, extend);
        }
    }

    fn move_vertical(&mut self, down: bool, extend: bool) {
        let cursor = self.model.cursor();
        let row_start = self.row_start();
        if let Some(to) = self
            .current_layout()
            .and_then(|layout| layout.vertical(cursor, row_start, self.goal_x, down))
        {
            self.goal_x = Some(to.x);
            self.model.set_cursor(to.offset, extend);
            self.row_start = to.row_start.then_some(to.offset);
            self.goal_row = to.hidden.then_some(GoalRow {
                cursor: to.offset,
                first: down,
                extend,
            });
            return;
        }
        // The row is out of view: keep the byte column on the next source line.
        let line = self.model.line_of(cursor);
        let column = cursor - self.model.line_range(line).start;
        let target = if down {
            if line + 1 == self.model.line_count() {
                self.model.set_cursor(self.model.text().len(), extend);
                return;
            }
            line + 1
        } else {
            let Some(line) = line.checked_sub(1) else {
                self.model.set_cursor(0, extend);
                return;
            };
            line
        };
        let range = self.model.line_range(target);
        self.model
            .set_cursor((range.start + column).min(range.end), extend);
    }

    fn offset_at(&self, position: Point<Pixels>) -> Option<usize> {
        self.current_layout()?.offset_at(position)
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.row_start = None;
        window.focus(&self.focus, cx);
        self.find = None;
        self.color = None;
        let hit = self
            .current_layout()
            .and_then(|layout| layout.hit(event.position));
        match hit {
            Some(Hit::Task(range)) => {
                // The cursor stays, so the view does not move.
                if self.model.toggle_task(&range) {
                    self.version += 1;
                    cx.emit(Changed);
                    cx.notify();
                }
                return;
            }
            Some(Hit::Open(path)) => {
                cx.emit(OpenFile { path });
                return;
            }
            None => {}
        }
        let Some(offset) = self.offset_at(event.position) else {
            return;
        };
        if event.modifiers.control
            && let Some(link) = self.model.link_at(offset)
        {
            self.open_link(link.target.clone(), cx);
            return;
        }
        let unit = match event.click_count {
            0 | 1 => Unit::Char,
            2 => Unit::Word,
            _ => Unit::Line,
        };
        let origin = match unit {
            Unit::Char if event.modifiers.shift => {
                let anchor = self.model.selection();
                let anchor = if self.model.is_reversed() {
                    anchor.end
                } else {
                    anchor.start
                };
                anchor..anchor
            }
            Unit::Char => offset..offset,
            Unit::Word => self.model.word_at(offset),
            Unit::Line => self.model.line_at(offset),
        };
        self.drag = Some(Drag {
            unit,
            origin,
            down: event.position,
            moving: false,
        });
        self.drag_to(offset);
        self.goal_x = None;
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = &mut self.drag else { return };
        if !drag.moving {
            let distance = event.position - drag.down;
            if distance.x.abs() < DRAG_START && distance.y.abs() < DRAG_START {
                return;
            }
            drag.moving = true;
        }
        if let Some(offset) = self.offset_at(event.position) {
            let before = self.model.selection();
            self.drag_to(offset);
            if self.model.selection() != before {
                cx.notify();
            }
        }
    }

    /// Selects from the drag origin to the unit at `offset`.
    fn drag_to(&mut self, offset: usize) {
        let Some(drag) = &self.drag else { return };
        let target = match drag.unit {
            Unit::Char => offset..offset,
            Unit::Word => self.model.word_at(offset),
            Unit::Line => self.model.line_at(offset),
        };
        let origin = drag.origin.clone();
        if target.start < origin.start {
            self.model.set_cursor(origin.end, false);
            self.model.set_cursor(target.start, true);
        } else {
            self.model.set_cursor(origin.start, false);
            self.model.set_cursor(target.end.max(origin.end), true);
        }
    }

    fn mouse_up(&mut self, _event: &MouseUpEvent, _window: &mut Window, _cx: &mut Context<Self>) {
        self.drag = None;
    }

    fn drop_files(&mut self, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.find = None;
        self.color = None;
        if let Some(offset) = self.offset_at(window.mouse_position()) {
            self.model.set_cursor(offset, false);
        }
        let files = paths
            .paths()
            .iter()
            .cloned()
            .map(PastedFile::Path)
            .collect();
        cx.emit(PasteFiles { files });
        cx.notify();
    }

    fn scroll(&mut self, event: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.scroll.offset -= event.delta.pixel_delta(window.line_height()).y;
        cx.notify();
    }

    fn utf16_range(&self, range: &Range<usize>) -> Range<usize> {
        self.model.offset_to_utf16(range.start)..self.model.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.model.utf16_to_offset(range.start)..self.model.utf16_to_offset(range.end)
    }
}

impl Focusable for Editor {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Editor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let color = self.color.as_ref().map(|window| {
            // Below the cursor, or at the top of the text column when the cursor is not laid out.
            let at = self.current_layout().and_then(|layout| {
                layout
                    .cursor_bounds(self.model.cursor(), self.row_start())
                    .map(|cursor| cursor.bottom_left() + Point::new(px(0.), px(4.)))
                    .or(Some(layout.bounds.origin))
            });
            window.view(&self.theme, &self.custom_colors, at.unwrap_or_default())
        });
        let editor = div().id("editor");
        // A copy of the text per frame would cost too much in a large note.
        #[cfg(feature = "test-support")]
        let editor = {
            use gpui_kit::StatefulInteractiveElement as _;
            editor
                .role(gpui_kit::Role::MultilineTextInput)
                .aria_value(self.text().to_owned())
        };
        editor
            .test_support()
            .key_context("Editor")
            .track_focus(&self.focus)
            .size_full()
            .cursor(CursorStyle::IBeam)
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_scroll(cx.listener(Self::scroll))
            .on_drop(cx.listener(Self::drop_files))
            .relative()
            .child(EditorElement {
                editor: cx.entity(),
            })
            .children(self.find.as_ref().map(|find| find.bar(&self.theme)))
            .children(color)
    }
}

/// The platform text input. Holm has no input method support in v1, so marked text is plain text.
impl EntityInputHandler for Editor {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range);
        adjusted_range.replace(self.utf16_range(&range));
        Some(self.model.text()[range].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.utf16_range(&self.model.selection()),
            reversed: self.model.is_reversed(),
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(window) = &mut self.color {
            window.push(text);
            cx.notify();
            return;
        }
        if let Some(find) = &mut self.find {
            find.push(text);
            find.refresh(&mut self.model);
            self.moved(cx);
            return;
        }
        let range = range.map_or_else(|| self.model.selection(), |r| self.range_from_utf16(&r));
        let changed = self
            .model
            .type_in(range, text, cx.background_executor().now());
        self.finish(changed, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_text_in_range(range, new_text, window, cx);
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.current_layout()?
            .cursor_bounds(self.model.cursor(), self.row_start())
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.model.offset_to_utf16(self.offset_at(point)?))
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.range_from_utf16(&range_utf16);
        self.model.select(range);
        cx.notify();
    }

    fn text_length_utf16(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.model.offset_to_utf16(self.model.text().len()))
    }
}

/// The files or the image on the clipboard. Files come first. It is empty for text.
fn pasted_files(item: &ClipboardItem) -> Vec<PastedFile> {
    let paths: Vec<PastedFile> = item
        .entries()
        .iter()
        .filter_map(|entry| match entry {
            ClipboardEntry::ExternalPaths(paths) => Some(paths.paths()),
            _ => None,
        })
        .flatten()
        .cloned()
        .map(PastedFile::Path)
        .collect();
    if !paths.is_empty() {
        return paths;
    }
    item.entries()
        .iter()
        .find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(PastedFile::Image {
                extension: image.format.extension(),
                bytes: image.bytes.clone(),
            }),
            _ => None,
        })
        .into_iter()
        .collect()
}
