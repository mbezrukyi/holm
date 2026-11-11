//! Draws the editor: the text column, its blocks, the selection and the cursor.

mod layout;
mod theme;

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{
    AnyElement, App, AvailableSpace, BorderStyle, Bounds, ContentMask, Corners, CursorStyle,
    DispatchPhase, ElementId, ElementInputHandler, Entity, FontStyle, FontWeight, GlobalElementId,
    Hitbox, HitboxBehavior, ImgResourceLoader, InspectorElementId, IntoElement, LayoutId,
    MouseMoveEvent, PathBuilder, Pixels, Point, Resource, Style, TextAlign, Window, WrappedLine,
    fill, outline, point, px, relative, size,
};

pub(crate) use layout::{Hit, ImageSizes, Layout, Scroll};
use theme::Place;
pub(crate) use theme::Theme;

use crate::Editor;
use crate::ink;
use crate::model::{LineKind, Mark, Quote, Run};
use layout::{COLUMN_WIDTH, SIDE_MARGIN};
use layout::{Content, LaidCard, LaidLine, Shaper, lay_out, run_font};

/// The width of a quote bar.
const BAR: Pixels = px(3.);

/// The GPUI element that draws an [`Editor`].
pub(crate) struct EditorElement {
    pub(crate) editor: Entity<Editor>,
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// The text column in the middle of `bounds`, with at least [`SIDE_MARGIN`] on each side.
fn column(bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    let width = (bounds.size.width - SIDE_MARGIN * 2.)
        .max(px(0.))
        .min(COLUMN_WIDTH);
    Bounds::new(
        point(
            bounds.origin.x + (bounds.size.width - width) / 2.,
            bounds.origin.y,
        ),
        size(width, bounds.size.height),
    )
}

/// The layout of a frame, the views before and after the text when they are in sight, and the
/// "Open" buttons of the file cards.
pub(crate) struct Prepainted {
    layout: Rc<Layout>,
    start: Option<AnyElement>,
    end: Option<AnyElement>,
    buttons: Vec<Hitbox>,
}

impl gpui_kit::Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<Prepainted>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let style = window.text_style();
        let column = column(bounds);
        let width = size(
            AvailableSpace::Definite(column.size.width),
            AvailableSpace::MinContent,
        );
        let editor = self.editor.read(cx);
        let focus = editor.focus.clone();
        let mut start_view = editor.start.clone().map(IntoElement::into_any_element);
        let mut end_view = editor.end.clone().map(IntoElement::into_any_element);
        let mut height = |view: Option<&mut AnyElement>| {
            view.map_or(px(0.), |view| view.layout_as_root(width, window, cx).height)
        };
        let start_height = height(start_view.as_mut());
        let end_height = height(end_view.as_mut());
        // Nothing else in the editor takes the focus, so it can only be in the start view.
        let focused = focus.is_focused(window);
        let in_start = start_view.is_some() && !focused && focus.contains_focused(window, cx);
        let (layout, start_origin, end_origin) = self.editor.update(cx, |editor, _| {
            editor.follow_focus(in_start, focused);
            let mut shaper = Shaper::new(
                style,
                column.size.width,
                editor.folder.as_deref(),
                &mut editor.image_sizes,
                &mut editor.highlighter,
                &editor.theme,
            );
            let lines = lay_out(
                &editor.model,
                &mut editor.scroll,
                editor.autoscroll,
                &mut shaper,
                column,
                start_height,
                window,
            );
            editor.autoscroll = false;
            let shows_start = editor.scroll.line == 0;
            // Out of sight, the start view still gets drawn, far above the view and clipped. So
            // it keeps the focus and its keys. Its popups stay out of the view too.
            let start_origin = if shows_start {
                column.origin - point(px(0.), editor.scroll.offset)
            } else {
                column.origin - point(px(0.), start_height + column.size.height)
            };
            let shows_end = editor.scroll.line + lines.len() == editor.model.line_count();
            let end_origin = shows_end.then(|| {
                let top = lines.last().map_or(px(0.), |line| line.top + line.height);
                column.origin + point(px(0.), top)
            });
            let mut views = Vec::new();
            if shows_start && start_view.is_some() {
                views.push(Bounds::new(
                    start_origin,
                    size(column.size.width, start_height),
                ));
            }
            if let Some(origin) = end_origin.filter(|_| end_view.is_some()) {
                views.push(Bounds::new(origin, size(column.size.width, end_height)));
            }
            let layout = Rc::new(Layout {
                version: editor.version,
                bounds: column,
                lines,
                views,
            });
            editor.reach_goal_row(&layout);
            editor.layout = Some(layout.clone());
            (layout, start_origin, end_origin)
        });
        let start = start_view.map(|mut start| {
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                start.prepaint_at(start_origin, window, cx);
            });
            start
        });
        let end = end_view.zip(end_origin).map(|(mut end, origin)| {
            end.prepaint_at(origin, window, cx);
            end
        });
        let buttons = card_buttons(&layout, bounds, window);
        Some(Prepainted {
            layout,
            start,
            end,
            buttons,
        })
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        layout: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(Prepainted {
            layout,
            mut start,
            mut end,
            buttons,
        }) = layout.take()
        else {
            return;
        };
        let hovered: Vec<Bounds<Pixels>> = buttons
            .iter()
            .filter(|button| button.is_hovered(window))
            .map(|button| button.bounds)
            .collect();
        for button in &buttons {
            window.set_cursor_style(CursorStyle::PointingHand, button);
        }
        // Draw again when the mouse moves onto a button or off it.
        let shown = hovered.clone();
        window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, _| {
            let changed = buttons
                .iter()
                .any(|button| button.is_hovered(window) != shown.contains(&button.bounds));
            if phase == DispatchPhase::Bubble && changed {
                window.refresh();
            }
        });
        let editor = self.editor.read(cx);
        let focus = editor.focus.clone();
        let selection = editor.model.selection();
        let cursor = editor.model.cursor();
        let row_start = editor.row_start();
        let theme = editor.theme.clone();
        let matches = editor
            .find
            .as_ref()
            .map(|find| layout.match_bounds(&find.matches))
            .unwrap_or_default();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );
        window.paint_quad(fill(bounds, theme.background));
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for line in &layout.lines {
                paint_blocks(line, layout.bounds, &theme, window);
            }
            for rect in matches {
                window.paint_quad(outline(rect, theme.muted, BorderStyle::Solid));
            }
            for rect in layout.selection_bounds(&selection) {
                window.paint_quad(fill(rect, theme.selection));
            }
            for line in &layout.lines {
                paint_line(line, layout.bounds, &theme, &hovered, window, cx);
            }
            if focus.is_focused(window)
                && let Some(rect) = layout.cursor_bounds(cursor, row_start)
            {
                window.paint_quad(fill(rect, theme.cursor));
            }
            if let Some(end) = &mut end {
                end.paint(window, cx);
            }
            // Last, so a popup of the start view shows over the text.
            if let Some(start) = &mut start {
                start.paint(window, cx);
            }
        });
    }
}

