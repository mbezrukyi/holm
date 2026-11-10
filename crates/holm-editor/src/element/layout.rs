//! Lays out source lines for one frame, and answers clicks and cursor moves from that layout.
//! Only the visible lines are shaped.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::{
    Bounds, Font, FontStyle, FontWeight, Pixels, Point, ShapedLine, Size, StrikethroughStyle,
    TextRun, TextStyle, Window, WrappedLine, point, px, size,
};
use holm_markdown::{Alignment, CalloutKind};

use super::theme::{Place, Theme};
use crate::highlight::{Highlighter, Tokens};
use crate::model::{Cell, DisplayLine, LineKind, Look, Mark, Model, Run};

/// Space above and below an image or a file card.
pub(crate) const IMAGE_MARGIN: Pixels = px(4.);

/// The widest the text column gets. A wider editor centers the column.
pub(crate) const COLUMN_WIDTH: Pixels = px(680.);

/// The font of code blocks and inline code.
pub(crate) const CODE_FONT: &str = "Adwaita Mono";

/// The size of code text, as a part of the text size.
const CODE_SCALE: f32 = 0.9;

/// The names that callouts show in place of their `> [!note]` line.
fn callout_name(kind: CalloutKind) -> &'static str {
    match kind {
        CalloutKind::Note => "Note",
        CalloutKind::Tip => "Tip",
        CalloutKind::Important => "Important",
        CalloutKind::Warning => "Warning",
        CalloutKind::Caution => "Caution",
    }
}

/// The text of the button on a file card.
const OPEN: &str = "Open";

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
    /// The bounds of the text column.
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) lines: Vec<LaidLine>,
}

/// What a line shows in its text row.
pub(crate) enum Content {
    /// The screen text of the line, which maps to its source.
    Text(WrappedLine),
    /// A name in place of the line, like the name of a callout. It maps to the line end.
    Label(WrappedLine),
    /// The cells of a table row.
    Cells(Vec<LaidCell>),
    /// No text row, like a line with only embedded files on it.
    Empty,
}

/// One source line, laid out. All bounds start at the left of the column and the top of the line.
pub(crate) struct LaidLine {
    pub(crate) display: DisplayLine,
    pub(crate) content: Content,
    /// The top of the line, from the top of the view.
    pub(crate) top: Pixels,
    /// The space above the text row.
    pub(crate) pad: Pixels,
    /// The text size of the line.
    pub(crate) font_size: Pixels,
    /// The font of the plain text of the line.
    pub(crate) font: Font,
    /// Where the text of the line sits.
    pub(crate) place: Place,
    /// The space that each quote around the line takes on the left.
    pub(crate) quote_step: Pixels,
    /// Where quote bars end and blocks like code start.
    pub(crate) block_left: Pixels,
    /// Where the text starts.
    pub(crate) left: Pixels,
    pub(crate) line_height: Pixels,
    pub(crate) text_height: Pixels,
    pub(crate) height: Pixels,
    pub(crate) mark: Option<LaidMark>,
    pub(crate) images: Vec<LaidImage>,
    pub(crate) cards: Vec<LaidCard>,
}

pub(crate) struct LaidCell {
    pub(crate) shown: Cell,
    pub(crate) text: WrappedLine,
    /// The left edge of the cell.
    pub(crate) x: Pixels,
    pub(crate) width: Pixels,
    /// Where the text starts in the cell: the padding plus the move for the alignment.
    pub(crate) inset: Pixels,
    pub(crate) place: Place,
}

pub(crate) struct LaidMark {
    pub(crate) mark: Mark,
    pub(crate) bounds: Bounds<Pixels>,
    /// The shaped number of an ordered list item.
    pub(crate) number: Option<ShapedLine>,
}

pub(crate) struct LaidImage {
    pub(crate) path: PathBuf,
    pub(crate) bounds: Bounds<Pixels>,
}

pub(crate) struct LaidCard {
    pub(crate) path: PathBuf,
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) name: ShapedLine,
    /// The file extension, in capital letters.
    pub(crate) tag: ShapedLine,
    pub(crate) open: ShapedLine,
    pub(crate) button: Bounds<Pixels>,
}

/// A part of the layout that a click acts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Hit {
    /// A task box, with the source of its `[ ]`.
    Task(Range<usize>),
    /// The "Open" button of a file card.
    Open(PathBuf),
}

