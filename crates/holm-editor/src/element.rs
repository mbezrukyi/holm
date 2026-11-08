//! Lays out and draws the editor. Only the visible lines are shaped.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{
    App, Bounds, ContentMask, Corners, ElementId, ElementInputHandler, Entity, FontStyle,
    FontWeight, GlobalElementId, Hsla, ImgResourceLoader, InspectorElementId, IntoElement,
    LayoutId, Pixels, Point, Resource, Size, StrikethroughStyle, Style, TextAlign, TextRun,
    TextStyle, UnderlineStyle, Window, WrappedLine, fill, point, px, relative, rgb, rgba, size,
};

use crate::Editor;
use crate::model::{DisplayLine, Highlight, Look, Model};

/// Space above and below an image.
pub(crate) const IMAGE_MARGIN: Pixels = px(4.);

const TEXT: u32 = 0x002b_2520;
const SYNTAX: u32 = 0x00a0_9484;
const LINK: u32 = 0x008a_5a2b;
const CODE_BACKGROUND: u32 = 0xeee7_ddff;
const HIGHLIGHT: u32 = 0xfde6_8aff;
const CURSOR: u32 = 0x008a_5a2b;
const SELECTION: u32 = 0xd9c7_b080;
const IMAGE_LOADING: u32 = 0xeee7_ddff;

/// Where the view starts: a source line, and how far its top lies above the view.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Scroll {
    pub(crate) line: usize,
    /// Can be below zero after a scroll up. The next layout fixes it.
    pub(crate) offset: Pixels,
}

/// The lines of the last frame, for drawing, clicks and up and down.
pub(crate) struct Layout {
    /// The edit count of the text that this layout shows.
    pub(crate) version: u64,
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) lines: Vec<LaidLine>,
}

pub(crate) struct LaidLine {
    pub(crate) display: DisplayLine,
    /// `None` for a line with only embedded files on it.
    pub(crate) text: Option<WrappedLine>,
    /// The top of the line, from the top of the view.
    pub(crate) top: Pixels,
    pub(crate) line_height: Pixels,
    pub(crate) text_height: Pixels,
    pub(crate) height: Pixels,
    pub(crate) images: Vec<LaidImage>,
}

pub(crate) struct LaidImage {
    pub(crate) path: PathBuf,
    /// From the top left corner of the line.
    pub(crate) bounds: Bounds<Pixels>,
}

/// The pixel sizes of image files, read once from their headers.
pub(crate) type ImageSizes = HashMap<PathBuf, Option<Size<u32>>>;

/// Shapes source lines for one frame.
pub(crate) struct Shaper<'a> {
    pub(crate) style: TextStyle,
    pub(crate) width: Pixels,
    pub(crate) folder: Option<&'a Path>,
    pub(crate) sizes: &'a mut ImageSizes,
}

