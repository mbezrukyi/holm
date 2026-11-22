//! A chip is a box between the text pieces. Its icon, name and detail have exact sizes, and its detail has a smaller
//! font. The line wraps with GPUI's [`LineWrapper`], which sees the parts of a chip around its name as fixed-width
//! elements. So a chip only breaks at a space in its name.
//!
//! [`LineWrapper`]: gpui_kit::LineWrapper

use std::ops::{Add, Range, Sub};

use icu_segmenter::LineSegmenter;
use icu_segmenter::options::LineBreakOptions;

use gpui_kit::{
    App, Bounds, Font, IndentAdjustment, LineFragment, LineWrapper, Pixels, Point, ShapedLine, SharedString, Size,
    TextAlign, TextRun, Window, point, px, size,
};

use crate::model::{ScreenOffset, ScreenRange};

/// The space between the edge of a chip and its content, as a part of the text size.
const CHIP_PAD: f32 = 0.45;

/// The side of the icon of a chip, as a part of the text size.
const CHIP_ICON: f32 = 1.07;

/// The space between the icon and the name, as a part of the text size.
const CHIP_ICON_GAP: f32 = 0.35;

/// The space between the name and the detail, as a part of the text size.
const CHIP_DETAIL_GAP: f32 = 0.35;

/// The height of a chip, as a part of the text size. A row lower than that cuts it.
const CHIP_HEIGHT: f32 = 1.8;

/// How far below the baseline the underline of a detail lies, as a part of its size.
const CHIP_UNDERLINE: f32 = 0.2;

/// The size of the detail of a chip, like the file size, as a part of the text size.
const DETAIL_SCALE: f32 = 0.72;

/// The widest the name in a chip gets, as a part of the text size. A longer name loses chars in the middle.
const CHIP_NAME_WIDTH: f32 = 18.;

/// A byte offset into the text that the wrapper sees: the screen text with the name of each chip in place of its screen
/// text, and one byte before and after the name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct WrapOffset(usize);

impl WrapOffset {
    const fn new(bytes: usize) -> Self {
        Self(bytes)
    }

    const fn get(self) -> usize {
        self.0
    }
}

impl Add<usize> for WrapOffset {
    type Output = Self;

    fn add(self, bytes: usize) -> Self {
        Self(self.0 + bytes)
    }
}

/// The bytes from `other` to `self`.
impl Sub for WrapOffset {
    type Output = usize;

