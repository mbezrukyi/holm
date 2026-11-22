//! Text with path chips. Each path shows in a small box inside the line.
//!
//! The text wraps only at spaces, and a path chip moves with the mark after it, like ".". In one line, a text that is
//! too wide ends in "…". A cut inside a path keeps the chip, and the "…" goes inside its box.

use std::fmt;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    App, AvailableSpace, Bounds, Element, ElementId, Font, GlobalElementId, Hsla, InspectorElementId, IntoElement,
    LayoutId, Pixels, ShapedLine, SharedString, Size, Style, TextAlign, TextRun, Window, fill, point, px, relative,
    size,
};

const CHIP_PAD_X: Pixels = px(5.);
const CHIP_PAD_Y: Pixels = px(1.);
const CHIP_RADIUS: Pixels = px(4.);
const CHIP_FONT_SIZE: Pixels = px(12.5);
const ELLIPSIS: &str = "…";

/// A path shows in a chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Plain,
    Path,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Part {
    text: String,
    kind: Kind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PathText {
    parts: Vec<Part>,
    plain: SharedString,
}

impl PathText {
    pub(crate) fn plain(mut self, text: &str) -> Self {
        match self.parts.last_mut() {
            Some(Part {
                text: last,
                kind: Kind::Plain,
            }) => last.push_str(text),
            _ => self.parts.push(Part {
                text: text.to_owned(),
                kind: Kind::Plain,
            }),
        }
        self.plain = format!("{}{text}", self.plain).into();
        self
    }

    pub(crate) fn path(mut self, path: String) -> Self {
        self.plain = format!("{}{path}", self.plain).into();
        self.parts.push(Part {
            text: path,
            kind: Kind::Path,
        });
        self
    }

    pub(crate) fn plain_text(&self) -> SharedString {
        self.plain.clone()
    }

    pub(crate) fn wrapped(&self) -> PathTextElement {
        PathTextElement {
            text: self.clone(),
            mode: Mode::Wrap,
        }
    }

    /// Cuts the text to its width.
    pub(crate) fn one_line(&self) -> PathTextElement {
        PathTextElement {
            text: self.clone(),
            mode: Mode::OneLine,
        }
    }
}

impl fmt::Display for PathText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.plain)
    }
}

impl From<&str> for PathText {
    fn from(text: &str) -> Self {
        Self::default().plain(text)
    }
}

impl From<String> for PathText {
    fn from(text: String) -> Self {
        Self::default().plain(&text)
    }
}