impl Shaper<'_> {
    fn line(&mut self, model: &Model, line: usize, window: &Window) -> LaidLine {
        let display = model.display_line(line);
        let scale = match display.heading {
            Some(1) => 1.6,
            Some(2) => 1.4,
            Some(3) => 1.2,
            _ => 1.0,
        };
        let font_size = self.style.font_size.to_pixels(window.rem_size()) * scale;
        let line_height = window.line_height() * scale;

        let images = self.images(&display.files);
        let text = if display.text.is_empty() && !images.is_empty() {
            None
        } else {
            let runs = self.runs(&display);
            window
                .text_system()
                .shape_text(
                    display.text.clone().into(),
                    font_size,
                    &runs,
                    Some(self.width),
                    None,
                )
                .ok()
                .and_then(|lines| lines.into_iter().next())
        };
        let text_height = text
            .as_ref()
            .map_or(px(0.), |text| text.size(line_height).height);

        let mut height = text_height;
        let images = images
            .into_iter()
            .map(|(path, image)| {
                #[allow(clippy::cast_precision_loss)]
                let image_size = size(px(image.width as f32), px(image.height as f32));
                let scale = (self.width / image_size.width).min(1.);
                let bounds = Bounds::new(
                    point(px(0.), height + IMAGE_MARGIN),
                    size(image_size.width * scale, image_size.height * scale),
                );
                height = bounds.bottom() + IMAGE_MARGIN;
                LaidImage { path, bounds }
            })
            .collect();

        LaidLine {
            display,
            text,
            top: px(0.),
            line_height,
            text_height,
            height,
            images,
        }
    }

    fn images(&mut self, files: &[String]) -> Vec<(PathBuf, Size<u32>)> {
        files
            .iter()
            .filter_map(|file| {
                let path = match self.folder {
                    Some(folder) => folder.join(file),
                    None => PathBuf::from(file),
                };
                let found = *self.sizes.entry(path.clone()).or_insert_with(|| {
                    imagesize::size(&path).ok().map(|found| Size {
                        width: u32::try_from(found.width).unwrap_or(u32::MAX),
                        height: u32::try_from(found.height).unwrap_or(u32::MAX),
                    })
                });
                Some((path, found?))
            })
            .collect()
    }

    fn runs(&self, display: &DisplayLine) -> Vec<TextRun> {
        let base = self.style.font();
        display
            .runs
            .iter()
            .map(|run| {
                let look = run.look;
                let mut font = base.clone();
                if look.strong || display.heading.is_some() {
                    font.weight = FontWeight::BOLD;
                }
                if look.emphasis {
                    font.style = FontStyle::Italic;
                }
                let color = text_color(look);
                TextRun {
                    len: run.len,
                    font,
                    color,
                    background_color: background(look),
                    underline: (look.underline || look.link).then_some(UnderlineStyle {
                        thickness: px(1.),
                        color: Some(color),
                        wavy: false,
                    }),
                    strikethrough: look.strikethrough.then_some(StrikethroughStyle {
                        thickness: px(1.),
                        color: Some(color),
                    }),
                }
            })
            .collect()
    }
}

fn text_color(look: Look) -> Hsla {
    if look.syntax {
        rgb(SYNTAX).into()
    } else if let Some(color) = look.color {
        hex(color)
    } else if look.link {
        rgb(LINK).into()
    } else {
        rgb(TEXT).into()
    }
}

fn background(look: Look) -> Option<Hsla> {
    match look.highlight {
        Highlight::Color(color) => Some(hex(color)),
        Highlight::Default => Some(rgba(HIGHLIGHT).into()),
        Highlight::None => look.code.then(|| rgba(CODE_BACKGROUND).into()),
    }
}

fn hex(color: holm_markdown::Color) -> Hsla {
    rgb(u32::from(color.r) << 16 | u32::from(color.g) << 8 | u32::from(color.b)).into()
}

/// Lays out the lines that fill `bounds`, from `scroll` on.
///
/// It first moves `scroll` to a line start. With `autoscroll`, it then moves it so the cursor is
/// in view.
pub(crate) fn lay_out(
    model: &Model,
    scroll: &mut Scroll,
    autoscroll: bool,
    shaper: &mut Shaper<'_>,
    bounds: Bounds<Pixels>,
    window: &Window,
) -> Vec<LaidLine> {
    let last = model.line_count() - 1;
    let view = bounds.size.height;
    scroll.line = scroll.line.min(last);
    while scroll.offset < px(0.) {
        if scroll.line == 0 {
            scroll.offset = px(0.);
            break;
        }
        scroll.line -= 1;
        scroll.offset += shaper.line(model, scroll.line, window).height;
    }
    while scroll.line < last {
        let height = shaper.line(model, scroll.line, window).height;
        if scroll.offset < height {
            break;
        }
        scroll.offset -= height;
        scroll.line += 1;
    }

    let mut lines = fill_view(model, *scroll, shaper, view, window);
    if !autoscroll {
        return lines;
    }
    let cursor = model.line_of(model.cursor());
    if cursor < scroll.line || (cursor == scroll.line && scroll.offset > px(0.)) {
        *scroll = Scroll {
            line: cursor,
            offset: px(0.),
        };
    } else {
        let visible = lines.iter().find(|line| {
            line.display.source().contains(&model.cursor())
                || line.display.source().end == model.cursor()
        });
        match visible {
            Some(line) if line.top + line.text_height <= view => return lines,
            _ => {}
        }
        // Put the text of the cursor line at the bottom of the view.
        let mut top = view - shaper.line(model, cursor, window).text_height;
        let mut line = cursor;
        while line > 0 && top > px(0.) {
            line -= 1;
            top -= shaper.line(model, line, window).height;
        }
        *scroll = Scroll {
            line,
            offset: (-top).max(px(0.)),
        };
    }
    lines = fill_view(model, *scroll, shaper, view, window);
    lines
}

