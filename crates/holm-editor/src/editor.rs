use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::{
    App, Bounds, ClipboardItem, Context, CursorStyle, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render,
    ScrollEvent, Styled, UTF16Selection, Window, div,
};

use crate::element::{EditorElement, ImageSizes, Layout, Scroll};
use crate::model::{Format, Model};

/// Spaces that Tab types outside a list.
const TAB: &str = "    ";

/// The live preview editor for one note body.
///
/// The text is the note body only, with LF line breaks. The editor turns other line breaks
/// into LF. Markdown syntax shows on the lines with the cursor or the selection, and hides on
/// all other lines. Embedded images show below their line.
///
/// The editor takes its font, size and line height from the text style of its parent element.
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
///
/// [`WritingHelp`] turns off the format keys and the list keys.
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
    goal_x: Option<Pixels>,
    drag: Option<Drag>,
    help: WritingHelp,
}

/// A mouse drag after a click. The selection grows by `unit` from `origin`.
#[derive(Clone, Debug)]
struct Drag {
    unit: Unit,
    origin: Range<usize>,
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
    /// The keys that add or remove Markdown syntax, like Ctrl+B.
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

/// The user changed the text. [`Editor::set_text`] does not send it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Changed;

impl EventEmitter<Changed> for Editor {}

impl Editor {
    /// An empty editor.
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            model: Model::new(""),
            folder: None,
            focus: cx.focus_handle(),
            scroll: Scroll::default(),
            autoscroll: false,
            version: 0,
            layout: None,
            image_sizes: ImageSizes::default(),
            goal_x: None,
            drag: None,
            help: WritingHelp::default(),
        }
    }

    /// Shows `text` from its start, with the cursor at the start and an empty undo stack.
    /// Use it to open a note.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.model = Model::new(text);
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

    /// Sets the folder of the note. Relative paths of embedded files start there.
    pub fn set_folder(&mut self, folder: Option<PathBuf>, cx: &mut Context<Self>) {
        self.folder = folder;
        self.image_sizes.clear();
        cx.notify();
    }

    /// The last layout, when it still shows the current text.
    fn current_layout(&self) -> Option<&Layout> {
        self.layout
            .as_deref()
            .filter(|layout| layout.version == self.version)
    }

    fn edited(&mut self, cx: &mut Context<Self>) {
        self.version += 1;
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
            "c" => {
                self.copy(cx);
                return true;
            }
            "x" => {
                self.copy(cx);
                let changed = self.model.remove_selection();
                self.finish(changed, cx);
                return true;
            }
            "v" => {
                let text = cx.read_from_clipboard().and_then(|item| item.text());
                let changed = text.is_some_and(|text| self.model.insert(&text));
                self.finish(changed, cx);
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

    /// A key with no Ctrl. Shift extends the selection.
    fn plain_key(&mut self, key: &str, shift: bool, cx: &mut Context<Self>) -> bool {
        let now = cx.background_executor().now();
        match key {
            "left" => self.model.move_left(shift),
            "right" => self.model.move_right(shift),
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
                let offset = self
                    .current_layout()
                    .and_then(|layout| layout.row_edge(cursor, end))
                    .unwrap_or_else(|| {
                        let line = self.model.line_range(self.model.line_of(cursor));
                        if end { line.end } else { line.start }
                    });
                self.model.set_cursor(offset, shift);
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
        if let Some((offset, x)) = self
            .current_layout()
            .and_then(|layout| layout.vertical(cursor, self.goal_x, down))
        {
            self.goal_x = Some(x);
            self.model.set_cursor(offset, extend);
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
        window.focus(&self.focus, cx);
        let Some(offset) = self.offset_at(event.position) else {
            return;
        };
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
        self.drag = Some(Drag { unit, origin });
        self.drag_to(offset);
        self.goal_x = None;
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.drag.is_none() {
            return;
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
        div()
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
            .child(EditorElement {
                editor: cx.entity(),
            })
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
        self.current_layout()?.cursor_bounds(self.model.cursor())
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