impl From<SharedString> for PathText {
    fn from(text: SharedString) -> Self {
        Self::default().plain(&text)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Wrap,
    OneLine,
}

/// `x` is from the left of the row, at the left edge of a chip box.
#[derive(Clone, Debug, PartialEq)]
struct Piece {
    x: Pixels,
    text: String,
    kind: Kind,
}

#[derive(Debug, Default, PartialEq)]
struct WrapRow {
    pieces: Vec<Piece>,
    width: Pixels,
}

/// The width of a text in the font of its kind.
type Measure<'a> = &'a dyn Fn(&str, Kind) -> Pixels;

/// With the padding of a chip.
fn piece_width(text: &str, kind: Kind, measure: Measure<'_>) -> Pixels {
    let width = measure(text, kind);
    match kind {
        Kind::Plain => width,
        Kind::Path => width + CHIP_PAD_X * 2.,
    }
}

/// With no `width`, the text stays in one row.
fn lay_out(parts: &[Part], mode: Mode, width: Option<Pixels>, measure: Measure<'_>) -> Vec<WrapRow> {
    let segs: Vec<Piece> = parts
        .iter()
        .map(|part| Piece {
            x: px(0.),
            text: part.text.clone(),
            kind: part.kind,
        })
        .collect();
    match (mode, width) {
        (Mode::Wrap, Some(width)) => wrap(segs, width, measure),
        (Mode::OneLine, Some(width)) => vec![one_line(segs, width, measure)],
        (_, None) => vec![one_line(segs, Pixels::MAX, measure)],
    }
}

/// A plain piece joins a plain piece before it.
fn push(row: &mut WrapRow, piece: Piece, measure: Measure<'_>) {
    let piece = match row.pieces.pop() {
        Some(mut last) if last.kind == Kind::Plain && piece.kind == Kind::Plain => {
            last.text.push_str(&piece.text);
            last
        }
        last => {
            row.pieces.extend(last);
            Piece { x: row.width, ..piece }
        }
    };
    row.width = piece.x + piece_width(&piece.text, piece.kind, measure);
    row.pieces.push(piece);
}

/// The pieces between two breaks. A break is after a space. A chip only breaks inside when it is wider than `width`;
/// then it loses its end to "…".
fn atoms(segs: Vec<Piece>, width: Pixels, measure: Measure<'_>) -> Vec<Vec<Piece>> {
    let mut atoms = Vec::new();
    let mut atom = Vec::new();
    for seg in segs {
        if seg.kind == Kind::Path {
            let text = if piece_width(&seg.text, Kind::Path, measure) > width {
                cut(&seg.text, |text| piece_width(text, Kind::Path, measure) <= width)
            } else {
                seg.text
            };
            atom.push(Piece { text, ..seg });
            continue;
        }
        for word in seg.text.split_inclusive(' ') {
            atom.push(Piece {
                text: word.to_owned(),
                ..seg.clone()
            });
            if word.ends_with(' ') {
                atoms.push(std::mem::take(&mut atom));
            }
        }
    }
    if !atom.is_empty() {
        atoms.push(atom);
    }
    atoms
}

fn wrap(segs: Vec<Piece>, width: Pixels, measure: Measure<'_>) -> Vec<WrapRow> {
    let mut rows = Vec::new();
    let mut row = WrapRow::default();
    for atom in atoms(segs, width, measure) {
        // The space at the end may hang past the edge.
        let shown: Pixels = atom
            .iter()
            .enumerate()
            .map(|(ix, piece)| {
                let text = if ix + 1 == atom.len() {
                    piece.text.trim_end()
                } else {
                    &piece.text
                };
                piece_width(text, piece.kind, measure)
            })
            .fold(px(0.), |sum, width| sum + width);
        if !row.pieces.is_empty() && row.width + shown > width {
            rows.push(std::mem::take(&mut row));
        }
        for piece in atom {
            push(&mut row, piece, measure);
        }
    }
    rows.push(row);
    rows
}

fn one_line(segs: Vec<Piece>, width: Pixels, measure: Measure<'_>) -> WrapRow {
    let mut row = WrapRow::default();
    let total = segs
        .iter()
        .map(|seg| piece_width(&seg.text, seg.kind, measure))
        .fold(px(0.), |sum, width| sum + width);
    if total <= width {
        for seg in segs {
            push(&mut row, seg, measure);
        }
        return row;
    }
    let ellipsis = measure(ELLIPSIS, Kind::Plain);
    for seg in segs {
        let whole = piece_width(&seg.text, seg.kind, measure);
        if row.width + whole + ellipsis <= width {
            push(&mut row, seg, measure);
            continue;
        }
        let room = width - row.width;
        let text = cut(&seg.text, |text| piece_width(text, seg.kind, measure) <= room);
        push(&mut row, Piece { text, ..seg }, measure);
        break;
    }
    row
}

/// The longest start of `text` with "…" at its end that `fits`, or only "…".
fn cut(text: &str, fits: impl Fn(&str) -> bool) -> String {
    let ends: Vec<usize> = text.char_indices().map(|(ix, _)| ix).collect();
    let with_mark = |end: usize| format!("{}{ELLIPSIS}", text[..end].trim_end());
    let (mut low, mut high) = (0, ends.len().saturating_sub(1));
    while low < high {
        let mid = (low + high).div_ceil(2);
        if fits(&with_mark(ends[mid])) {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    with_mark(ends.get(low).copied().unwrap_or(0))
}

#[derive(Clone)]
pub(crate) struct PathTextStyle {
    font: Font,
    font_size: Pixels,
    color: Hsla,
    line_height: Pixels,
    chip_font: Font,
}

impl PathTextStyle {
    fn new(window: &Window, cx: &App) -> Self {
        let style = window.text_style();
        let font = style.font();
        let chip_font = Font {
            family: cx.theme().mono_font_family.clone(),
            ..font.clone()
        };
        Self {
            font,
            font_size: style.font_size.to_pixels(window.rem_size()),
            color: style.color,
            line_height: style.line_height_in_pixels(window.rem_size()),
            chip_font,
        }
    }

    fn shape(&self, text: &str, kind: Kind, window: &Window) -> ShapedLine {
        let (font, font_size) = match kind {
            Kind::Plain => (&self.font, self.font_size),
            Kind::Path => (&self.chip_font, CHIP_FONT_SIZE),
        };
        let run = TextRun {
            len: text.len(),
            font: font.clone(),
            color: self.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_line(text.to_owned().into(), font_size, &[run], None)
    }

    fn lay_out(&self, text: &PathText, mode: Mode, width: Option<Pixels>, window: &Window) -> Vec<WrapRow> {
        let measure = |text: &str, kind: Kind| self.shape(text, kind, window).width();
        lay_out(&text.parts, mode, width, &measure)
    }
}

pub(crate) struct PathTextElement {
    text: PathText,
    mode: Mode,
}

impl IntoElement for PathTextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

pub(crate) struct ShapedRow(Vec<(Pixels, ShapedLine, Kind)>);

impl Element for PathTextElement {
    type RequestLayoutState = ();
    type PrepaintState = (PathTextStyle, Vec<ShapedRow>);

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
        if self.mode == Mode::OneLine {
            style.size.width = relative(1.).into();
        }
        let path_style = PathTextStyle::new(window, cx);
        let text = self.text.clone();
        let mode = self.mode;
        let id = window.request_measured_layout(style, move |known, available, window, _| {
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(width) => Some(width),
                AvailableSpace::MinContent => Some(px(0.)),
                AvailableSpace::MaxContent => None,
            });
            let rows = path_style.lay_out(&text, mode, width, window);
            let widest = rows.iter().map(|row| row.width).fold(px(0.), Pixels::max);
            let height = path_style.line_height * rows.len();
            Size {
                width: known.width.unwrap_or(widest),
                height,
            }
        });
        (id, ())
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
        let path_style = PathTextStyle::new(window, cx);
        let rows = path_style
            .lay_out(&self.text, self.mode, Some(bounds.size.width), window)
            .into_iter()
            .map(|row| {
                ShapedRow(
                    row.pieces
                        .into_iter()
                        .map(|piece| (piece.x, path_style.shape(&piece.text, piece.kind, window), piece.kind))
                        .collect(),
                )
            })
            .collect();
        (path_style, rows)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        (path_style, rows): &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Plain text paints with the metrics of its shaped line, so the chip uses them too.
        let plain = path_style.shape(" ", Kind::Plain, window);
        let (ascent, descent) = (plain.ascent, plain.descent);
        let background = cx.theme().accent;
        let line_height = path_style.line_height;
        for (ix, ShapedRow(pieces)) in rows.iter().enumerate() {
            let top = bounds.origin.y + line_height * ix;
            let baseline = top + (line_height - ascent - descent) / 2. + ascent;
            for (x, line, kind) in pieces {
                let left = bounds.origin.x + *x;
                // A failed paint only loses glyphs in this frame.
                if *kind == Kind::Path {
                    let shift = ((ascent - descent) - (line.ascent - line.descent)) / 2.;
                    let chip_bounds = Bounds::new(
                        point(left, baseline - line.ascent - CHIP_PAD_Y),
                        size(
                            line.width() + CHIP_PAD_X * 2.,
                            line.ascent + line.descent + CHIP_PAD_Y * 2.,
                        ),
                    );
                    window.paint_quad(fill(chip_bounds, background).corner_radii(CHIP_RADIUS));
                    let origin = point(left + CHIP_PAD_X, top + shift);
                    let _ = line.paint(origin, line_height, TextAlign::Left, None, window, cx);
                } else {
                    let origin = point(left, top);
                    let _ = line.paint(origin, line_height, TextAlign::Left, None, window, cx);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 10 px for each char, in both fonts.
    fn measure(text: &str, _kind: Kind) -> Pixels {
        px(10.) * text.chars().count()
    }

    fn rows(text: &PathText, mode: Mode, width: f32) -> Vec<Vec<Piece>> {
        lay_out(&text.parts, mode, Some(px(width)), &measure)
            .into_iter()
            .map(|row| row.pieces)
            .collect()
    }

    fn piece(x: f32, text: &str, kind: Kind) -> Piece {
        Piece {
            x: px(x),
            text: text.to_owned(),
            kind,
        }
    }

    fn missing() -> PathText {
        PathText::default()
            .plain("Not at ")
            .path("~/Gone".to_owned())
            .plain(". Pick one.")
    }

    #[test]
    fn the_plain_text_holds_the_paths() {
        assert_eq!(missing().plain_text(), "Not at ~/Gone. Pick one.");
        assert_eq!(PathText::from("a").plain("b"), PathText::from("ab"));
    }

    #[test]
    fn a_text_that_fits_stays_in_one_row() {
        assert_eq!(
            rows(&missing(), Mode::Wrap, 1000.),
            vec![vec![
                piece(0., "Not at ", Kind::Plain),
                piece(70., "~/Gone", Kind::Path),
                piece(140., ". Pick one.", Kind::Plain),
            ]]
        );
    }

    #[test]
    fn the_mark_after_a_path_wraps_with_the_chip() {
        // "Not at " and the chip fit, but the "." does not.
        assert_eq!(
            rows(&missing(), Mode::Wrap, 145.),
            vec![
                vec![piece(0., "Not at ", Kind::Plain)],
                vec![piece(0., "~/Gone", Kind::Path), piece(70., ". Pick ", Kind::Plain)],
                vec![piece(0., "one.", Kind::Plain)],
            ]
        );
    }

    #[test]
    fn a_path_wider_than_the_row_ends_in_an_ellipsis() {
        assert_eq!(
            rows(&missing(), Mode::Wrap, 60.),
            vec![
                vec![piece(0., "Not at ", Kind::Plain)],
                vec![piece(0., "~/Go…", Kind::Path), piece(60., ". ", Kind::Plain)],
                vec![piece(0., "Pick ", Kind::Plain)],
                vec![piece(0., "one.", Kind::Plain)],
            ]
        );
    }

    #[test]
    fn one_line_cuts_the_plain_text() {
        assert_eq!(
            rows(&missing(), Mode::OneLine, 200.),
            vec![vec![
                piece(0., "Not at ", Kind::Plain),
                piece(70., "~/Gone", Kind::Path),
                piece(140., ". Pic…", Kind::Plain),
            ]]
        );
    }

    #[test]
    fn a_cut_inside_the_path_keeps_the_chip() {
        assert_eq!(
            rows(&missing(), Mode::OneLine, 120.),
            vec![vec![piece(0., "Not at ", Kind::Plain), piece(70., "~/G…", Kind::Path)]]
        );
    }
}