fn fill_view(
    model: &Model,
    scroll: Scroll,
    shaper: &mut Shaper<'_>,
    view: Pixels,
    window: &Window,
) -> Vec<LaidLine> {
    let mut lines = Vec::new();
    let mut top = -scroll.offset;
    for ix in scroll.line..model.line_count() {
        if top >= view {
            break;
        }
        let mut line = shaper.line(model, ix, window);
        line.top = top;
        top += line.height;
        lines.push(line);
    }
    lines
}

impl Layout {
    /// The source offset under a window point. Image space maps to the last text row above it.
    pub(crate) fn offset_at(&self, position: Point<Pixels>) -> Option<usize> {
        let local = position - self.bounds.origin;
        let line = self
            .lines
            .iter()
            .find(|line| local.y < line.top + line.height)
            .or(self.lines.last())?;
        let Some(text) = &line.text else {
            return Some(line.display.source().end);
        };
        let y = (local.y - line.top)
            .max(px(0.))
            .min(line.text_height - px(1.));
        let index = text
            .closest_index_for_position(point(local.x, y), line.line_height)
            .unwrap_or_else(|index| index);
        Some(line.display.to_source(index))
    }

    /// The source offset one text row above or below the cursor, near `goal_x`.
    /// It returns `None` when that row is not in this layout.
    pub(crate) fn vertical(
        &self,
        cursor: usize,
        goal_x: Option<Pixels>,
        down: bool,
    ) -> Option<(usize, Pixels)> {
        let ix = self
            .lines
            .iter()
            .position(|line| contains(&line.display.source(), cursor))?;
        let line = &self.lines[ix];
        let at = line
            .text
            .as_ref()?
            .position_for_index(line.display.to_display(cursor), line.line_height)?;
        let x = goal_x.unwrap_or(at.x);
        let origin = self.bounds.origin;
        let target = if down && at.y + line.line_height < line.text_height {
            point(x, line.top + at.y + line.line_height * 1.5)
        } else if !down && at.y >= line.line_height {
            point(x, line.top + at.y - line.line_height * 0.5)
        } else {
            let other = if down {
                self.lines.get(ix + 1)?
            } else {
                self.lines[..ix].last()?
            };
            // A line with only files has no row. The cursor goes to its start or end.
            if other.text.is_none() {
                let source = other.display.source();
                return Some((if down { source.start } else { source.end }, x));
            }
            if down {
                point(x, other.top + other.line_height * 0.5)
            } else {
                point(x, other.top + other.text_height - other.line_height * 0.5)
            }
        };
        Some((self.offset_at(origin + target)?, x))
    }

    /// The source offset at the start or the end of the screen row with the cursor.
    /// It returns `None` when that row is not in this layout.
    pub(crate) fn row_edge(&self, cursor: usize, end: bool) -> Option<usize> {
        let line = self
            .lines
            .iter()
            .find(|line| contains(&line.display.source(), cursor))?;
        let Some(text) = &line.text else {
            let source = line.display.source();
            return Some(if end { source.end } else { source.start });
        };
        let at = text.position_for_index(line.display.to_display(cursor), line.line_height)?;
        let x = if end { self.bounds.size.width } else { px(0.) };
        let index = text
            .closest_index_for_position(point(x, at.y), line.line_height)
            .unwrap_or_else(|index| index);
        Some(line.display.to_source(index))
    }

