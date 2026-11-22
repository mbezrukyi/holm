//! A one-line text field that draws its own text, selection and caret.
//!
//! The field has no frame and no padding. It takes the text style of its parent, and it is as wide as its text and the
//! caret. In a narrower parent it shrinks and scrolls, so the caret stays in view. So the field lays out its text like
//! any other text of the same style.
//!
//! - `Enter` sends [`FieldEvent::Enter`], and `Esc` sends [`FieldEvent::Escape`].
//! - When the field loses the focus, it sends [`FieldEvent::Blur`].
//! - `←`, `→`, `Home` and `End` move the caret. With `Shift`, they select.
//! - `Backspace`, `Delete`, `Ctrl+A`, `Ctrl+C`, `Ctrl+X` and `Ctrl+V` work as usual.
//! - A click puts the caret, a drag selects, and a double click selects all.
//!
//! Typed text comes through the platform input handler. It has no undo. A locked field takes no keys and no typed text.

use std::ops::Range;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    App, Bounds, ClipboardItem, ContentMask, Context, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, InspectorElementId,
    InteractiveElement as _, IntoElement, KeyDownEvent, Keystroke, LayoutId, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, ParentElement as _, Pixels, Point, Render, Role, ShapedLine, SharedString,
    StatefulInteractiveElement as _, Style, Styled as _, Subscription, TestSupportExt as _, TextAlign, UTF16Selection,
    Window, div, fill, point, px, size,
};
use holm_editor::shortcuts::Command as TextCommand;

use crate::shortcuts::{Command, Place, Run};

const CARET_WIDTH: Pixels = px(1.);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FieldEvent {
    Enter,
    Escape,
    Blur,
}

/// What a caret move does to the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Selection {
    /// The anchor moves with the caret, so nothing stays selected.
    Collapse,
    /// The anchor stays, so the selection grows or shrinks.
    Extend,
}

impl Selection {
    /// Shift extends the selection.
    fn of(modifiers: Modifiers) -> Self {
        if modifiers.shift { Self::Extend } else { Self::Collapse }
    }
}

pub(crate) struct TextField {
    id: SharedString,
    label: SharedString,
    placeholder: SharedString,
    text: String,
    /// The fixed end of the selection, in bytes.
    anchor: usize,
    /// The end of the selection with the caret, in bytes.
    head: usize,
    focus: FocusHandle,
    /// The text as the last frame drew it, for clicks and the input method.
    shaped: Option<Shaped>,
    /// How far the text scrolls to the left, when the field is narrower than its text.
    scroll: Pixels,
    selecting: bool,
    locked: bool,
    _blur: Subscription,
}

#[derive(Clone)]
struct Shaped {
    line: ShapedLine,
    origin: Point<Pixels>,
    line_height: Pixels,
}

impl Shaped {
    /// The box of the glyphs from `start` to `end` on the line. The line paints its glyphs in the middle of the line
    /// height, so the box fits the text in any line height.
    fn span(&self, start: Pixels, end: Pixels) -> Bounds<Pixels> {
        let height = self.line.ascent + self.line.descent;
        let top = (self.line_height - height) / 2.;
        Bounds::new(self.origin + point(start, top), size(end - start, height))
    }
}