/// The pixel sizes of image files, read once from their headers. `None` for a file that is not
/// an image.
pub(crate) type ImageSizes = HashMap<PathBuf, Option<Size<u32>>>;

/// Shapes source lines for one frame.
pub(crate) struct Shaper<'a> {
    style: TextStyle,
    /// The width of the text column.
    width: Pixels,
    folder: Option<&'a Path>,
    sizes: &'a mut ImageSizes,
    highlighter: &'a mut Highlighter,
    theme: &'a Theme,
    /// The column widths of the tables in this frame, by the start of each table.
    tables: HashMap<usize, Rc<[Pixels]>>,
}

impl<'a> Shaper<'a> {
    /// A shaper for one frame, with a column of `width`. Relative file links start at `folder`.
    pub(crate) fn new(
        style: TextStyle,
        width: Pixels,
        folder: Option<&'a Path>,
        sizes: &'a mut ImageSizes,
        highlighter: &'a mut Highlighter,
        theme: &'a Theme,
    ) -> Self {
        Self {
            style,
            width,
            folder,
            sizes,
            highlighter,
            theme,
            tables: HashMap::new(),
        }
    }
}

impl Shaper<'_> {
    fn font_size(&self, window: &Window) -> Pixels {
        self.style.font_size.to_pixels(window.rem_size())
    }

    pub(crate) fn line(&mut self, model: &Model, line: usize, window: &Window) -> LaidLine {
        let display = model.display_line(line);
        let base = self.font_size(window);
        let base_height = window.line_height();
        let scale = match display.heading {
            Some(1) => 1.6,
            Some(2) => 1.35,
            Some(3) => 1.15,
            _ => 1.0,
        };
        let font_size = base * scale;
        let line_height = base_height * scale;
        let pad = match display.heading {
            Some(1..=3) => base_height * 0.3,
            _ => px(0.),
        };

        #[allow(clippy::cast_precision_loss, reason = "a small depth")]
        let quote_left = base * (display.quotes.len() as f32);
        #[allow(clippy::cast_precision_loss, reason = "a small depth")]
        let list_left = base * 1.6 * (display.depth as f32);
        let block_left = quote_left + list_left;
        let code = matches!(display.kind, LineKind::Code { .. });
        let left = if code {
            block_left + base * 0.75
        } else if let Some(prefix) = display.prefix.as_ref().filter(|_| display.raw) {
            // The marker hangs to the left, so the text stays where the preview shows it.
            (block_left - self.width_of(&model.text()[prefix.clone()], font_size, window))
                .max(quote_left)
        } else {
            block_left
        };
        let width = (self.width - left).max(px(1.));

        let (content, text_height) =
            self.content(model, &display, font_size, line_height, width, window);
        let mark = display
            .mark
            .clone()
            .map(|mark| self.mark(mark, left, pad, base, line_height, window));

        let mut height = pad + text_height;
        let mut images = Vec::new();
        let mut cards = Vec::new();
        for file in &display.files {
            let path = match self.folder {
                Some(folder) => folder.join(file),
                None => PathBuf::from(file),
            };
            if let Some(found) = self.image_size(&path) {
                #[allow(clippy::cast_precision_loss)]
                let image_size = size(px(found.width as f32), px(found.height as f32));
                let fit = (width / image_size.width).min(1.);
                let bounds = Bounds::new(
                    point(left, height + IMAGE_MARGIN),
                    size(image_size.width * fit, image_size.height * fit),
                );
                height = bounds.bottom() + IMAGE_MARGIN;
                images.push(LaidImage { path, bounds });
            } else {
                let card = self.card(
                    path,
                    point(left, height + IMAGE_MARGIN),
                    width,
                    base,
                    window,
                );
                height = card.bounds.bottom() + IMAGE_MARGIN;
                cards.push(card);
            }
        }
        if let LineKind::Code { fence: true, .. } = display.kind
            && !display.raw
        {
            height = base_height * 0.5;
        }

        let place = line_place(&display);
        LaidLine {
            display,
            content,
            top: px(0.),
            pad,
            quote_step: base,
            font_size,
            font: self.style.font(),
            place,
            block_left,
            left,
            line_height,
            text_height,
            height,
            mark,
            images,
            cards,
        }
    }

    /// The width of `text` in the plain text font.
    fn width_of(&self, text: &str, font_size: Pixels, window: &Window) -> Pixels {
        let run = TextRun {
            len: text.len(),
            font: self.style.font(),
            color: self.theme.muted,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_line(text.to_owned().into(), font_size, &[run], None)
            .width()
    }

    fn content(
        &mut self,
        model: &Model,
        display: &DisplayLine,
        font_size: Pixels,
        line_height: Pixels,
        width: Pixels,
        window: &Window,
    ) -> (Content, Pixels) {
        let shape = |text: &str, runs: &[TextRun], wrap: Pixels, font_size: Pixels| {
            window
                .text_system()
                .shape_text(text.to_owned().into(), font_size, runs, Some(wrap), None)
                .ok()
                .and_then(|lines| lines.into_iter().next())
        };
        let content = match &display.kind {
            LineKind::Code { fence: true, .. } if !display.raw => Content::Empty,
            LineKind::TableRule => Content::Empty,
            LineKind::Rule => return (Content::Empty, line_height),
            LineKind::CalloutTitle(kind) => {
                let name = callout_name(*kind);
                let mut font = self.style.font();
                font.weight = FontWeight::BOLD;
                let run = TextRun {
                    len: name.len(),
                    font,
                    color: self.theme.callout(*kind).accent,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                shape(name, &[run], width, font_size).map_or(Content::Empty, Content::Label)
            }
            LineKind::Row { head, table, cells } => {
                let columns = self.columns(model, table, width, font_size, window);
                return self.row(cells, *head, &columns, font_size, line_height, window);
            }
            LineKind::Code {
                language,
                code,
                fence: false,
            } => {
                let runs = self.code_runs(model, display, language, code);
                shape(&display.shown.text, &runs, width, font_size * CODE_SCALE)
                    .map_or(Content::Empty, Content::Text)
            }
            _ if display.shown.text.is_empty() && !display.files.is_empty() => Content::Empty,
            _ => {
                let runs = self.runs(&display.shown.runs, display);
                shape(&display.shown.text, &runs, width, font_size)
                    .map_or(Content::Empty, Content::Text)
            }
        };
        let height = match &content {
            Content::Text(text) | Content::Label(text) => text.size(line_height).height,
            Content::Cells(_) | Content::Empty => px(0.),
        };
        (content, height)
    }

    /// The text runs of a code line, colored by the tokens of its block.
    fn code_runs(
        &mut self,
        model: &Model,
        display: &DisplayLine,
        language: &str,
        code: &Range<usize>,
    ) -> Vec<TextRun> {
        let source = display.source();
        let text = &display.shown.text;
        let mut font = self.style.font();
        font.family = CODE_FONT.into();
        let run = |len: usize, color| TextRun {
            len,
            font: font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let plain = self.theme.text;
        if text.len() != source.len() || source.start < code.start || source.end > code.end {
            return vec![run(
                text.len(),
                if display.raw { self.theme.muted } else { plain },
            )];
        }
        let tokens: Tokens = self
            .highlighter
            .tokens(language, &model.text()[code.clone()]);
        let start = source.start - code.start;
        let end = source.end - code.start;
        let first = tokens.partition_point(|(range, _)| range.end <= start);
        let mut runs = Vec::new();
        let mut at = start;
        for (range, token) in tokens[first..]
            .iter()
            .take_while(|(range, _)| range.start < end)
        {
            let from = range.start.max(start);
            let to = range.end.min(end);
            if at < from {
                runs.push(run(from - at, plain));
            }
            runs.push(run(to - from, self.theme.token(*token)));
            at = to;
        }
        if at < end {
            runs.push(run(end - at, plain));
        }
        runs
    }

    fn runs(&self, runs: &[Run], display: &DisplayLine) -> Vec<TextRun> {
        self.text_runs(runs, line_place(display))
    }

    /// The text runs of `runs` at `place`.
    fn text_runs(&self, runs: &[Run], place: Place) -> Vec<TextRun> {
        let base = self.style.font();
        runs.iter()
            .map(|run| {
                let color = self.theme.text_color(run.look, place);
                TextRun {
                    len: run.len,
                    font: run_font(&base, run.look, place),
                    color,
                    background_color: None,
                    underline: None,
                    strikethrough: run.look.strikethrough.then_some(StrikethroughStyle {
                        thickness: px(1.),
                        color: Some(color),
                    }),
                }
            })
            .collect()
    }

    /// The widths of the columns of `table`, so the table fits in the column.
    fn columns(
        &mut self,
        model: &Model,
        table: &Range<usize>,
        room: Pixels,
        font_size: Pixels,
        window: &Window,
    ) -> Rc<[Pixels]> {
        if let Some(columns) = self.tables.get(&table.start) {
            return columns.clone();
        }
        let pad = font_size;
        let mut natural: Vec<Pixels> = Vec::new();
        for line in model.line_of(table.start)..=model.line_of(table.end) {
            let display = model.display_line(line);
            let LineKind::Row { cells, head, .. } = &display.kind else {
                continue;
            };
            for (ix, cell) in cells.iter().enumerate() {
                let runs = self.cell_runs(&cell.shown.runs, *head);
                // A little more room, so a rounding error does not wrap the text.
                let width = window
                    .text_system()
                    .shape_line(cell.shown.text.clone().into(), font_size, &runs, None)
                    .width()
                    + pad
                    + px(2.);
                if ix < natural.len() {
                    natural[ix] = natural[ix].max(width);
                } else {
                    natural.push(width);
                }
            }
        }
        let total = natural.iter().fold(px(0.), |sum, w| sum + *w);
        let columns: Rc<[Pixels]> = if total > room && total > px(0.) {
            let fit = room / total;
            natural.iter().map(|w| (*w * fit).max(pad * 2.)).collect()
        } else {
            natural.into()
        };
        self.tables.insert(table.start, columns.clone());
        columns
    }

    fn row(
        &self,
        cells: &[Cell],
        head: bool,
        columns: &[Pixels],
        font_size: Pixels,
        line_height: Pixels,
        window: &Window,
    ) -> (Content, Pixels) {
        let pad = font_size * 0.5;
        let mut x = px(0.);
        let mut height = line_height;
        let mut laid = Vec::new();
        for (ix, cell) in cells.iter().enumerate() {
            let width = columns.get(ix).copied().unwrap_or(pad * 4.);
            let inner = (width - pad * 2.).max(px(1.));
            let display_runs = self.cell_runs(&cell.shown.runs, head);
            let text = window
                .text_system()
                .shape_text(
                    cell.shown.text.clone().into(),
                    font_size,
                    &display_runs,
                    Some(inner),
                    None,
                )
                .ok()
                .and_then(|lines| lines.into_iter().next());
            let Some(text) = text else {
                x += width;
                continue;
            };
            let size = text.size(line_height);
            height = height.max(size.height);
            let free = (inner - text.width()).max(px(0.));
            let shift = if size.height > line_height {
                px(0.)
            } else {
                match cell.alignment {
                    Alignment::Right => free,
                    Alignment::Center => free / 2.,
                    Alignment::None | Alignment::Left => px(0.),
                }
            };
            laid.push(LaidCell {
                shown: cell.clone(),
                text,
                x,
                width,
                inset: pad + shift,
                place: cell_place(head),
            });
            x += width;
        }
        (Content::Cells(laid), height + line_height * 0.3)
    }

    /// The text runs of a table cell. A head cell is bold.
    fn cell_runs(&self, runs: &[Run], head: bool) -> Vec<TextRun> {
        self.text_runs(runs, cell_place(head))
    }

    fn mark(
        &self,
        mark: Mark,
        left: Pixels,
        pad: Pixels,
        base: Pixels,
        line_height: Pixels,
        window: &Window,
    ) -> LaidMark {
        let area = base * 1.6;
        let (bounds, number) = match &mark {
            Mark::Bullet => {
                let side = base * 0.35;
                (
                    Bounds::new(
                        point(
                            left - area / 2. - side / 2.,
                            pad + (line_height - side) / 2.,
                        ),
                        size(side, side),
                    ),
                    None,
                )
            }
            Mark::Task { .. } => {
                let side = base * 0.95;
                (
                    Bounds::new(
                        point(
                            left - area / 2. - side / 2. - base * 0.1,
                            pad + (line_height - side) / 2.,
                        ),
                        size(side, side),
                    ),
                    None,
                )
            }
            Mark::Number(text) => {
                let run = TextRun {
                    len: text.len(),
                    font: self.style.font(),
                    color: self.theme.muted,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let shaped =
                    window
                        .text_system()
                        .shape_line(text.clone().into(), base, &[run], None);
                let width = shaped.width();
                (
                    Bounds::new(
                        point(left - width - base * 0.4, pad),
                        size(width, line_height),
                    ),
                    Some(shaped),
                )
            }
        };
        LaidMark {
            mark,
            bounds,
            number,
        }
    }

    fn image_size(&mut self, path: &Path) -> Option<Size<u32>> {
        *self.sizes.entry(path.to_owned()).or_insert_with(|| {
            imagesize::size(path).ok().map(|found| Size {
                width: u32::try_from(found.width).unwrap_or(u32::MAX),
                height: u32::try_from(found.height).unwrap_or(u32::MAX),
            })
        })
    }

    fn card(
        &self,
        path: PathBuf,
        origin: Point<Pixels>,
        width: Pixels,
        base: Pixels,
        window: &Window,
    ) -> LaidCard {
        let line = |text: String, bold: bool, code: bool, color, font_size: Pixels| {
            let mut run_font = self.style.font();
            if bold {
                run_font.weight = FontWeight::BOLD;
            }
            if code {
                run_font.family = CODE_FONT.into();
            }
            let run = TextRun {
                len: text.len(),
                font: run_font,
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .shape_line(text.into(), font_size, &[run], None)
        };
        let name = path
            .file_name()
            .map_or_else(|| path.to_string_lossy(), |name| name.to_string_lossy())
            .into_owned();
        let tag = path
            .extension()
            .map(|ext| ext.to_string_lossy().to_uppercase())
            .unwrap_or_default();
        let tag: String = tag.chars().take(4).collect();
        let height = base * 3.2;
        let bounds = Bounds::new(origin, size(width.min(base * 26.), height));
        let open = line(OPEN.to_owned(), false, false, self.theme.link, base * 0.9);
        let button_size = size(open.width() + base * 1.6, base * 1.8);
        let button = Bounds::new(
            point(
                bounds.right() - button_size.width - base * 0.7,
                bounds.top() + (height - button_size.height) / 2.,
            ),
            button_size,
        );
        LaidCard {
            path,
            bounds,
            name: line(name, true, false, self.theme.text, base),
            tag: line(tag, true, true, self.theme.muted, base * 0.6),
            open,
            button,
        }
    }
}

/// The font of a piece of text with `look` at `place`, from the plain text font `base`.
pub(crate) fn run_font(base: &Font, look: Look, place: Place) -> Font {
    let mut font = base.clone();
    if look.strong || place.bold {
        font.weight = FontWeight::BOLD;
    }
    if look.emphasis {
        font.style = FontStyle::Italic;
    }
    if look.code {
        font.family = CODE_FONT.into();
    }
    font
}

/// Where the text of a line sits.
fn line_place(display: &DisplayLine) -> Place {
    Place {
        bold: display.heading.is_some(),
        quoted: !display.quotes.is_empty(),
        done: matches!(display.mark, Some(Mark::Task { checked: true, .. })),
    }
}

/// Where the text of a table cell sits.
fn cell_place(head: bool) -> Place {
    Place {
        bold: head,
        ..Place::default()
    }
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
        let visible = lines
            .iter()
            .find(|line| contains(&line.display.source(), model.cursor()));
        match visible {
            Some(line) if line.top + line.pad + line.text_height <= view => return lines,
            _ => {}
        }
        // Put the text of the cursor line at the bottom of the view.
        let laid = shaper.line(model, cursor, window);
        let mut top = view - laid.pad - laid.text_height;
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

impl LaidLine {
    /// The text row that maps to the source, if the line shows one.
    pub(crate) fn text(&self) -> Option<&WrappedLine> {
        match &self.content {
            Content::Text(text) => Some(text),
            _ => None,
        }
    }

    /// The top of the text row, from the top of the view.
    pub(crate) fn text_top(&self) -> Pixels {
        self.top + self.pad
    }
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
        let y = (local.y - line.text_top())
            .max(px(0.))
            .min((line.text_height - px(1.)).max(px(0.)));
        match &line.content {
            Content::Text(text) => {
                let index = text
                    .closest_index_for_position(point(local.x - line.left, y), line.line_height)
                    .unwrap_or_else(|index| index);
                Some(line.display.to_source(index))
            }
            Content::Cells(cells) => {
                let x = local.x - line.left;
                let cell = cells
                    .iter()
                    .find(|cell| x < cell.x + cell.width)
                    .or(cells.last())?;
                let inner = point(x - cell.x - cell.inset, y);
                let index = cell
                    .text
                    .closest_index_for_position(inner, line.line_height)
                    .unwrap_or_else(|index| index);
                Some(cell.shown.shown.to_source(index))
            }
            Content::Label(_) | Content::Empty => Some(line.display.source().end),
        }
    }

    /// The task box or the file card button under a window point.
    pub(crate) fn hit(&self, position: Point<Pixels>) -> Option<Hit> {
        let local = position - self.bounds.origin;
        let line = self
            .lines
            .iter()
            .find(|line| line.top <= local.y && local.y < line.top + line.height)?;
        let at = point(local.x, local.y - line.top);
        if let Some(mark) = &line.mark
            && let Mark::Task { range, .. } = &mark.mark
            && mark.bounds.dilate(px(3.)).contains(&at)
        {
            return Some(Hit::Task(range.clone()));
        }
        line.cards
            .iter()
            .find(|card| card.button.contains(&at))
            .map(|card| Hit::Open(card.path.clone()))
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
            .text()?
            .position_for_index(line.display.to_display(cursor), line.line_height)?;
        let x = goal_x.unwrap_or(line.left + at.x);
        let origin = self.bounds.origin;
        let target = if down && at.y + line.line_height < line.text_height {
            point(x, line.text_top() + at.y + line.line_height * 1.5)
        } else if !down && at.y >= line.line_height {
            point(x, line.text_top() + at.y - line.line_height * 0.5)
        } else {
            let other = if down {
                self.lines[ix + 1..].iter().find(|l| l.height > px(0.))?
            } else {
                self.lines[..ix].iter().rev().find(|l| l.height > px(0.))?
            };
            // A line with no text row: the cursor goes to its start or end.
            if other.text().is_none() {
                let source = other.display.source();
                return Some((if down { source.start } else { source.end }, x));
            }
            if down {
                point(x, other.text_top() + other.line_height * 0.5)
            } else {
                point(
                    x,
                    other.text_top() + other.text_height - other.line_height * 0.5,
                )
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
        let Some(text) = line.text() else {
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
            .text()?
            .position_for_index(line.display.to_display(cursor), line.line_height)?;
        // The cursor is as tall as the text, in the middle of its row.
        let height = (line.font_size * 1.3).min(line.line_height);
        let y = line.text_top() + at.y + (line.line_height - height) / 2.;
        Some(Bounds::new(
            self.bounds.origin + point(line.left + at.x, y),
            size(px(2.), height),
        ))
    }

    /// The window rectangles that cover `selection`.
    pub(crate) fn selection_bounds(&self, selection: &Range<usize>) -> Vec<Bounds<Pixels>> {
        self.text_bounds(selection, true)
    }

    /// The rects of the find matches in this layout.
    pub(crate) fn match_bounds(&self, matches: &[Range<usize>]) -> Vec<Bounds<Pixels>> {
        let (Some(first), Some(last)) = (self.lines.first(), self.lines.last()) else {
            return Vec::new();
        };
        let (start, end) = (first.display.source().start, last.display.source().end);
        matches[matches.partition_point(|m| m.end < start)
            ..matches.partition_point(|m| m.start <= end)]
            .iter()
            .flat_map(|m| self.text_bounds(m, false))
            .collect()
    }

    /// The rects of the text in `range`. With `raw_only`, only lines that show their syntax.
    fn text_bounds(&self, selection: &Range<usize>, raw_only: bool) -> Vec<Bounds<Pixels>> {
        let mut rects = Vec::new();
        for line in self
            .lines
            .iter()
            .filter(|line| line.display.raw || !raw_only)
        {
            let Some(text) = line.text() else { continue };
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
            let origin = self.bounds.origin + point(line.left, line.text_top());
            let mut y = from.y;
            while y <= to.y {
                let left = if y == from.y { from.x } else { px(0.) };
                let right = if y == to.y {
                    to.x + tail
                } else {
                    self.bounds.size.width - line.left
                };
                rects.push(Bounds::from_corners(
                    origin + point(left, y),
                    origin + point(right, y + line.line_height),
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