    fn sub(self, other: Self) -> usize {
        self.0 - other.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Icon {
    File,
    Folder,
}

/// It takes the place of its screen text.
pub(crate) struct ChipBox {
    /// The screen text of the chip.
    pub(crate) range: Range<ScreenOffset>,
    pub(crate) icon: Icon,
    pub(crate) name: ShapedLine,
    /// A small text after the name, like the file size.
    pub(crate) detail: Option<ShapedLine>,
}

impl ChipBox {
    /// A chip in place of the screen text `range`. Its name has the look of a run, and gets shorter to fit. Its detail
    /// has the look of a run too, in a smaller font. All sizes grow with `font_size`, so a chip fits in a heading too.
    pub(crate) fn new(
        range: Range<ScreenOffset>,
        icon: Icon,
        (name, name_run): (&str, &TextRun),
        detail: Option<(String, &TextRun)>,
        font_size: Pixels,
        window: &Window,
    ) -> Self {
        let text_system = window.text_system();
        let shape = |text: String, run: &TextRun, font_size: Pixels| {
            let run = TextRun {
                len: text.len(),
                ..run.clone()
            };
            text_system.shape_line(text.into(), font_size, &[run], None)
        };
        let name = shorten(name, |name| {
            shape(name.to_owned(), name_run, font_size).width() <= font_size * CHIP_NAME_WIDTH
        });
        Self {
            range,
            icon,
            name: shape(name, name_run, font_size),
            detail: detail.map(|(text, run)| shape(text, run, font_size * DETAIL_SCALE)),
        }
    }
}

/// `name` as it is when it `fits`. Else the longest cut that fits, with `…` in place of chars in the middle. The
/// extension and the few chars before it stay.
fn shorten(name: &str, fits: impl Fn(&str) -> bool) -> String {
    if fits(name) {
        return name.to_owned();
    }
    let chars: Vec<char> = name.chars().collect();
    let tail = name
        .rfind('.')
        .map_or(0, |dot| name[dot..].chars().count())
        .saturating_add(3)
        .min(chars.len());
    let end: String = chars[chars.len() - tail..].iter().collect();
    let head = chars.len() - tail;
    let cut = |keep: usize| -> String { chars[..keep].iter().collect::<String>() + "\u{2026}" + &end };
    // The most chars from the start that still fit.
    let (mut low, mut high) = (0, head);
    while low < high {
        let mid = (low + high).div_ceil(2);
        if fits(&cut(mid)) {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    cut(low)
}

pub(crate) struct FlowLine {
    pub(crate) text: SharedString,
    chips: Vec<ChipBox>,
    rows: Vec<Row>,
    font_size: Pixels,
    ascent: Pixels,
    descent: Pixels,
}

struct Row {
    /// The screen text of the row. The end of a row is also the start of the next row.
    range: Range<ScreenOffset>,
    pieces: Vec<Piece>,
    width: Pixels,
}

enum Piece {
    Text {
        /// Where `line` starts.
        start: ScreenOffset,
        x: Pixels,
        line: ShapedLine,
    },
    Chip(ChipPiece),
}

/// The part of a chip on one row.
struct ChipPiece {
    chip: usize,
    x: Pixels,
    width: Pixels,
    /// The start of the chip, with the icon.
    start: ChipEnd,
    /// The end of the chip, with the detail.
    end: ChipEnd,
    /// The part of the name on this row.
    name: ShapedLine,
}

/// Where an end of a chip lies, for a piece of the chip. A wrap inside the name puts an end on another row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChipEnd {
    ThisRow,
    OtherRow,
}

/// All points are from the top left of the line.
pub(crate) struct ChipPaint<'a> {
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) icon: Option<(Icon, Bounds<Pixels>)>,
    pub(crate) name: (&'a ShapedLine, Point<Pixels>),
    /// The detail, the top left of its text, and its underline.
    pub(crate) detail: Option<(&'a ShapedLine, Point<Pixels>, Bounds<Pixels>)>,
}

enum WrapSegment {
    Text {
        display: Range<ScreenOffset>,
        line: Box<ShapedLine>,
    },
    /// The part of a chip before its name.
    Lead(usize),
    Name(usize),
    /// The part of a chip after its name.
    Tail(usize),
}

impl FlowLine {
    /// Lays out `text` with its `runs` in rows of `wrap_width`. `chips` are in order and take the place of their screen
    /// text. `font` is the main font of the line.
    pub(crate) fn new(
        text: SharedString,
        runs: &[TextRun],
        chips: Vec<ChipBox>,
        font: &Font,
        font_size: Pixels,
        wrap_width: Pixels,
        window: &Window,
    ) -> Self {
        let (mut flow, segments) = Self::unwrapped(text, runs, chips, font, font_size, window);
        flow.wrap_into_rows(&segments, font, wrap_width, window);
        flow
    }

    /// The narrowest and the widest width of the line, as [`Self::new`] takes it.
    ///
    /// The narrowest width is that of the widest piece no row breaks inside: rows of that width break only at the
    /// places the wrapper picks for a break, like a space or after a hyphen. The widest width is that of the line on
    /// one row.
    pub(crate) fn widths(
        text: SharedString,
        runs: &[TextRun],
        chips: Vec<ChipBox>,
        font: &Font,
        font_size: Pixels,
        window: &Window,
    ) -> (Pixels, Pixels) {
        let (mut flow, segments) = Self::unwrapped(text, runs, chips, font, font_size, window);
        let fragments = flow.fragments(&segments);
        // The text the wrapper sees, with a letter for each byte of an element, as it does.
        let mut seen = String::new();
        for fragment in &fragments {
            match fragment {
                LineFragment::Text { text } => seen.push_str(text),
                LineFragment::Element { len_utf8, .. } => {
                    seen.extend(std::iter::repeat_n('a', *len_utf8));
                }
            }
        }
        // The wrapper adds the width of each char of its own font, which can be wider than the shaped text. A piece
        // needs room for both, or the wrapper breaks inside it.
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(font);
        let mut summed = vec![px(0.); seen.len() + 1];
        let mut at = 0;
        let mut sum = px(0.);
        for fragment in &fragments {
            match fragment {
                LineFragment::Text { text } => {
                    for c in text.chars() {
                        summed[at] = sum;
                        sum += text_system.layout_width(font_id, font_size, c);
                        at += c.len_utf8();
                    }
                }
                LineFragment::Element { width, len_utf8 } => {
                    summed[at] = sum;
                    sum += *width;
                    at += len_utf8;
                }
            }
        }
        summed[at] = sum;
        // The wrapper takes no break before the first char that is not a space.
        let lead = seen.len() - seen.trim_start_matches(' ').len();
        let mut narrowest = px(0.);
        let mut start = 0;
        // GPUI's line wrapper breaks with this segmenter and these options, so these are the breaks it can take. The
        // workspace keeps `icu_segmenter` at gpui-pre's version and features.
        let breaks = LineSegmenter::new_for_non_complex_scripts(LineBreakOptions::default())
            .segment_str(&seen)
            .filter(|&at| at > lead || at == seen.len());
        for end in breaks {
            if end <= start {
                continue;
            }
            let piece = flow
                .measure(&segments, WrapOffset::new(start)..WrapOffset::new(end))
                .max(summed[end] - summed[start]);
            narrowest = narrowest.max(piece);
            start = end;
        }
        flow.wrap_into_rows(&segments, font, Pixels::MAX, window);
        (narrowest, flow.width().max(narrowest))
    }

    /// The line with its shaped parts and no rows yet.
    fn unwrapped(
        text: SharedString,
        runs: &[TextRun],
        chips: Vec<ChipBox>,
        font: &Font,
        font_size: Pixels,
        window: &Window,
    ) -> (Self, Vec<WrapSegment>) {
        let text_system = window.text_system();
        let shape = |range: Range<ScreenOffset>| {
            let bytes = range.bytes();
            text_system.shape_line(
                SharedString::from(text[bytes.clone()].to_owned()),
                font_size,
                &slice_runs(runs, &bytes),
                None,
            )
        };
        let end = ScreenOffset::new(text.len());
        let mut segments = Vec::new();
        let mut at = ScreenOffset::default();
        for (ix, chip) in chips.iter().enumerate() {
            if at < chip.range.start {
                segments.push(WrapSegment::Text {
                    display: at..chip.range.start,
                    line: Box::new(shape(at..chip.range.start)),
                });
            }
            segments.extend([WrapSegment::Lead(ix), WrapSegment::Name(ix), WrapSegment::Tail(ix)]);
            at = chip.range.end;
        }
        if at < end {
            segments.push(WrapSegment::Text {
                display: at..end,
                line: Box::new(shape(at..end)),
            });
        }

        let font_id = text_system.resolve_font(font);
        let lines = segments.iter().filter_map(|segment| match segment {
            WrapSegment::Text { line, .. } => Some(line.as_ref()),
            _ => None,
        });
        let lines = lines.chain(chips.iter().map(|chip| &chip.name));
        let (ascent, descent) = lines.fold(
            (
                text_system.ascent(font_id, font_size),
                text_system.descent(font_id, font_size),
            ),
            |(ascent, descent), line| (ascent.max(line.ascent), descent.max(line.descent)),
        );

        let flow = Self {
            text,
            chips,
            rows: Vec::new(),
            font_size,
            ascent,
            descent,
        };
        (flow, segments)
    }

    fn wrap_into_rows(&mut self, segments: &[WrapSegment], font: &Font, wrap_width: Pixels, window: &Window) {
        let mut wrapper = window.text_system().line_wrapper(font.clone(), self.font_size);
        let cuts = self.wrap(segments, &mut wrapper, wrap_width);
        let mut cutter = Cutter::new(segments.len());
        self.rows = cuts
            .windows(2)
            .map(|cut| self.row(segments, cut[0]..cut[1], &mut cutter))
            .collect();
    }

    fn lead_width(&self, start: ChipEnd) -> Pixels {
        let pad = self.font_size * CHIP_PAD;
        match start {
            ChipEnd::ThisRow => pad + self.font_size * (CHIP_ICON + CHIP_ICON_GAP),
            ChipEnd::OtherRow => pad,
        }
    }

    fn tail_width(&self, chip: usize, end: ChipEnd) -> Pixels {
        let pad = self.font_size * CHIP_PAD;
        match &self.chips[chip].detail {
            Some(detail) if end == ChipEnd::ThisRow => self.font_size * CHIP_DETAIL_GAP + detail.width() + pad,
            _ => pad,
        }
    }

    /// The length of `segment` for the wrapper. A part of a chip without text counts as one byte.
    fn segment_len(&self, segment: &WrapSegment) -> usize {
        match segment {
            WrapSegment::Text { display, .. } => display.end - display.start,
            WrapSegment::Lead(_) | WrapSegment::Tail(_) => 1,
            WrapSegment::Name(chip) => self.chips[*chip].name.len(),
        }
    }

    /// The parts of the line for the wrapper. It sees the parts of a chip around its name as elements of a fixed width.
    fn fragments<'a>(&'a self, segments: &'a [WrapSegment]) -> Vec<LineFragment<'a>> {
        segments
            .iter()
            .map(|segment| match segment {
                WrapSegment::Text { display, .. } => LineFragment::text(&self.text[display.bytes()]),
                WrapSegment::Lead(_) => LineFragment::element(self.lead_width(ChipEnd::ThisRow), 1),
                WrapSegment::Name(chip) => LineFragment::text(&self.chips[*chip].name.text),
                WrapSegment::Tail(chip) => LineFragment::element(self.tail_width(*chip, ChipEnd::ThisRow), 1),
            })
            .collect()
    }

    /// The offsets in the wrapper text where rows start, and its length at the end.
    ///
    /// The wrapper adds the width of each char and does not see kerning or other fonts, so each row is checked against
    /// its shaped width, as gpui-base does it.
    fn wrap(&self, segments: &[WrapSegment], wrapper: &mut LineWrapper, wrap_width: Pixels) -> Vec<WrapOffset> {
        let total = WrapOffset::new(segments.iter().map(|segment| self.segment_len(segment)).sum());
        let fragments = self.fragments(segments);
        let width = |range: Range<WrapOffset>| self.measure(segments, range);
        let mut cuts = vec![WrapOffset::default()];
        let mut start = WrapOffset::default();
        // One wrapper pass gives the ends of all rows. Only a row that is too wide by its shaped width starts a new
        // pass, after its new end.
        'pass: while start < total {
            let rest = fragments_from(&fragments, start);
            let mut ends: Vec<WrapOffset> = wrapper
                .wrap_line(&rest, wrap_width, IndentAdjustment::NoIndent)
                .map(|boundary| (start + boundary.ix).min(total))
                .collect();
            ends.push(total);
            for (ix, &end) in ends.iter().enumerate() {
                if end <= start {
                    continue;
                }
                if width(start..end) > wrap_width {
                    // A new wrap of this row: the boundaries of `rest` count from the start of the pass, which is an
                    // earlier row.
                    let row = fragments_from(&fragments, start);
                    start = self.shrink(segments, &row, wrapper, (start, end), total, wrap_width);
                    cuts.push(start);
                    continue 'pass;
                }
                // Kerning can make a rest fit that the wrapper broke.
                let end = if ix + 2 == ends.len() && width(start..total) <= wrap_width {
                    total
                } else {
                    end
                };
                cuts.push(end);
                start = end;
            }
        }
        if total == WrapOffset::default() {
            cuts.push(total);
        }
        cuts
    }

    /// A new end for the row `start..end`, which is too wide. A smaller budget makes the wrapper break earlier. The
    /// step doubles while the break stays. A word wider than the row keeps its row.
    fn shrink(
        &self,
        segments: &[WrapSegment],
        rest: &[LineFragment<'_>],
        wrapper: &mut LineWrapper,
        (start, mut end): (WrapOffset, WrapOffset),
        total: WrapOffset,
        wrap_width: Pixels,
    ) -> WrapOffset {
        let mut budget = wrap_width;
        let mut overflow = self.measure(segments, start..end) - wrap_width;
        let mut step = overflow;
        while overflow > px(0.) {
            budget -= step.max(px(0.5));
            if budget <= px(0.) {
                break;
            }
            let tighter = next_wrap(wrapper, rest, budget, start, total);
            if tighter < end {
                end = tighter;
                overflow = self.measure(segments, start..end) - wrap_width;
                step = overflow;
            } else {
                step *= 2.;
            }
        }
        end
    }

    /// The width of the row of the wrapper text in `range`, as [`Self::row`] lays it out.
    fn measure(&self, segments: &[WrapSegment], range: Range<WrapOffset>) -> Pixels {
        let part = |line: &ShapedLine, low: usize, high: usize| line.x_for_index(high) - line.x_for_index(low);
        let mut width = px(0.);
        let (mut start_row, mut any, mut name) = (ChipEnd::OtherRow, false, px(0.));
        let mut start = WrapOffset::default();
        for segment in segments {
            if start >= range.end && !matches!(segment, WrapSegment::Name(_) | WrapSegment::Tail(_)) {
                break;
            }
            let end = start + self.segment_len(segment);
            let (low, high) = (range.start.max(start), range.end.min(end));
            let inside = low < high;
            let row = if inside { ChipEnd::ThisRow } else { ChipEnd::OtherRow };
            match segment {
                WrapSegment::Text { line, .. } if inside => width += part(line, low - start, high - start),
                WrapSegment::Lead(_) => (start_row, any, name) = (row, inside, px(0.)),
                WrapSegment::Name(chip) if inside => {
                    any = true;
                    name = part(&self.chips[*chip].name, low - start, high - start);
                }
                WrapSegment::Tail(chip) if inside || any => {
                    width += self.lead_width(start_row) + name + self.tail_width(*chip, row);
                }
                _ => {}
            }
            start = end;
        }
        width
    }

    /// The screen offset of the wrapper offset `at`. All of a chip but its end maps to its start.
    fn display_at(&self, segments: &[WrapSegment], at: WrapOffset) -> ScreenOffset {
        let mut start = WrapOffset::default();
        for segment in segments {
            let end = start + self.segment_len(segment);
            if at < end {
                return match segment {
                    WrapSegment::Text { display, .. } => display.start + (at - start),
                    WrapSegment::Lead(chip) | WrapSegment::Name(chip) | WrapSegment::Tail(chip) => {
                        self.chips[*chip].range.start
                    }
                };
            }
            start = end;
        }
        ScreenOffset::new(self.text.len())
    }

    /// The row of the wrapper text in `range`.
    fn row(&self, segments: &[WrapSegment], range: Range<WrapOffset>, cutter: &mut Cutter) -> Row {
        let mut pieces = Vec::new();
        let mut open: Option<ChipPiece> = None;
        let mut x = px(0.);
        let mut start = WrapOffset::default();
        for (ix, segment) in segments.iter().enumerate() {
            let end = start + self.segment_len(segment);
            let (low, high) = (range.start.max(start), range.end.min(end));
            if low < high {
                let chip = match segment {
                    WrapSegment::Text { .. } => None,
                    WrapSegment::Lead(chip) | WrapSegment::Name(chip) | WrapSegment::Tail(chip) => Some(*chip),
                };
                if let Some(piece) = open.take_if(|piece| Some(piece.chip) != chip) {
                    x += self.close(&mut pieces, piece);
                }
                match segment {
                    WrapSegment::Text { display, line } => {
                        let line = cutter.cut(ix, line, low - start..high - start);
                        let width = line.width();
                        pieces.push(Piece::Text {
                            start: display.start + (low - start),
                            x,
                            line,
                        });
                        x += width;
                    }
                    WrapSegment::Lead(chip) | WrapSegment::Name(chip) | WrapSegment::Tail(chip) => {
                        let piece = open.get_or_insert_with(|| ChipPiece {
                            chip: *chip,
                            x,
                            width: px(0.),
                            start: ChipEnd::OtherRow,
                            end: ChipEnd::OtherRow,
                            name: ShapedLine::default(),
                        });
                        match segment {
                            WrapSegment::Lead(_) => piece.start = ChipEnd::ThisRow,
                            WrapSegment::Tail(_) => piece.end = ChipEnd::ThisRow,
                            _ => {
                                piece.name = cutter.cut(ix, &self.chips[*chip].name, low - start..high - start);
                            }
                        }
                    }
                }
            }
            start = end;
        }
        if let Some(piece) = open {
            x += self.close(&mut pieces, piece);
        }
        Row {
            range: self.display_at(segments, range.start)..self.display_at(segments, range.end),
            pieces,
            width: x,
        }
    }

    /// Ends `piece` with its width, and adds it to `pieces`. It returns the width.
    fn close(&self, pieces: &mut Vec<Piece>, mut piece: ChipPiece) -> Pixels {
        piece.width = self.lead_width(piece.start) + piece.name.width() + self.tail_width(piece.chip, piece.end);
        let width = piece.width;
        pieces.push(Piece::Chip(piece));
        width
    }

    /// How far a line with its own ascent and descent moves down, so it sits on the baseline of this line.
    fn shift(&self, line: &ShapedLine) -> Pixels {
        ((self.ascent - self.descent) - (line.ascent - line.descent)) / 2.
    }

    pub(crate) fn font_size(&self) -> Pixels {
        self.font_size
    }

    pub(crate) fn ascent(&self) -> Pixels {
        self.ascent
    }

    pub(crate) fn descent(&self) -> Pixels {
        self.descent
    }

    /// The width of the widest row.
    pub(crate) fn width(&self) -> Pixels {
        self.rows.iter().map(|row| row.width).fold(px(0.), Pixels::max)
    }

    pub(crate) fn size(&self, line_height: Pixels) -> Size<Pixels> {
        size(self.width(), line_height * self.rows.len())
    }

    /// Where the screen offset `index` shows, from the top left of the line. An offset between two rows shows at the
    /// end of the first row, as in [`gpui_kit::WrappedLine`].
    pub(crate) fn position_for_index(&self, index: ScreenOffset, line_height: Pixels) -> Option<Point<Pixels>> {
        let (ix, row) = self.rows.iter().enumerate().find(|(_, row)| index <= row.range.end)?;
        if index < row.range.start {
            return None;
        }
        let y = line_height * ix;
        Some(point(self.x_in(row, index), y))
    }

    fn x_in(&self, row: &Row, index: ScreenOffset) -> Pixels {
        for piece in &row.pieces {
            match piece {
                Piece::Text { start, x, line } => {
                    if *start <= index && index <= *start + line.len() {
                        return *x + line.x_for_index(index - *start);
                    }
                }
                Piece::Chip(piece) => {
                    let range = &self.chips[piece.chip].range;
                    if piece.start == ChipEnd::ThisRow && index == range.start {
                        return piece.x;
                    }
                    if piece.end == ChipEnd::ThisRow && index == range.end {
                        return piece.x + piece.width;
                    }
                }
            }
        }
        if index <= row.range.start { px(0.) } else { row.width }
    }

    /// The screen offset closest to `position`. A point on a chip gives the start of the chip. A point outside the text
    /// gives `Err` with the nearest offset.
    pub(crate) fn closest_index_for_position(
        &self,
        position: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<ScreenOffset, ScreenOffset> {
        let Some((_, row)) = self
            .rows_with_y(line_height)
            .find(|(y, _)| position.y < *y + line_height)
        else {
            return Err(ScreenOffset::new(self.text.len()));
        };
        if position.x < px(0.) {
            return Err(row.range.start);
        }
        for piece in &row.pieces {
            match piece {
                Piece::Text { start, x, line } if position.x < *x + line.width() => {
                    return Ok(*start + line.closest_index_for_x(position.x - *x));
                }
                Piece::Chip(piece) if position.x < piece.x + piece.width => {
                    return Ok(self.chips[piece.chip].range.start);
                }
                _ => {}
            }
        }
        Err(row.range.end)
    }

    /// Paints the text pieces. A failed paint only loses glyphs in this frame.
    pub(crate) fn paint(&self, origin: Point<Pixels>, line_height: Pixels, window: &mut Window, cx: &mut App) {
        for (y, row) in self.rows_with_y(line_height) {
            for piece in &row.pieces {
                if let Piece::Text { x, line, .. } = piece {
                    let at = origin + point(*x, y + self.shift(line));
                    let _ = line.paint(at, line_height, TextAlign::Left, None, window, cx);
                }
            }
        }
    }

    fn rows_with_y(&self, line_height: Pixels) -> impl Iterator<Item = (Pixels, &Row)> {
        self.rows
            .iter()
            .enumerate()
            .map(move |(ix, row)| (line_height * ix, row))
    }

    /// Whether the screen offset `at` is in the screen text of a chip.
    pub(crate) fn in_chip(&self, at: ScreenOffset) -> bool {
        self.chips.iter().any(|chip| chip.range.contains(&at))
    }

    pub(crate) fn chip_paints(&self, line_height: Pixels) -> Vec<ChipPaint<'_>> {
        let font_size = self.font_size;
        let height = (font_size * CHIP_HEIGHT).min(line_height - px(2.));
        let mut paints = Vec::new();
        for (y, row) in self.rows_with_y(line_height) {
            for piece in &row.pieces {
                let Piece::Chip(piece) = piece else { continue };
                let chip = &self.chips[piece.chip];
                let bounds = Bounds::new(
                    point(piece.x, y + (line_height - height) / 2.),
                    size(piece.width, height),
                );
                let pad = font_size * CHIP_PAD;
                let side = font_size * CHIP_ICON;
                let icon = (piece.start == ChipEnd::ThisRow).then(|| {
                    let at = point(piece.x + pad, y + (line_height - side) / 2.);
                    (chip.icon, Bounds::new(at, size(side, side)))
                });
                let name_x = piece.x + self.lead_width(piece.start);
                let name = (&piece.name, point(name_x, y + self.shift(&piece.name)));
                let detail = chip
                    .detail
                    .as_ref()
                    .filter(|_| piece.end == ChipEnd::ThisRow)
                    .map(|detail| {
                        let x = name_x + piece.name.width() + font_size * CHIP_DETAIL_GAP;
                        // In the middle of the row, like the name, so it sits a bit above the baseline of the name.
                        let at = point(x, y + (line_height - detail.ascent - detail.descent) / 2.);
                        let under = at.y + detail.ascent + detail.font_size * CHIP_UNDERLINE;
                        let underline = Bounds::new(point(x, under), size(detail.width(), px(1.)));
                        (detail, at, underline)
                    });
                paints.push(ChipPaint {
                    bounds,
                    icon,
                    name,
                    detail,
                });
            }
        }
        paints
    }
}

/// The part of `runs` in `range` of their text.
fn slice_runs(runs: &[TextRun], range: &Range<usize>) -> Vec<TextRun> {
    let mut sliced = Vec::new();
    let mut start = 0;
    for run in runs {
        let end = start + run.len;
        let (low, high) = (range.start.max(start), range.end.min(end));
        if low < high {
            sliced.push(TextRun {
                len: high - low,
                ..run.clone()
            });
        }
        start = end;
    }
    sliced
}

/// Cuts the rows out of the shaped pieces of a line. It keeps the rest of each piece after its last cut. So rows in
/// order split each piece once per row.
struct Cutter {
    /// For each piece, the start of its rest and the rest, from x zero.
    rests: Vec<Option<(usize, ShapedLine)>>,
}

impl Cutter {
    fn new(pieces: usize) -> Self {
        Self {
            rests: std::iter::repeat_with(|| None).take(pieces).collect(),
        }
    }

    /// The part of `line`, the piece `ix`, in `range`, from x zero.
    fn cut(&mut self, ix: usize, line: &ShapedLine, range: Range<usize>) -> ShapedLine {
        let kept = self.rests[ix].take().filter(|(start, _)| *start == range.start);
        let (start, from) = match &kept {
            Some((start, rest)) => (*start, rest),
            None => (0, line),
        };
        if range.end == line.len() {
            return match kept {
                Some((_, rest)) => rest,
                None if range.start == 0 => line.clone(),
                None => line.split_at(range.start).1,
            };
        }
        let (head, tail) = from.split_at(range.end - start);
        self.rests[ix] = Some((range.end, tail));
        if range.start == start {
            head
        } else {
            head.split_at(range.start - start).1
        }
    }
}

/// The end of the row that starts at `start`, for a budget of `wrap_width`.
fn next_wrap(
    wrapper: &mut LineWrapper,
    fragments: &[LineFragment<'_>],
    wrap_width: Pixels,
    start: WrapOffset,
    total: WrapOffset,
) -> WrapOffset {
    let end = wrapper
        .wrap_line(fragments, wrap_width, IndentAdjustment::NoIndent)
        .next()
        .map_or(total, |boundary| (start + boundary.ix).min(total));
    if end > start { end } else { total }
}

/// `fragments` without their first `skip` bytes. A wrap never falls inside an element.
fn fragments_from<'a>(fragments: &[LineFragment<'a>], skip: WrapOffset) -> Vec<LineFragment<'a>> {
    let skip = skip.get();
    let mut offset = 0;
    let mut rest = Vec::with_capacity(fragments.len());
    for fragment in fragments {
        let len = match fragment {
            LineFragment::Text { text } => text.len(),
            LineFragment::Element { len_utf8, .. } => *len_utf8,
        };
        if offset + len > skip {
            rest.push(match fragment {
                LineFragment::Text { text } => LineFragment::text(&text[skip.saturating_sub(offset)..]),
                LineFragment::Element { width, len_utf8 } => LineFragment::element(*width, *len_utf8),
            });
        }
        offset += len;
    }
    rest
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use gpui_kit::{TestAppContext, TestDispatcher, black, font};

    use super::*;

    const SIZE: Pixels = px(16.);
    const LINE: Pixels = px(20.);

    /// Runs `test` in a window with the real text system and the repo font.
    fn in_window(test: impl FnOnce(&mut Window)) {
        let text_system = gpui_kit::platform::current_platform(true).text_system();
        text_system
            .add_fonts(vec![
                Cow::Borrowed(include_bytes!("../../tests/fonts/FiraMono-Medium.ttf")),
                // A narrower font than Fira Mono, from the app.
                Cow::Borrowed(include_bytes!("../../../holm/fonts/Inter-Regular.ttf")),
            ])
            .expect("the repo font loads");
        let mut app = TestAppContext::build_with_text_system(TestDispatcher::new(0), None, text_system);
        app.add_empty_window().update(|window, _| test(window));
    }

    fn run(len: usize) -> TextRun {
        TextRun {
            len,
            font: font("Fira Mono"),
            color: black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }
    }

    fn shape(text: &str, font_size: Pixels, window: &Window) -> ShapedLine {
        window
            .text_system()
            .shape_line(text.to_owned().into(), font_size, &[run(text.len())], None)
    }

    /// `before`, a chip with `name` and `detail`, then `after`, in rows of `wrap`. It also gives the screen text of the
    /// chip.
    fn flow(
        (before, name, detail, after): (&str, &str, Option<&str>, &str),
        wrap: Pixels,
        window: &Window,
    ) -> (FlowLine, Range<ScreenOffset>) {
        let text = format!("{before}\u{fffc}{after}");
        let range = ScreenOffset::new(before.len())..ScreenOffset::new(text.len() - after.len());
        let look = run(0);
        let chip = ChipBox::new(
            range.clone(),
            Icon::File,
            (name, &look),
            detail.map(|detail| (detail.to_owned(), &look)),
            SIZE,
            window,
        );
        let line = FlowLine::new(
            text.clone().into(),
            &[run(text.len())],
            vec![chip],
            &font("Fira Mono"),
            SIZE,
            wrap,
            window,
        );
        (line, range)
    }

    fn x(line: &FlowLine, index: ScreenOffset) -> Pixels {
        line.position_for_index(index, LINE).expect("in the line").x
    }

    fn close(a: Pixels, b: Pixels) -> bool {
        (a - b).abs() < px(0.01)
    }

    #[test]
    fn a_cutter_gives_the_parts_of_a_split() {
        in_window(|window| {
            let line = shape("one two three four five", SIZE, window);
            let same = |part: &ShapedLine, range: Range<usize>| {
                let (head, _) = line.split_at(range.end);
                let split = head.split_at(range.start).1;
                assert_eq!(part.len(), split.len());
                assert!(close(part.width(), split.width()));
                for at in 0..=part.len() {
                    assert!(close(part.x_for_index(at), split.x_for_index(at)));
                }
            };
            let mut cutter = Cutter::new(1);
            for range in [0..4, 4..8, 8..14, 14..line.len()] {
                same(&cutter.cut(0, &line, range.clone()), range);
            }
            for range in [4..8, 0..4, 14..line.len(), 8..19] {
                same(&cutter.cut(0, &line, range.clone()), range);
            }
            assert!(close(cutter.cut(0, &line, 0..line.len()).width(), line.width()));
        });
    }

    #[test]
    fn a_long_name_loses_chars_in_the_middle() {
        let fits = |name: &str| name.chars().count() <= 12;
        assert_eq!(shorten("short.txt", fits), "short.txt");
        assert_eq!(shorten("aaaaaaaaaaaaaaaa.txt", fits), "aaaa\u{2026}aaa.txt");
        assert_eq!(shorten("abcdefghijklmnopq", fits), "abcdefgh\u{2026}opq");
    }

    #[test]
    fn a_chip_box_has_the_width_of_its_parts() {
        in_window(|window| {
            let (line, chip) = flow(("See ", "report.pdf", Some("48 KB"), " now"), px(600.), window);
            let parts = SIZE * (CHIP_PAD + CHIP_ICON + CHIP_ICON_GAP)
                + shape("report.pdf", SIZE, window).width()
                + SIZE * CHIP_DETAIL_GAP
                + shape("48 KB", SIZE * DETAIL_SCALE, window).width()
                + SIZE * CHIP_PAD;
            assert!(close(x(&line, chip.start), shape("See ", SIZE, window).width()));
            assert!(close(x(&line, chip.end) - x(&line, chip.start), parts));
            let [paint] = line.chip_paints(LINE).try_into().ok().expect("one piece");
            assert!(close(paint.bounds.size.width, parts));
            assert_eq!(line.size(LINE).height, LINE);
        });
    }

    #[test]
    fn a_line_wraps_before_a_chip_that_does_not_fit() {
        in_window(|window| {
            let (one_row, _) = flow(("aaaa ", "report.pdf", None, " b"), px(600.), window);
            let chip_width = one_row.width() - shape("aaaa  b", SIZE, window).width();
            let wrap = chip_width + SIZE * 2.;
            let (line, chip) = flow(("aaaa ", "report.pdf", None, " b"), wrap, window);
            let [paint] = line.chip_paints(LINE).try_into().ok().expect("one piece");
            assert!(paint.icon.is_some());
            assert_eq!(paint.bounds.origin.x, px(0.));
            assert!(paint.bounds.origin.y > LINE);
            assert!(close(paint.bounds.size.width, chip_width));
            assert_eq!(line.position_for_index(chip.end, LINE).map(|at| at.y), Some(LINE));

            // A chip that only just fits a row moves whole to the next row.
            let (line, _) = flow(("aaaa ", "report.pdf", None, " b"), chip_width + px(1.), window);
            let [paint] = line.chip_paints(LINE).try_into().ok().expect("one piece");
            assert!(paint.icon.is_some() && paint.bounds.origin.y > LINE);
        });
    }

    #[test]
    fn a_chip_wraps_at_a_space_in_its_name_with_padding_on_each_piece() {
        in_window(|window| {
            let detail = Some("2 KB");
            let (whole, _) = flow(("", "my report.pdf", detail, ""), px(600.), window);
            let (line, _) = flow(("", "my report.pdf", detail, ""), whole.width() - px(1.), window);
            let [first, last] = line.chip_paints(LINE).try_into().ok().expect("two pieces");
            let pad = SIZE * CHIP_PAD;
            assert!(first.icon.is_some() && first.detail.is_none());
            assert!(last.icon.is_none() && last.detail.is_some());
            assert_eq!(first.name.0.text.as_ref(), "my ");
            assert_eq!(last.name.0.text.as_ref(), "report.pdf");
            assert!(close(
                first.bounds.size.width,
                SIZE * (CHIP_PAD + CHIP_ICON + CHIP_ICON_GAP) + shape("my ", SIZE, window).width() + pad
            ));
            assert!(close(last.name.1.x, pad));
            assert_eq!(line.size(LINE).height, LINE * 2.);
        });
    }

    #[test]
    fn the_caret_and_a_click_after_a_chip_land_on_the_right_offset() {
        in_window(|window| {
            let (line, chip) = flow(("a", "report.pdf", Some("1 KB"), "bc"), px(600.), window);
            let b = shape("b", SIZE, window).width();
            let after = x(&line, chip.end);
            assert!(close(x(&line, chip.end + 1), after + b));
            let click = |x: Pixels| line.closest_index_for_position(point(x, LINE / 2.), LINE);
            assert_eq!(click(after + b * 0.3), Ok(chip.end));
            assert_eq!(click(after + b * 0.7), Ok(chip.end + 1));
            // A click anywhere on the chip gives its start.
            assert_eq!(click(after - px(1.)), Ok(chip.start));
            assert_eq!(click(x(&line, chip.start) + px(1.)), Ok(chip.start));
        });
    }

    /// A line laid out at its narrowest width breaks only between pieces: after a space, after a hyphen, or at a space
    /// in the name of a chip.
    #[test]
    fn a_line_at_its_narrowest_width_breaks_no_word() {
        in_window(|window| {
            let widths = |text: &str, runs: &[TextRun], base: &Font| {
                FlowLine::widths(text.into(), runs, Vec::new(), base, SIZE, window)
            };
            let fira = font("Fira Mono");
            let inter = font("Inter");
            let code = |len: usize| run(len);
            let text = |len: usize| TextRun {
                len,
                font: inter.clone(),
                ..run(0)
            };
            let cases: [(&str, Vec<TextRun>, &Font); 5] = [
                ("Axis", vec![run(4)], &fira),
                ("Property one long word", vec![run(22)], &fira),
                // Code in a wider font than the base font of the wrapper.
                ("see justify-content now", vec![text(4), code(15), text(4)], &inter),
                ("flex-direction", vec![code(14)], &inter),
                // Text in a narrower font than the base font of the wrapper.
                ("Distributes leftover space", vec![text(26)], &fira),
            ];
            for (line_text, runs, base) in &cases {
                let (min, max) = widths(line_text, runs, base);
                assert!(min <= max, "{line_text}");
                let line = FlowLine::new((*line_text).into(), runs, Vec::new(), base, SIZE, min, window);
                for row in &line.rows {
                    let end = row.range.bytes().end;
                    assert!(
                        end == line_text.len() || matches!(&line_text[end - 1..end], " " | "-"),
                        "a row of {min:?} ends in {:?}",
                        &line_text[row.range.bytes()]
                    );
                }
            }
            let (min, _) = widths("Axis", &[run(4)], &fira);
            assert!(close(min, shape("Axis", SIZE, window).width()));

            let detail = Some("2 KB");
            let (whole, _) = flow(("", "my report.pdf", detail, ""), px(600.), window);
            let text = "\u{fffc}";
            let look = run(0);
            let chip = ChipBox::new(
                ScreenOffset::default()..ScreenOffset::new(text.len()),
                Icon::File,
                ("my report.pdf", &look),
                detail.map(|detail| (detail.to_owned(), &look)),
                SIZE,
                window,
            );
            let (min, max) = FlowLine::widths(text.into(), &[run(text.len())], vec![chip], &fira, SIZE, window);
            assert!(close(max, whole.width()));
            assert!(min < max);
            let (line, _) = flow(("", "my report.pdf", detail, ""), min, window);
            let names: Vec<&str> = line
                .chip_paints(LINE)
                .iter()
                .map(|paint| paint.name.0.text.as_ref())
                .collect();
            assert_eq!(names, ["my ", "report.pdf"]);
        });
    }

    /// Code in a wider font than the text makes rows wider than the wrapper thinks. A row after the first one of a wrap
    /// pass that is too wide still breaks between words.
    #[test]
    fn a_too_wide_row_with_code_breaks_between_words() {
        in_window(|window| {
            let text = "items scale with width or height. It uses modern CSS: gap, logical \
                        properties, clamp(), aspect-ratio, dynamic viewport units, and queries.";
            let inter = font("Inter");
            let mut runs = Vec::new();
            let mut at = 0;
            for code in ["gap", "clamp()", "aspect-ratio"] {
                let start = text.find(code).expect("the code is in the text");
                runs.push(TextRun {
                    len: start - at,
                    font: inter.clone(),
                    ..run(0)
                });
                runs.push(run(code.len()));
                at = start + code.len();
            }
            runs.push(TextRun {
                len: text.len() - at,
                font: inter.clone(),
                ..run(0)
            });
            let word = text.find("aspect-ratio").expect("the word is in the text");
            let after_hyphen = word + "aspect-".len();
            for half in 300_u16..1400 {
                let wrap = px(f32::from(half) / 2.);
                let line = FlowLine::new(text.into(), &runs, Vec::new(), &inter, SIZE, wrap, window);
                for row in &line.rows {
                    let end = row.range.bytes().end;
                    assert!(
                        end <= word || end >= word + "aspect-ratio".len() || end == after_hyphen,
                        "a row of {wrap:?} ends in {:?}",
                        &text[word..end]
                    );
                }
            }
        });
    }
}