impl TextField {
    /// A field with `text`, all selected. The element gets `id` and the accessible name `label`.
    pub(crate) fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        let blur = cx.on_blur(&focus, window, |_, _, cx| cx.emit(FieldEvent::Blur));
        Self {
            id: id.into(),
            label: label.into(),
            placeholder: SharedString::default(),
            text: text.to_owned(),
            anchor: 0,
            head: text.len(),
            focus,
            shaped: None,
            scroll: px(0.),
            selecting: false,
            locked: false,
            _blur: blur,
        }
    }

    /// Shows `placeholder` muted while the field is empty.
    pub(crate) fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn set_locked(&mut self, locked: bool) {
        self.locked = locked;
    }

    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    fn select(&mut self, to: usize, selection: Selection, cx: &mut Context<Self>) {
        self.head = to;
        if selection == Selection::Collapse {
            self.anchor = to;
        }
        cx.notify();
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        self.anchor = 0;
        self.head = self.text.len();
        cx.notify();
    }

    /// Puts `text` in the place of `range`, with the caret after it.
    fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        let text = text.replace(['\n', '\r'], " ");
        self.text.replace_range(range.clone(), &text);
        self.select(range.start + text.len(), Selection::Collapse, cx);
    }

    fn previous(&self, at: usize) -> usize {
        self.text[..at].char_indices().next_back().map_or(0, |(at, _)| at)
    }

    fn next(&self, at: usize) -> usize {
        self.text[at..].chars().next().map_or(at, |char| at + char.len_utf8())
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if self.locked || modifiers.alt || modifiers.platform {
            return;
        }
        self.edit_key(keystroke, self.selection(), cx);
    }

    fn run(&mut self, Run(command): &Run, _: &mut Window, cx: &mut Context<Self>) {
        if self.locked {
            return cx.propagate();
        }
        let selection = self.selection();
        match command {
            Command::Confirm => cx.emit(FieldEvent::Enter),
            Command::Cancel => cx.emit(FieldEvent::Escape),
            Command::Text(TextCommand::SelectAll) => self.select_all(cx),
            Command::Text(command @ (TextCommand::Copy | TextCommand::Cut)) => {
                if !selection.is_empty() {
                    let text = self.text[selection.clone()].to_owned();
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    if *command == TextCommand::Cut {
                        self.replace(selection, "", cx);
                    }
                }
            }
            Command::Text(TextCommand::Paste) => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.replace(selection, &text, cx);
                }
            }
            _ => cx.propagate(),
        }
    }

    /// A cursor key, Backspace or Delete. It stops the key when the field takes it.
    fn edit_key(&mut self, keystroke: &Keystroke, selection: Range<usize>, cx: &mut Context<Self>) {
        let shift = Selection::of(keystroke.modifiers);
        match (keystroke.modifiers.control, keystroke.key.as_str()) {
            (false, "left") if shift == Selection::Collapse && !selection.is_empty() => {
                self.select(selection.start, Selection::Collapse, cx);
            }
            (false, "right") if shift == Selection::Collapse && !selection.is_empty() => {
                self.select(selection.end, Selection::Collapse, cx);
            }
            (false, "left") => self.select(self.previous(self.head), shift, cx),
            (false, "right") => self.select(self.next(self.head), shift, cx),
            (false, "home") => self.select(0, shift, cx),
            (false, "end") => self.select(self.text.len(), shift, cx),
            (false, "backspace") if selection.is_empty() => {
                self.replace(self.previous(self.head)..self.head, "", cx);
            }
            (false, "delete") if selection.is_empty() => {
                self.replace(self.head..self.next(self.head), "", cx);
            }
            (false, "backspace" | "delete") => self.replace(selection, "", cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    /// The byte offset nearest to `position` in the window.
    fn offset_at(&self, position: Point<Pixels>) -> usize {
        self.shaped.as_ref().map_or(0, |shaped| {
            if self.text.is_empty() {
                0
            } else {
                shaped.line.closest_index_for_x(position.x - shaped.origin.x)
            }
        })
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        window.focus(&self.focus, cx);
        if event.click_count >= 2 {
            self.select_all(cx);
            return;
        }
        self.selecting = true;
        let at = self.offset_at(event.position);
        self.select(at, Selection::of(event.modifiers), cx);
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting && event.pressed_button == Some(MouseButton::Left) {
            let at = self.offset_at(event.position);
            self.select(at, Selection::Extend, cx);
        }
    }

    fn utf16_offset(&self, offset: usize) -> Utf16Offset {
        Utf16Offset(self.text[..offset].encode_utf16().count())
    }

    fn offset_from_utf16(&self, utf16: Utf16Offset) -> usize {
        let mut count = 0;
        for (at, char) in self.text.char_indices() {
            if count >= utf16.0 {
                return at;
            }
            count += char.len_utf16();
        }
        self.text.len()
    }

    fn utf16_range(&self, range: &Range<usize>) -> Range<Utf16Offset> {
        self.utf16_offset(range.start)..self.utf16_offset(range.end)
    }

    fn range_from_utf16(&self, range: Range<Utf16Offset>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }
}

/// An offset in UTF-16 code units, as the platform text input counts. The text counts bytes.
#[derive(Clone, Copy)]
struct Utf16Offset(usize);

impl Utf16Offset {
    fn range_from_platform(range: Range<usize>) -> Range<Self> {
        Self(range.start)..Self(range.end)
    }

    fn range_to_platform(range: Range<Self>) -> Range<usize> {
        range.start.0..range.end.0
    }

    fn to_platform(self) -> usize {
        self.0
    }
}

impl EventEmitter<FieldEvent> for TextField {}

impl Focusable for TextField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TextField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id(ElementId::Name(self.id.clone()))
            .role(Role::TextInput)
            .aria_label(self.label.clone())
            .aria_value(SharedString::from(self.text.clone()))
            .test_support()
            .key_context(Place::TextField.key_context())
            .track_focus(&self.focus)
            .flex_shrink_1()
            .min_w_0()
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(Self::run))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(|field, _, _, _| field.selecting = false))
            .child(Line(Source::Field(cx.entity())))
    }
}