/// The hitboxes of the "Open" buttons of the file cards, clipped to the editor `bounds`.
fn card_buttons(layout: &Layout, bounds: Bounds<Pixels>, window: &mut Window) -> Vec<Hitbox> {
    let column = layout.bounds;
    let buttons = layout
        .lines
        .iter()
        .flat_map(|line| {
            let origin = column.origin + point(px(0.), line.top);
            line.cards
                .iter()
                .map(move |card| Bounds::new(origin + card.button.origin, card.button.size))
        })
        .collect::<Vec<_>>();
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        buttons
            .into_iter()
            .map(|button| window.insert_hitbox(button, HitboxBehavior::Normal))
            .collect()
    })
}

/// Paints what lies behind the text: code backgrounds, quote bars, callouts, tables and rules.
fn paint_blocks(line: &LaidLine, column: Bounds<Pixels>, theme: &Theme, window: &mut Window) {
    let origin = column.origin + point(px(0.), line.top);
    let width = column.size.width;
    let full = |left: Pixels| {
        Bounds::new(
            origin + point(left, px(0.)),
            size((width - left).max(px(0.)), line.height),
        )
    };
    for (ix, quote) in line.display.quotes.iter().enumerate() {
        #[allow(clippy::cast_precision_loss, reason = "a small depth")]
        let x = line.quote_step * (ix as f32);
        let color = match quote {
            Quote::Plain => theme.line,
            Quote::Callout(kind) => {
                let colors = theme.callout(*kind);
                window.paint_quad(fill(full(x), colors.background));
                colors.accent
            }
        };
        window.paint_quad(fill(
            Bounds::new(origin + point(x, px(0.)), size(BAR, line.height)),
            color,
        ));
    }
    match &line.display.kind {
        LineKind::Code { .. } => {
            window.paint_quad(fill(full(line.block_left), theme.code_background));
        }
        LineKind::Rule => window.paint_quad(fill(
            Bounds::new(
                origin + point(line.block_left, line.height / 2.),
                size(width - line.block_left, px(1.)),
            ),
            theme.line,
        )),
        LineKind::Row { head, .. } => {
            let Content::Cells(cells) = &line.content else {
                return;
            };
            let right = cells.last().map_or(px(0.), |cell| cell.x + cell.width);
            let table = Bounds::new(origin + point(line.left, px(0.)), size(right, line.height));
            if *head {
                window.paint_quad(fill(table, theme.panel));
                window.paint_quad(fill(
                    Bounds::new(table.origin, size(right, px(1.))),
                    theme.line,
                ));
            }
            window.paint_quad(fill(
                Bounds::new(
                    table.origin + point(px(0.), line.height - px(1.)),
                    size(right, px(1.)),
                ),
                theme.line,
            ));
            for x in std::iter::once(px(0.)).chain(cells.iter().map(|cell| cell.x + cell.width)) {
                let x = x.min(right - px(1.));
                window.paint_quad(fill(
                    Bounds::new(table.origin + point(x, px(0.)), size(px(1.), line.height)),
                    theme.line,
                ));
            }
        }
        _ => {}
    }
}