    /// How many rows of the base line height fit in the view, at least one.
    pub(crate) fn rows_in_view(&self) -> usize {
        let line_height = self
            .lines
            .iter()
            .map(|line| line.line_height)
            .reduce(Pixels::min)
            .unwrap_or(self.bounds.size.height);
        let rows = (self.bounds.size.height / line_height).floor();
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a small row count"
        )]
        let rows = rows.max(1.) as usize;
        rows
    }

    /// The window bounds of the cursor, when its line is in this layout.
    pub(crate) fn cursor_bounds(&self, cursor: usize) -> Option<Bounds<Pixels>> {
        let line = self
            .lines
            .iter()
            .find(|line| line.display.raw && contains(&line.display.source(), cursor))?;
        let at = line
            .text
            .as_ref()?
            .position_for_index(line.display.to_display(cursor), line.line_height)?;
        Some(Bounds::new(
            self.bounds.origin + point(at.x, line.top + at.y),
            size(px(2.), line.line_height),
        ))
    }

    /// The window rectangles that cover `selection`.
    fn selection_bounds(&self, selection: &Range<usize>) -> Vec<Bounds<Pixels>> {
        let mut rects = Vec::new();
        for line in self.lines.iter().filter(|line| line.display.raw) {
            let Some(text) = &line.text else { continue };
            let source = line.display.source();
            if selection.end < source.start || selection.start > source.end {
                continue;
            }
            let start = line.display.to_display(selection.start.max(source.start));
            let end = line.display.to_display(selection.end.min(source.end));
            let (Some(from), Some(to)) = (
                text.position_for_index(start, line.line_height),
                text.position_for_index(end, line.line_height),
            ) else {
                continue;
            };
            // A selected line break shows as a little space after the text.
            let tail = if selection.end > source.end {
                line.line_height / 3.
            } else {
                px(0.)
            };
            let mut y = from.y;
            while y <= to.y {
                let left = if y == from.y { from.x } else { px(0.) };
                let right = if y == to.y {
                    to.x + tail
                } else {
                    self.bounds.size.width
                };
                rects.push(Bounds::from_corners(
                    self.bounds.origin + point(left, line.top + y),
                    self.bounds.origin + point(right, line.top + y + line.line_height),
                ));
                y += line.line_height;
            }
        }
        rects
    }
}

/// Whether `offset` is in `range`, with its end.
fn contains(range: &Range<usize>, offset: usize) -> bool {
    range.start <= offset && offset <= range.end
}

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
        let layout = self.editor.update(cx, |editor, _| {
            let mut shaper = Shaper {
                style,
                width: bounds.size.width,
                folder: editor.folder.as_deref(),
                sizes: &mut editor.image_sizes,
            };
            let lines = lay_out(
                &editor.model,
                &mut editor.scroll,
                editor.autoscroll,
                &mut shaper,
                bounds,
                window,
            );
            editor.autoscroll = false;
            let layout = Rc::new(Layout {
                version: editor.version,
                bounds,
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
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for rect in layout.selection_bounds(&selection) {
                window.paint_quad(fill(rect, rgba(SELECTION)));
            }
            for line in &layout.lines {
                paint_line(line, bounds.origin, window, cx);
            }
            if focus.is_focused(window)
                && let Some(rect) = layout.cursor_bounds(cursor)
            {
                window.paint_quad(fill(rect, rgb(CURSOR)));
            }
        });
    }
}

fn paint_line(line: &LaidLine, origin: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let origin = origin + point(px(0.), line.top);
    if let Some(text) = &line.text {
        // A failed paint only loses glyphs in this frame. The next frame tries again.
        let _ = text.paint_background(origin, line.line_height, TextAlign::Left, None, window, cx);
        let _ = text.paint(origin, line.line_height, TextAlign::Left, None, window, cx);
    }
    for image in &line.images {
        let bounds = Bounds::new(origin + image.bounds.origin, image.bounds.size);
        let resource = Resource::Path(Arc::from(image.path.as_path()));
        match window.use_asset::<ImgResourceLoader>(&resource, cx) {
            Some(Ok(data)) => {
                let _ = window.paint_image(bounds, bounds, Corners::default(), data, 0, false);
            }
            _ => window.paint_quad(fill(bounds, rgba(IMAGE_LOADING))),
        }
    }
}