/// The platform text input. Marked text of an input method is plain text.
impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let bytes = self.range_from_utf16(Utf16Offset::range_from_platform(range));
        adjusted_range.replace(Utf16Offset::range_to_platform(self.utf16_range(&bytes)));
        Some(self.text[bytes].to_owned())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: Utf16Offset::range_to_platform(self.utf16_range(&self.selection())),
            reversed: self.head < self.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.locked {
            return;
        }
        // A key that no binding takes can come here as its control char, like Tab on the test platform.
        let text: String = text.chars().filter(|char| !char.is_control()).collect();
        if text.is_empty() {
            return;
        }
        let range = range.map_or_else(
            || self.selection(),
            |range| self.range_from_utf16(Utf16Offset::range_from_platform(range)),
        );
        self.replace(range, &text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_text_in_range(range, text, window, cx);
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let shaped = self.shaped.as_ref()?;
        let range = self.range_from_utf16(Utf16Offset::range_from_platform(range));
        Some(shaped.span(shaped.line.x_for_index(range.start), shaped.line.x_for_index(range.end)))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.utf16_offset(self.offset_at(position)).to_platform())
    }
}

/// `text` as a field shows it, with no caret and no keys. It is as wide as a field with the same text, so a field and
/// its text can take turns in one place.
pub(crate) fn label(text: impl Into<SharedString>) -> impl IntoElement {
    Line(Source::Text(text.into()))
}

/// The element that draws the text of a field, or a plain text the same way.
struct Line(Source);

enum Source {
    Field(Entity<TextField>),
    Text(SharedString),
}