/// Paints the text, the list marker, the images and the file cards of a line.
/// `hovered` holds the bounds of the card button under the mouse.
fn paint_line(
    line: &LaidLine,
    column: Bounds<Pixels>,
    theme: &Theme,
    hovered: &[Bounds<Pixels>],
    window: &mut Window,
    cx: &mut App,
) {
    let origin = column.origin + point(px(0.), line.top);
    let text_origin = origin + point(line.left, line.pad);
    match &line.content {
        Content::Text(text) => {
            let runs = &line.display.shown.runs;
            paint_text(
                (text, runs, line.place),
                text_origin,
                line,
                theme,
                window,
                cx,
            );
        }
        Content::Label(text) => {
            paint_text(
                (text, &[], line.place),
                text_origin,
                line,
                theme,
                window,
                cx,
            );
        }
        Content::Cells(cells) => {
            let top = (line.height - line.text_height.max(line.line_height)) / 2.;
            for cell in cells {
                let cell_origin = text_origin + point(cell.x + cell.inset, top.max(px(0.)));
                let piece = (&cell.text, cell.shown.shown.runs.as_slice(), cell.place);
                paint_text(piece, cell_origin, line, theme, window, cx);
            }
        }
        Content::Empty => {}
    }
    if let Some(mark) = &line.mark {
        let bounds = Bounds::new(origin + mark.bounds.origin, mark.bounds.size);
        match &mark.mark {
            Mark::Bullet => {
                window.paint_quad(fill(bounds, theme.muted).corner_radii(bounds.size.width / 2.));
            }
            Mark::Number(_) => {
                if let Some(number) = &mark.number {
                    let _ = number.paint(
                        bounds.origin,
                        line.line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
            }
            Mark::Task { checked, .. } => paint_task(bounds, *checked, theme, window),
        }
    }
    for image in &line.images {
        let bounds = Bounds::new(origin + image.bounds.origin, image.bounds.size);
        let resource = Resource::Path(Arc::from(image.path.as_path()));
        match window.use_asset::<ImgResourceLoader>(&resource, cx) {
            Some(Ok(data)) => {
                let _ = window.paint_image(bounds, bounds, Corners::default(), data, 0, false);
            }
            _ => window.paint_quad(fill(bounds, theme.panel)),
        }
    }
    for card in &line.cards {
        paint_card(card, origin, line.line_height, theme, hovered, window, cx);
    }
}

/// Paints a text of `line`, with its runs and its place, and the highlights, code backgrounds
/// and underlines of its runs.
/// A failed paint only loses glyphs in this frame.
fn paint_text(
    (text, runs, place): (&WrappedLine, &[Run], Place),
    origin: Point<Pixels>,
    line: &LaidLine,
    theme: &Theme,
    window: &mut Window,
    cx: &mut App,
) {
    let line_height = line.line_height;
    // A background is a bit lower than the row and a bit wider than its text, so two rows of
    // backgrounds do not touch and the text keeps some room on its sides.
    let height = (line.font_size * 1.3).min(line_height);
    let side = line.font_size * 0.25;
    let mut start = 0;
    for run in runs {
        let end = start + run.len;
        if let Some(color) = theme.text_background(run.look) {
            for (x, y, right) in row_spans(text, start..end, line_height, line.font_size) {
                let bounds = Bounds::from_corners(
                    origin + point(x - side, y + (line_height - height) / 2.),
                    origin + point(right + side, y + (line_height + height) / 2.),
                );
                window.paint_quad(fill(bounds, color).corner_radii(px(3.)));
            }
        }
        start = end;
    }
    let _ = text.paint(origin, line_height, TextAlign::Left, None, window, cx);
    paint_underlines(text, runs, place, origin, line, theme, window);
}

/// Paints the underlines of underlined text and links: thin, a bit light, close below the text
/// line, with a gap where the ink of a letter crosses it, like a browser does.
fn paint_underlines(
    text: &WrappedLine,
    runs: &[Run],
    place: Place,
    origin: Point<Pixels>,
    line: &LaidLine,
    theme: &Theme,
    window: &mut Window,
) {
    let line_height = line.line_height;
    let layout = &text.unwrapped_layout;
    let baseline = (line_height - layout.ascent - layout.descent) / 2. + layout.ascent;
    let offset = (line.font_size * 0.1).max(px(1.5));
    // The room between the line and the ink on each side of a gap.
    let room = (line.font_size * 0.07).max(px(1.));
    // Only ink at the height of the line itself makes a gap, as in a browser.
    let band = ink::Band {
        font_size: f32::from(layout.font_size),
        top: f32::from(offset),
        bottom: f32::from(offset) + 1.,
    };
    let content = text.text.as_ref();
    let mut start = 0;
    for run in runs {
        let end = start + run.len;
        if !(run.look.underline || run.look.link) {
            start = end;
            continue;
        }
        let font = run_font(&line.font, run.look, place);
        let font = ink::FontKey {
            family: font.family.to_string(),
            bold: font.weight == FontWeight::BOLD,
            italic: font.style == FontStyle::Italic,
        };
        let color = theme.text_color(run.look, place).opacity(0.7);
        // Each screen row of the run: its top, and its parts of line from left to right.
        let mut rows: Vec<(Pixels, Vec<(Pixels, Pixels)>)> = Vec::new();
        for (at, c) in content[start..end].char_indices() {
            let at = start + at;
            let Some(here) = text.position_for_index(at, line_height) else {
                continue;
            };
            let right = text
                .position_for_index(at + c.len_utf8(), line_height)
                .filter(|next| next.y == here.y)
                .map_or(here.x + line.font_size * 0.5, |next| next.x);
            if rows.last().is_none_or(|(y, _)| *y != here.y) {
                rows.push((here.y, vec![(here.x, here.x)]));
            }
            let Some((_, parts)) = rows.last_mut() else {
                continue;
            };
            for (left, gap_right) in ink::gaps(&font, c, band) {
                // A tail can reach under the letter before, like the tail of `y`, so the part
                // before the gap can get shorter.
                if let Some(last) = parts.last_mut() {
                    last.1 = last.1.min(here.x + px(left) - room);
                }
                let after = here.x + px(gap_right) + room;
                parts.push((after, after));
            }
            if let Some(last) = parts.last_mut() {
                last.1 = last.1.max(right);
            }
        }
        for (y, parts) in rows {
            for (left, right) in parts {
                if right > left {
                    let bounds = Bounds::new(
                        origin + point(left, y + baseline + offset),
                        size(right - left, px(1.)),
                    );
                    window.paint_quad(fill(bounds, color));
                }
            }
        }
        start = end;
    }
}

/// The left x, the top y and the right x of each screen row that the text in `range` takes.
fn row_spans(
    text: &WrappedLine,
    range: std::ops::Range<usize>,
    line_height: Pixels,
    font_size: Pixels,
) -> Vec<(Pixels, Pixels, Pixels)> {
    let content = text.text.as_ref();
    let mut spans: Vec<(Pixels, Pixels, Pixels)> = Vec::new();
    for (at, c) in content[range.clone()].char_indices() {
        let at = range.start + at;
        let Some(here) = text.position_for_index(at, line_height) else {
            continue;
        };
        let right = text
            .position_for_index(at + c.len_utf8(), line_height)
            .filter(|next| next.y == here.y)
            .map_or(here.x + font_size * 0.5, |next| next.x);
        match spans.last_mut() {
            Some(last) if last.1 == here.y => last.2 = right,
            _ => spans.push((here.x, here.y, right)),
        }
    }
    spans
}

/// Paints a file card. `origin` is the top left of its line.
fn paint_card(
    card: &LaidCard,
    origin: Point<Pixels>,
    line_height: Pixels,
    theme: &Theme,
    hovered: &[Bounds<Pixels>],
    window: &mut Window,
    cx: &mut App,
) {
    let bounds = Bounds::new(origin + card.bounds.origin, card.bounds.size);
    window.paint_quad(
        fill(bounds, theme.panel)
            .corner_radii(px(6.))
            .border_widths(px(1.))
            .border_color(theme.line),
    );
    let unit = bounds.size.height / 3.2;
    let icon_width = (card.tag.width() + unit * 0.8).max(unit * 1.6);
    let icon = Bounds::new(
        bounds.origin + point(unit * 0.7, unit * 0.6),
        size(icon_width, unit * 2.),
    );
    window.paint_quad(fill(icon, theme.line).corner_radii(px(4.)));
    let tag_origin = icon.origin
        + point(
            (icon.size.width - card.tag.width()) / 2.,
            (icon.size.height - line_height) / 2.,
        );
    let _ = card
        .tag
        .paint(tag_origin, line_height, TextAlign::Left, None, window, cx);
    let name_origin = point(
        icon.right() + unit * 0.6,
        bounds.top() + (bounds.size.height - line_height) / 2.,
    );
    let name_room = Bounds::from_corners(
        point(name_origin.x, bounds.top()),
        point(origin.x + card.button.left() - unit * 0.4, bounds.bottom()),
    );
    window.with_content_mask(Some(ContentMask { bounds: name_room }), |window| {
        let _ = card
            .name
            .paint(name_origin, line_height, TextAlign::Left, None, window, cx);
    });
    let button = Bounds::new(origin + card.button.origin, card.button.size);
    let fill_color = if hovered.contains(&button) {
        theme.line
    } else {
        theme.background
    };
    window.paint_quad(
        fill(button, fill_color)
            .corner_radii(px(4.))
            .border_widths(px(1.))
            .border_color(theme.line),
    );
    let open_origin = button.origin
        + point(
            (button.size.width - card.open.width()) / 2.,
            (button.size.height - line_height) / 2.,
        );
    let _ = card
        .open
        .paint(open_origin, line_height, TextAlign::Left, None, window, cx);
}

fn paint_task(bounds: Bounds<Pixels>, checked: bool, theme: &Theme, window: &mut Window) {
    let radius = bounds.size.width * 0.2;
    if !checked {
        window.paint_quad(
            outline(bounds, theme.muted, BorderStyle::Solid)
                .corner_radii(radius)
                .border_widths(px(1.5)),
        );
        return;
    }
    window.paint_quad(fill(bounds, theme.link).corner_radii(radius));
    let side = bounds.size.width;
    let at = |x: f32, y: f32| -> Point<Pixels> { bounds.origin + point(side * x, side * y) };
    let mut check = PathBuilder::stroke(side * 0.13);
    check.move_to(at(0.25, 0.52));
    check.line_to(at(0.43, 0.70));
    check.line_to(at(0.76, 0.32));
    if let Ok(path) = check.build() {
        window.paint_path(path, theme.background);
    }
}
