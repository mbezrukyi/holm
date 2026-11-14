//! Lays out source lines for one frame, and answers clicks and cursor moves from that layout.
//! Only the visible lines are shaped.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::{
    Bounds, Font, FontStyle, FontWeight, Pixels, Point, ShapedLine, StrikethroughStyle, TextRun,
    TextStyle, Window, point, px, size,
};
use holm_markdown::{Alignment, CalloutKind};

use super::flow::{ChipBox, FlowLine, Icon};
use super::theme::{Place, Theme};
use crate::files::{FileKind, LinkedFiles};
use crate::highlight::{Highlighter, Tokens};
use crate::model::{Cell, Chip, DisplayLine, Embed, LineKind, Look, Mark, Model, Run};

/// Space above and below an image.
pub(crate) const IMAGE_MARGIN: Pixels = px(4.);

/// The widest the text column gets. A wider editor centers the column.
pub(crate) const COLUMN_WIDTH: Pixels = px(680.);

/// The least space between the text column and each side of the editor.
pub(crate) const SIDE_MARGIN: Pixels = px(32.);

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

/// Where the view starts: a source line, and how far its top lies above the view.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Scroll {
    pub(crate) line: usize,
    /// Can be below zero after a scroll up. The next layout fixes it.
    pub(crate) offset: Pixels,
}

/// Where Up or Down puts the cursor.
pub(crate) struct Vertical {
    pub(crate) offset: usize,
    /// The goal x for the next Up or Down.
    pub(crate) x: Pixels,
    /// Whether the offset shows at the start of its row, as in [`LaidLine::caret`].
    pub(crate) row_start: bool,
    /// The offset is in a line that showed no text, like a line with only an image. Its source
    /// shows in the next frame, and only then can the cursor go to `x`.
    pub(crate) hidden: bool,
}

/// The lines of the last frame, for drawing, clicks and up and down.
pub(crate) struct Layout {
    /// The edit count of the text that this layout shows.
    pub(crate) version: u64,
    /// The bounds of the text column.
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) lines: Vec<LaidLine>,
    /// The window bounds of the view before the text, when it is in sight.
    pub(crate) start: Option<Bounds<Pixels>>,
}

/// What a line shows in its text row.
pub(crate) enum Content {
    /// The screen text of the line, which maps to its source.
    Text(FlowLine),
    /// A name in place of the line, like the name of a callout. It maps to the line end.
    Label(FlowLine),
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
}

pub(crate) struct LaidCell {
    pub(crate) shown: Cell,
    pub(crate) text: FlowLine,
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

/// A file size like `48 KB` or `2.1 MB`, with a no-break space.
fn file_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    #[allow(clippy::cast_precision_loss, reason = "one decimal is shown")]
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 999.5 && unit < UNITS.len() - 1 {
        value /= 1000.;
        unit += 1;
    }
    if unit == 0 || value >= 10. {
        format!("{value:.0}\u{a0}{}", UNITS[unit])
    } else {
        format!("{value:.1}\u{a0}{}", UNITS[unit])
    }
}

/// Shapes source lines for one frame.
pub(crate) struct Shaper<'a> {
    style: TextStyle,
    /// The width of the text column.
    width: Pixels,
    files: &'a mut LinkedFiles,
    highlighter: &'a mut Highlighter,
    theme: &'a Theme,
    /// The column widths of the tables in this frame, by the start of each table.
    tables: HashMap<usize, Rc<[Pixels]>>,
}

