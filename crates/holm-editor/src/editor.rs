use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::{
    App, Bounds, Context, CursorStyle, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, Pixels, Point, Render, ScrollEvent, Styled, UTF16Selection,
    Window, div,
};

use crate::element::{EditorElement, ImageSizes, Layout, Scroll};
use crate::model::Model;

/// The live preview editor for one note body.
///
/// The text is the note body only, with LF line breaks. Markdown syntax shows on the lines
/// with the cursor or the selection, and hides on all other lines. Embedded images show
/// below their line.
///
/// The editor takes its font, size and line height from the text style of its parent element.
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
    selecting: bool,
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
            selecting: false,
        }
    }

    /// Shows `text` from its start, with the cursor at the start.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.model = Model::new(text);
        self.scroll = Scroll::default();
        self.goal_x = None;
        self.version += 1;
        cx.notify();
    }

    /// The text.
    pub fn text(&self) -> &str {
        self.model.text()
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
        if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
            return;
        }
        let extend = modifiers.shift;
        let key = event.keystroke.key.as_str();
        match key {
            "left" => self.model.move_left(extend),
            "right" => self.model.move_right(extend),
            "up" | "down" => self.move_vertical(key == "down", extend),
            "backspace" | "delete" | "enter" => {
                let changed = match key {
                    "backspace" => self.model.backspace(),
                    "delete" => self.model.delete(),
                    _ => {
                        self.model.insert("\n");
                        true
                    }
                };
                if changed {
                    self.edited(cx);
                }
                cx.stop_propagation();
                return;
            }
            _ => return,
        }
        if !matches!(key, "up" | "down") {
            self.goal_x = None;
        }
        self.moved(cx);
        cx.stop_propagation();
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
        if let Some(offset) = self.offset_at(event.position) {
            self.model.set_cursor(offset, event.modifiers.shift);
            self.selecting = true;
            self.goal_x = None;
            cx.notify();
        }
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        if let Some(offset) = self.offset_at(event.position)
            && offset != self.model.cursor()
        {
            self.model.set_cursor(offset, true);
            cx.notify();
        }
    }

    fn mouse_up(&mut self, _event: &MouseUpEvent, _window: &mut Window, _cx: &mut Context<Self>) {
        self.selecting = false;
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
        self.model.replace(range, text);
        self.edited(cx);
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
