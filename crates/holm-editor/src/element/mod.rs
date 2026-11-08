//! Draws the editor: the text column, its blocks, the selection and the cursor.

mod layout;
mod theme;

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{
    App, BorderStyle, Bounds, ContentMask, Corners, ElementId, ElementInputHandler, Entity,
    GlobalElementId, ImgResourceLoader, InspectorElementId, IntoElement, LayoutId, PathBuilder,
    Pixels, Point, Resource, Style, TextAlign, Window, WrappedLine, fill, outline, point, px,
    relative, size,
};

pub(crate) use layout::{Hit, ImageSizes, Layout, Scroll};
pub(crate) use theme::Theme;

use crate::Editor;
use crate::model::{LineKind, Mark, Quote, Run};
use layout::{COLUMN_WIDTH, Content, LaidCard, LaidLine, Shaper, lay_out};

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

/// The text column in the middle of `bounds`.
fn column(bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    let width = bounds.size.width.min(COLUMN_WIDTH);
    Bounds::new(
        point(
            bounds.origin.x + (bounds.size.width - width) / 2.,
            bounds.origin.y,
        ),
        size(width, bounds.size.height),
    )
}

impl gpui_kit::Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<Rc<Layout>>;

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
        let layout = self.editor.update(cx, |editor, _| {
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
                window,
            );
            editor.autoscroll = false;
            let layout = Rc::new(Layout {
                version: editor.version,
                bounds: column,
                lines,
            });
            editor.layout = Some(layout.clone());
            layout
        });
        Some(layout)
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
        let Some(layout) = layout.take() else { return };
        let editor = self.editor.read(cx);
        let focus = editor.focus.clone();
        let selection = editor.model.selection();
        let cursor = editor.model.cursor();
        let theme = editor.theme.clone();
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
            for rect in layout.selection_bounds(&selection) {
                window.paint_quad(fill(rect, theme.selection));
            }
            for line in &layout.lines {
                paint_line(line, layout.bounds, &theme, window, cx);
            }
            if focus.is_focused(window)
                && let Some(rect) = layout.cursor_bounds(cursor)
            {
                window.paint_quad(fill(rect, theme.cursor));
            }
        });
    }
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
fn paint_line(
    line: &LaidLine,
    column: Bounds<Pixels>,
    theme: &Theme,
    window: &mut Window,
    cx: &mut App,
) {
    let origin = column.origin + point(px(0.), line.top);
    let text_origin = origin + point(line.left, line.pad);
    match &line.content {
        Content::Text(text) => {
            let runs = &line.display.shown.runs;
            paint_text(text, runs, text_origin, line, theme, window, cx);
        }
        Content::Label(text) => paint_text(text, &[], text_origin, line, theme, window, cx),
        Content::Cells(cells) => {
            let top = (line.height - line.text_height.max(line.line_height)) / 2.;
            for cell in cells {
                let cell_origin = text_origin + point(cell.x + cell.inset, top.max(px(0.)));
                let runs = &cell.shown.shown.runs;
                paint_text(&cell.text, runs, cell_origin, line, theme, window, cx);
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
        paint_card(card, origin, line.line_height, theme, window, cx);
    }
}

/// Paints a text of `line` with the highlights and code backgrounds of its `runs`.
/// A failed paint only loses glyphs in this frame.
fn paint_text(
    text: &WrappedLine,
    runs: &[Run],
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
    window.paint_quad(
        fill(button, theme.background)
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