impl IntoElement for Line {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Line {
    type RequestLayoutState = Shaped;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let style = window.text_style();
        let (text, color) = match &self.0 {
            Source::Field(field) => match field.read(cx) {
                field if field.text.is_empty() => (field.placeholder.clone(), cx.theme().muted_foreground),
                field => (SharedString::from(field.text.clone()), style.color),
            },
            Source::Text(text) => (text.clone(), style.color),
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let mut run = style.to_run(text.len());
        run.color = color;
        let line = window.text_system().shape_line(text, font_size, &[run], None);
        // The room after the text holds the caret. Rounding up keeps the text in the bounds that the layout rounds to
        // device pixels.
        let scale = window.scale_factor();
        let width = px((f32::from(line.width()) * scale).ceil() / scale) + CARET_WIDTH;
        let mut layout = Style::default();
        layout.size.width = width.into();
        layout.size.height = line_height.into();
        layout.min_size.width = px(0.).into();
        layout.flex_shrink = 1.;
        let id = window.request_layout(layout, [], cx);
        let shaped = Shaped {
            line,
            origin: Point::default(),
            line_height,
        };
        (id, shaped)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        shaped: &mut Self::RequestLayoutState,
        (): &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let entity = match &self.0 {
            Source::Field(entity) => entity.clone(),
            Source::Text(_) => {
                shaped
                    .line
                    .paint(bounds.origin, shaped.line_height, TextAlign::Left, None, window, cx)
                    .ok();
                return;
            }
        };
        let theme = cx.theme();
        let (selection_color, caret_color) = (theme.selection, theme.caret);
        let field = entity.read(cx);
        let focus = field.focus.clone();
        let focused = focus.is_focused(window);
        let selection = field.selection();
        let empty = field.text.is_empty();
        let caret = if empty {
            px(0.)
        } else {
            shaped.line.x_for_index(field.head)
        };
        let width = if empty { px(0.) } else { shaped.line.width() };
        let view = bounds.size.width - CARET_WIDTH;
        // The caret stays in view, and the text never scrolls past its end.
        let mut scroll = field.scroll;
        if caret - scroll > view {
            scroll = caret - view;
        }
        scroll = scroll.min(caret).min(width - view).max(px(0.));
        shaped.origin = bounds.origin - point(scroll, px(0.));
        let x = |at: usize| shaped.line.x_for_index(at);
        let selected = (!empty && !selection.is_empty()).then(|| shaped.span(x(selection.start), x(selection.end)));
        let shaped = shaped.clone();
        entity.update(cx, |field, _| {
            field.scroll = scroll;
            field.shaped = Some(shaped.clone());
        });
        window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(selected) = selected.filter(|_| focused) {
                window.paint_quad(fill(selected, selection_color));
            }
            shaped
                .line
                .paint(shaped.origin, shaped.line_height, TextAlign::Left, None, window, cx)
                .ok();
            if focused && selected.is_none() {
                window.paint_quad(fill(shaped.span(caret, caret + CARET_WIDTH), caret_color));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{TestAppContext, VisualTestContext};

    use super::*;

    fn field<'a>(cx: &'a mut TestAppContext, text: &str) -> (Entity<TextField>, &'a mut VisualTestContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::shortcuts::bind(cx);
        });
        let (field, cx) = cx.add_window_view(|window, cx| TextField::new("f", "F", text, window, cx));
        cx.update(|window, cx| window.focus(&field.focus_handle(cx), cx));
        (field, cx)
    }

    #[gpui_kit::test]
    fn typing_replaces_the_selection_and_keys_edit_the_text(cx: &mut TestAppContext) {
        let (field, cx) = field(cx, "Plans");
        cx.simulate_input("Goals");
        cx.simulate_keystrokes("left left backspace");
        cx.simulate_input("ö");
        cx.simulate_keystrokes("shift-home delete");
        cx.simulate_input("x");
        cx.simulate_keystrokes("end shift-left");
        cx.simulate_input("S");
        field.read_with(cx, |field, _| assert_eq!(field.text(), "xlS"));
    }

    #[gpui_kit::test]
    fn ctrl_a_and_cut_and_paste_work(cx: &mut TestAppContext) {
        let (field, cx) = field(cx, "a b");
        cx.simulate_keystrokes("end ctrl-a ctrl-x");
        field.read_with(cx, |field, _| assert_eq!(field.text(), ""));
        cx.write_to_clipboard(ClipboardItem::new_string("one\ntwo".to_owned()));
        cx.simulate_keystrokes("ctrl-v");
        field.read_with(cx, |field, _| assert_eq!(field.text(), "one two"));
    }
}