impl<'a> Shaper<'a> {
    /// A shaper for one frame, with a column of `width`.
    pub(crate) fn new(
        style: TextStyle,
        width: Pixels,
        files: &'a mut LinkedFiles,
        highlighter: &'a mut Highlighter,
        theme: &'a Theme,
    ) -> Self {
        Self {
            style,
            width,
            files,
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
        let display = model.display_line(line, &mut |target| self.embed(target));
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
        for file in &display.images {
            let FileKind::Image(found) = self.files.kind(file) else {
                continue;
            };
            let path = self.files.path(file);
            #[allow(clippy::cast_precision_loss)]
            let image_size = size(px(found.width as f32), px(found.height as f32));
            let fit = (width / image_size.width).min(1.);
            let bounds = Bounds::new(
                point(left, height + IMAGE_MARGIN),
                size(image_size.width * fit, image_size.height * fit),
            );
            height = bounds.bottom() + IMAGE_MARGIN;
            images.push(LaidImage { path, bounds });
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
        let mut code_font = self.style.font();
        code_font.family = CODE_FONT.into();
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
                Content::Label(FlowLine::new(
                    name.into(),
                    &[run],
                    Vec::new(),
                    &self.style.font(),
                    font_size,
                    width,
                    window,
                ))
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
                Content::Text(FlowLine::new(
                    display.shown.text.clone().into(),
                    &runs,
                    Vec::new(),
                    &code_font,
                    font_size * CODE_SCALE,
                    width,
                    window,
                ))
            }
            _ if display.shown.text.is_empty() && !display.images.is_empty() => Content::Empty,
            _ => {
                let runs = self.runs(&display.shown.runs, display);
                let chips = self.chip_boxes(&display.shown.chips, &runs, font_size, window);
                Content::Text(FlowLine::new(
                    display.shown.text.clone().into(),
                    &runs,
                    chips,
                    &self.style.font(),
                    font_size,
                    width,
                    window,
                ))
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
            let display = model.display_line(line, &mut |target| self.embed(target));
            let LineKind::Row { cells, head, .. } = &display.kind else {
                continue;
            };
            for (ix, cell) in cells.iter().enumerate() {
                let runs = self.cell_runs(&cell.shown.runs, *head);
                let chips = self.chip_boxes(&cell.shown.chips, &runs, font_size, window);
                let text = FlowLine::new(
                    cell.shown.text.clone().into(),
                    &runs,
                    chips,
                    &self.style.font(),
                    font_size,
                    Pixels::MAX,
                    window,
                );
                // A little more room, so a rounding error does not wrap the text.
                let width = text.width() + pad + px(2.);
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
        &mut self,
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
            let chips = self.chip_boxes(&cell.shown.chips, &display_runs, font_size, window);
            let text = FlowLine::new(
                cell.shown.text.clone().into(),
                &display_runs,
                chips,
                &self.style.font(),
                font_size,
                inner,
                window,
            );
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

    /// How a link to `target` shows.
    fn embed(&mut self, target: &str) -> Embed {
        match self.files.kind(target) {
            FileKind::Image(_) => Embed::Image,
            _ => Embed::Chip,
        }
    }

    /// The boxes of `chips` in a text with `runs`. A chip name has the look of its screen text.
    fn chip_boxes(
        &mut self,
        chips: &[Chip],
        runs: &[TextRun],
        font_size: Pixels,
        window: &Window,
    ) -> Vec<ChipBox> {
        let detail_run = TextRun {
            len: 0,
            font: self.style.font(),
            color: self.theme.text.opacity(0.7),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        chips
            .iter()
            .map(|chip| {
                let mut at = 0;
                let run = runs
                    .iter()
                    .find(|run| {
                        at += run.len;
                        at > chip.range.start
                    })
                    .unwrap_or(&detail_run);
                let kind = self.files.kind(&chip.target);
                let name = match chip.name.as_str() {
                    "" => Path::new(&chip.target).file_name().map_or_else(
                        || chip.target.as_str().into(),
                        |name| name.to_string_lossy(),
                    ),
                    name => name.into(),
                };
                let detail = match kind {
                    FileKind::File(bytes) => Some((file_size(bytes), &detail_run)),
                    _ => None,
                };
                let icon = if kind == FileKind::Folder {
                    Icon::Folder
                } else {
                    Icon::File
                };
                ChipBox::new(
                    chip.range.clone(),
                    icon,
                    (&name, run),
                    detail,
                    font_size,
                    window,
                )
            })
            .collect()
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
/// `start` is the height of the view before the first line. The scroll counts it as a part of
/// line 0, so `scroll.offset` on line 0 starts at the top of that view.
///
/// It first moves `scroll` to a line start. With `autoscroll`, it then moves it so the cursor is
/// in view.
pub(crate) fn lay_out(
    model: &Model,
    scroll: &mut Scroll,
    autoscroll: bool,
    shaper: &mut Shaper<'_>,
    bounds: Bounds<Pixels>,
    start: Pixels,
    window: &Window,
) -> Vec<LaidLine> {
    let view = bounds.size.height;
    let start_of = |line: usize| if line == 0 { start } else { px(0.) };
    let height =
        |shaper: &mut Shaper<'_>, line| shaper.line(model, line, window).height + start_of(line);
    let lines = fill_view(model, scroll, shaper, view, start, window);
    if !autoscroll {
        return lines;
    }
    let cursor = model.line_of(model.cursor());
    // On line 0 the start view can be cut off while the text still shows. Then nothing moves.
    let text_cut_off = lines.first().is_some_and(|line| line.text_top() < px(0.));
    if cursor < scroll.line || (cursor == scroll.line && text_cut_off) {
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
        let mut top = view - laid.pad - laid.text_height - start_of(cursor);
        let mut line = cursor;
        while line > 0 && top > px(0.) {
            line -= 1;
            top -= height(shaper, line);
        }
        *scroll = Scroll {
            line,
            offset: (-top).max(px(0.)),
        };
    }
    fill_view(model, scroll, shaper, view, start, window)
}

/// The lines that fill a view of height `view`, after a start view of height `start`.
///
/// It moves `scroll` to the start of the first line in view.
fn fill_view(
    model: &Model,
    scroll: &mut Scroll,
    shaper: &mut Shaper<'_>,
    view: Pixels,
    start: Pixels,
    window: &Window,
) -> Vec<LaidLine> {
    let last = model.line_count() - 1;
    let start_of = |line: usize| if line == 0 { start } else { px(0.) };
    scroll.line = scroll.line.min(last);
    // The lines that a scroll up passes. The last one is the new scroll line.
    let mut above = Vec::new();
    while scroll.offset < px(0.) {
        if scroll.line == 0 {
            scroll.offset = px(0.);
            break;
        }
        scroll.line -= 1;
        let line = shaper.line(model, scroll.line, window);
        scroll.offset += line.height + start_of(scroll.line);
        above.push(line);
    }

    let mut lines = Vec::new();
    let mut top = -scroll.offset;
    for ix in scroll.line..=last {
        top += start_of(ix);
        if top >= view {
            break;
        }
        let mut line = above
            .pop()
            .unwrap_or_else(|| shaper.line(model, ix, window));
        let bottom = top + line.height;
        if bottom <= px(0.) && ix < last {
            *scroll = Scroll {
                line: ix + 1,
                offset: -bottom,
            };
        } else {
            line.top = top;
            lines.push(line);
        }
        top = bottom;
    }
    lines
}

impl LaidLine {
    /// The text row that maps to the source, if the line shows one.
    pub(crate) fn text(&self) -> Option<&FlowLine> {
        match &self.content {
            Content::Text(text) => Some(text),
            _ => None,
        }
    }

    /// The top of the text row, from the top of the view.
    pub(crate) fn text_top(&self) -> Pixels {
        self.top + self.pad
    }

    /// Where `offset` shows, from the top left of the text. A wrap inside a word puts the end
    /// of a row and the start of the next row at the same offset. `row_start` picks the start
    /// of the next row there.
    fn caret(&self, offset: usize, row_start: bool) -> Option<Point<Pixels>> {
        let text = self.text()?;
        let index = self.display.to_display(offset);
        let at = text.position_for_index(index, self.line_height)?;
        if row_start && at.x > px(0.) {
            let next = point(px(0.), at.y + self.line_height);
            if text.closest_index_for_position(next, self.line_height) == Ok(index) {
                return Some(next);
            }
        }
        Some(at)
    }

    /// Whether `offset`, which a point on the row at `row_y` gave, shows at the end of the row
    /// above. Then only `row_start` keeps it on its row.
    fn shows_above(&self, offset: usize, row_y: Pixels) -> bool {
        self.caret(offset, false)
            .is_some_and(|at| at.y + self.line_height <= row_y)
    }
}

impl Layout {
    /// Whether a window point lies in the view before the text.
    pub(crate) fn in_view(&self, position: Point<Pixels>) -> bool {
        self.start.is_some_and(|start| start.contains(&position))
    }

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

    /// The source of the `[ ]` of the task box under a window point.
    pub(crate) fn task_at(&self, position: Point<Pixels>) -> Option<Range<usize>> {
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
            return Some(range.clone());
        }
        None
    }

    /// The source offset one text row above or below the cursor, near `goal_x`.
    /// It returns `None` when that row is not in this layout.
    pub(crate) fn vertical(
        &self,
        cursor: usize,
        row_start: bool,
        goal_x: Option<Pixels>,
        down: bool,
    ) -> Option<Vertical> {
        let ix = self
            .lines
            .iter()
            .position(|line| contains(&line.display.source(), cursor))?;
        let line = &self.lines[ix];
        let at = line.caret(cursor, row_start)?;
        let x = goal_x.unwrap_or(line.left + at.x);
        let origin = self.bounds.origin;
        let (line, target) = if down && at.y + line.line_height < line.text_height {
            (
                line,
                point(x, line.text_top() + at.y + line.line_height * 1.5),
            )
        } else if !down && at.y >= line.line_height {
            (
                line,
                point(x, line.text_top() + at.y - line.line_height * 0.5),
            )
        } else {
            let other = if down {
                self.lines[ix + 1..].iter().find(|l| l.height > px(0.))?
            } else {
                self.lines[..ix].iter().rev().find(|l| l.height > px(0.))?
            };
            // A line with no text row: the cursor goes to its start or end.
            if other.text().is_none() {
                let source = other.display.source();
                return Some(Vertical {
                    offset: if down { source.start } else { source.end },
                    x,
                    row_start: false,
                    hidden: true,
                });
            }
            let y = if down {
                other.text_top() + other.line_height * 0.5
            } else {
                other.text_top() + other.text_height - other.line_height * 0.5
            };
            (other, point(x, y))
        };
        let offset = self.offset_at(origin + target)?;
        let row_y = target.y - line.text_top();
        Some(Vertical {
            offset,
            x,
            row_start: line.shows_above(offset, row_y),
            hidden: false,
        })
    }

    /// The source offset at `x` on the first or the last text row of the line with the cursor,
    /// and whether it is the start of its row, as in [`LaidLine::caret`].
    pub(crate) fn row_at(&self, cursor: usize, x: Pixels, first: bool) -> Option<(usize, bool)> {
        let line = self
            .lines
            .iter()
            .find(|line| contains(&line.display.source(), cursor))?;
        line.text()?;
        let row_y = if first {
            px(0.)
        } else {
            line.text_height - line.line_height
        };
        let target = point(x, line.text_top() + row_y + line.line_height * 0.5);
        let offset = self.offset_at(self.bounds.origin + target)?;
        Some((
            offset,
            line.shows_above(offset, row_y + line.line_height * 0.5),
        ))
    }

    /// The source offset at the start or the end of the screen row with the cursor.
    /// It returns `None` when that row is not in this layout.
    /// It also returns whether the offset is the start of its row, as in [`LaidLine::caret`].
    pub(crate) fn row_edge(
        &self,
        cursor: usize,
        row_start: bool,
        end: bool,
    ) -> Option<(usize, bool)> {
        let line = self
            .lines
            .iter()
            .find(|line| contains(&line.display.source(), cursor))?;
        let Some(text) = line.text() else {
            let source = line.display.source();
            return Some((if end { source.end } else { source.start }, false));
        };
        let at = line.caret(cursor, row_start)?;
        let x = if end { self.bounds.size.width } else { px(0.) };
        let index = text
            .closest_index_for_position(point(x, at.y), line.line_height)
            .unwrap_or_else(|index| index);
        let offset = line.display.to_source(index);
        Some((offset, line.shows_above(offset, at.y)))
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
    /// `row_start` is as in [`LaidLine::caret`].
    pub(crate) fn cursor_bounds(&self, cursor: usize, row_start: bool) -> Option<Bounds<Pixels>> {
        let line = self
            .lines
            .iter()
            .find(|line| line.display.raw && contains(&line.display.source(), cursor))?;
        let at = line.caret(cursor, row_start)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_sizes_have_a_unit_and_at_most_one_decimal() {
        assert_eq!(file_size(512), "512\u{a0}B");
        assert_eq!(file_size(48_000), "48\u{a0}KB");
        assert_eq!(file_size(2_140_000), "2.1\u{a0}MB");
        assert_eq!(file_size(999_999), "1.0\u{a0}MB");
    }
}
