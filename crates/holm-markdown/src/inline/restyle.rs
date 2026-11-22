//! Format keys. A key makes a flat list of styled text pieces from the touched part of each leaf. It adds or removes
//! one style and builds a tree again. It writes the tree and parses the result to check it. When the check fails, it
//! tries other markers.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use super::parse::{Defs, parse};
use super::{Container, Kind, Leaf, Node};
use super::{html_tag, scan};
use crate::{BlockKind, Body, Color};

/// A format key: the inline style it adds or removes, or the color it sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Ctrl+B: `**text**`.
    Bold,
    /// Ctrl+I: `*text*`.
    Italic,
    /// Ctrl+U: `<u>text</u>`.
    Underline,
    /// Ctrl+D: `~~text~~`.
    Strikethrough,
    /// Ctrl+H: `==text==`. Text with any highlight counts as highlighted, so the key removes it.
    Highlight,
    /// Ctrl+E: `` `text` ``.
    Code,
    /// The text color: `<span style="color: #rrggbb">`. It takes the place of any other text color. `None` removes the
    /// text color.
    TextColor(Option<Color>),
    /// The highlight color: `<mark style="background: #rrggbb">`. It takes the place of any other highlight, also
    /// `==text==`. `None` removes the highlight.
    HighlightColor(Option<Color>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    /// The replaced source.
    pub range: Range<usize>,
    pub text: String,
    /// The selection in the new text. It covers the same visible text as before.
    pub selection: Range<usize>,
}

/// `body` is the parse of `text`.
///
/// The key adds the style when any selected text lacks it. It removes the style when all of it has it. A color key sets
/// its color on all selected text. With no selection, the key works on the word at the cursor. Between words, it puts
/// in an empty pair of markers. It gives `None` when nothing changes, or when no valid Markdown gives the wanted
/// styles.
///
/// ```
/// use holm_markdown::Body;
/// use holm_markdown::inline::{Format, restyle};
///
/// let text = "Write the report";
/// let edit = restyle(text, &Body::parse(text), 0..5, Format::Bold).unwrap();
/// assert_eq!(edit.text, "**Write**");
/// assert_eq!(edit.range, 0..5);
/// ```
pub fn restyle(text: &str, body: &Body, selection: Range<usize>, format: Format) -> Option<Edit> {
    let selection = selection.start.min(text.len())..selection.end.min(text.len());
    if selection.is_empty() {
        return at_cursor(text, body, selection.start, format);
    }
    let changes = change(text, body, &selection, format)?;
    let start = map(&changes, selection.start, End::Start);
    let end = map(&changes, selection.end, End::End);
    let edit = join(text, &changes, start..end);
    keeps_blocks(text, body, &edit).then_some(edit)
}

/// Whether the blocks around `edit` keep their kinds. A marker or escape at the start of a line can start another
/// block, like a code fence.
fn keeps_blocks(text: &str, body: &Body, edit: &Edit) -> bool {
    let blocks = body.blocks();
    let first = blocks.partition_point(|b| b.range.end < edit.range.start);
    let last = blocks.partition_point(|b| b.range.start <= edit.range.end);
    if first >= last {
        return true;
    }
    let old = blocks[first].range.start..blocks[last - 1].range.end;
    let mut new = text[old.start..edit.range.start].to_owned();
    new.push_str(&edit.text);
    new.push_str(&text[edit.range.end..old.end]);
    let (before, after) = (Body::parse(&text[old]), Body::parse(&new));
    before.leaves().len() == after.leaves().len() && same_shape(before.blocks(), after.blocks())
}

fn same_shape(a: &[crate::Block], b: &[crate::Block]) -> bool {
    let kind = |block: &crate::Block| match &block.kind {
        BlockKind::Item { task } => BlockKind::Item {
            task: task.as_ref().map(|t| crate::Task {
                checked: t.checked,
                range: 0..0,
            }),
        },
        other => other.clone(),
    };
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(a, b)| kind(a) == kind(b) && same_shape(&a.children, &b.children))
}

/// A visible style. Markers that look the same are one style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Look {
    Emphasis,
    Strong,
    Strike,
    Highlight(Option<Color>),
    Underline,
    Color(Color),
}

/// Looks that differ only in their color. A piece has at most one look of each family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Color,
    Highlight,
}

impl Family {
    fn has(self, look: Look) -> bool {
        look.family() == Some(self)
    }

    fn look(self, color: Color) -> Look {
        match self {
            Self::Color => Look::Color(color),
            Self::Highlight => Look::Highlight(Some(color)),
        }
    }
}

impl Look {
    fn family(self) -> Option<Family> {
        match self {
            Self::Color(_) => Some(Family::Color),
            Self::Highlight(_) => Some(Family::Highlight),
            _ => None,
        }
    }

    /// Whether `other` is this look, or a look of its family.
    fn is_kin(self, other: Self) -> bool {
        self == other || self.family().is_some_and(|family| family.has(other))
    }

    /// Bold and italic do not.
    fn shows_on_space(self) -> bool {
        !matches!(self, Self::Emphasis | Self::Strong)
    }

    fn of(kind: &Kind) -> Option<Self> {
        Some(match kind {
            Kind::Emphasis => Self::Emphasis,
            Kind::Strong => Self::Strong,
            Kind::Strikethrough => Self::Strike,
            Kind::Highlight(color) => Self::Highlight(*color),
            Kind::Underline => Self::Underline,
            Kind::Color(color) => Self::Color(*color),
            _ => return None,
        })
    }

    /// The markers Holm writes for a new style.
    fn default_spelling(self) -> Spelling {
        match self {
            Self::Emphasis => Spelling::Delim("*"),
            Self::Strong => Spelling::Delim("**"),
            Self::Strike => Spelling::Delim("~~"),
            Self::Highlight(None) => Spelling::Delim("=="),
            Self::Highlight(Some(c)) => tag(&format!("mark style=\"background: {c}\""), "mark"),
            Self::Underline => tag("u", "u"),
            Self::Color(c) => tag(&format!("span style=\"color: {c}\""), "span"),
        }
    }

    /// The HTML tag for a style that Markdown cannot write in a place.
    fn html(self) -> Option<Spelling> {
        match self {
            Self::Emphasis => Some(tag("i", "i")),
            Self::Strong => Some(tag("b", "b")),
            Self::Strike => Some(tag("s", "s")),
            Self::Highlight(None) => Some(tag("mark", "mark")),
            _ => None,
        }
    }
}

impl Format {
    fn look(self) -> Option<Look> {
        match self {
            Self::Bold => Some(Look::Strong),
            Self::Italic => Some(Look::Emphasis),
            Self::Underline => Some(Look::Underline),
            Self::Strikethrough => Some(Look::Strike),
            Self::Highlight => Some(Look::Highlight(None)),
            Self::Code | Self::TextColor(_) | Self::HighlightColor(_) => None,
        }
    }

    /// The empty marker pair, and the length of its first half. A key that only removes has none.
    fn pair(self) -> Option<(String, usize)> {
        let delim = |pair: &str| Some((pair.to_owned(), pair.len() / 2));
        match self {
            Self::Bold => delim("****"),
            Self::Italic => delim("**"),
            Self::Underline => Some(("<u></u>".to_owned(), 3)),
            Self::Strikethrough => delim("~~~~"),
            Self::Highlight => delim("===="),
            Self::Code => delim("``"),
            Self::TextColor(color) => color.map(|c| Look::Color(c).default_spelling().pair()),
            Self::HighlightColor(color) => color.map(|c| Look::Highlight(Some(c)).default_spelling().pair()),
        }
    }
}

fn tag(open: &str, close: &str) -> Spelling {
    Spelling::Tag(format!("<{open}>"), format!("</{close}>"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Spelling {
    Delim(&'static str),
    Tag(String, String),
}

impl Spelling {
    fn of(open: &str, close: &str) -> Self {
        match open {
            "*" => Self::Delim("*"),
            "_" => Self::Delim("_"),
            "**" => Self::Delim("**"),
            "__" => Self::Delim("__"),
            "~" => Self::Delim("~"),
            "~~" => Self::Delim("~~"),
            "==" => Self::Delim("=="),
            _ => Self::Tag(open.to_owned(), close.to_owned()),
        }
    }

    fn open(&self) -> &str {
        match self {
            Self::Delim(d) => d,
            Self::Tag(open, _) => open,
        }
    }

    fn close(&self) -> &str {
        match self {
            Self::Delim(d) => d,
            Self::Tag(_, close) => close,
        }
    }

    /// The empty pair, and the length of its first half.
    fn pair(&self) -> (String, usize) {
        (format!("{}{}", self.open(), self.close()), self.open().len())
    }

    /// `*` and `_`, or `**` and `__`, swapped.
    fn swapped(&self) -> Self {
        match self {
            Self::Delim("*") => Self::Delim("_"),
            Self::Delim("_") => Self::Delim("*"),
            Self::Delim("**") => Self::Delim("__"),
            Self::Delim("__") => Self::Delim("**"),
            other => other.clone(),
        }
    }
}

/// A style on a piece, with the node it came from.
#[derive(Clone, Debug)]
struct Mark {
    look: Look,
    spelling: Spelling,
    /// The content of the old node. `None` for a new style.
    node: Option<Range<usize>>,
    /// How many pieces the old node had.
    count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Word,
    Space,
    Punct,
    Code,
    /// A link or an image.
    Link,
    Html,
    Break,
}

impl Class {
    fn is_blank(self) -> bool {
        matches!(self, Self::Space | Self::Break)
    }

    fn is_text(self) -> bool {
        matches!(self, Self::Word | Self::Space | Self::Punct)
    }
}

/// A piece of visible text, or a whole code span, link, image, HTML tag or line break.
#[derive(Clone, Debug)]
struct Piece {
    /// The old source, in the joined text.
    src: Range<usize>,
    class: Class,
    /// The new source, when it is not the old one.
    text: Option<String>,
    /// For a code span: the code without its ticks.
    inner: Range<usize>,
    /// The styles, the outer one first.
    marks: Vec<Mark>,
    selected: bool,
}

impl Piece {
    fn has(&self, look: Look) -> bool {
        self.marks.iter().any(|m| m.look == look)
    }

    /// Whether the piece has `look` or a look of its family, like a colored highlight.
    fn has_kin(&self, look: Look) -> bool {
        self.marks.iter().any(|m| look.is_kin(m.look))
    }

    fn mark(&self, look: Look) -> Option<&Mark> {
        self.marks.iter().find(|m| m.look == look)
    }

    fn source<'a>(&'a self, joined: &'a str) -> &'a str {
        self.text.as_deref().unwrap_or(&joined[self.src.clone()])
    }
}

/// Splits `nodes` into pieces. Text splits into words, spaces and other chars, and also at `cuts`. An escape or an
/// entity is never split.
fn flatten(joined: &str, nodes: &[Node], cuts: &Range<usize>, marks: &mut Vec<Mark>, out: &mut Vec<Piece>) {
    for node in nodes {
        let class = match &node.kind {
            Kind::Text => {
                split_text(joined, node.source.clone(), cuts, marks, out);
                continue;
            }
            Kind::Code => Class::Code,
            Kind::Link(_) | Kind::Image(_) => Class::Link,
            Kind::Html => Class::Html,
            Kind::HardBreak => Class::Break,
            kind => {
                let Some(look) = Look::of(kind) else {
                    continue;
                };
                marks.push(Mark {
                    look,
                    spelling: Spelling::of(
                        &joined[node.source.start..node.content.start],
                        &joined[node.content.end..node.source.end],
                    ),
                    node: Some(node.content.clone()),
                    count: 0,
                });
                flatten(joined, &node.children, cuts, marks, out);
                marks.pop();
                continue;
            }
        };
        out.push(Piece {
            src: node.source.clone(),
            class,
            text: None,
            inner: node.content.clone(),
            marks: marks.clone(),
            selected: false,
        });
    }
}

fn split_text(joined: &str, range: Range<usize>, cuts: &Range<usize>, marks: &[Mark], out: &mut Vec<Piece>) {
    let b = joined.as_bytes();
    let mut start = range.start;
    let mut class = None;
    let mut i = range.start;
    let mut last_unit_start = range.start;
    while i < range.end {
        let (len, unit) = if b[i] == b'\\' && b.get(i + 1).is_some_and(u8::is_ascii_punctuation) {
            (2, Class::Punct)
        } else if let Some(len) = entity_len(&b[i..range.end]) {
            (len, Class::Punct)
        } else {
            let c = joined[i..].chars().next().unwrap_or(' ');
            let unit = if c.is_whitespace() {
                Class::Space
            } else if c.is_alphanumeric() {
                Class::Word
            } else {
                Class::Punct
            };
            (c.len_utf8(), unit)
        };
        // A cut inside an escape or an entity moves out to hold all of it.
        let cut = (i <= cuts.start && cuts.start < i + len) || (last_unit_start < cuts.end && cuts.end <= i);
        if i > start && (class != Some(unit) || cut) {
            push_text(out, start..i, class, marks);
            start = i;
        }
        class = Some(unit);
        last_unit_start = i;
        i += len;
    }
    if i > start {
        push_text(out, start..range.end, class, marks);
    }
}

fn push_text(out: &mut Vec<Piece>, src: Range<usize>, class: Option<Class>, marks: &[Mark]) {
    out.push(Piece {
        inner: src.clone(),
        src,
        class: class.unwrap_or(Class::Punct),
        text: None,
        marks: marks.to_vec(),
        selected: false,
    });
}

/// The length of an entity like `&amp;` or `&#123;` at the start of `b`.
fn entity_len(b: &[u8]) -> Option<usize> {
    if b.first() != Some(&b'&') {
        return None;
    }
    let body = b[1..]
        .iter()
        .take(32)
        .take_while(|c| c.is_ascii_alphanumeric() || **c == b'#')
        .count();
    (body > 0 && b.get(1 + body) == Some(&b';')).then_some(body + 2)
}

/// What a key does to the selected pieces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Add(Look),
    Remove(Look),
    AddCode,
    RemoveCode,
    /// Takes off each look of the family, then adds the color.
    Set(Family, Option<Color>),
}

impl Action {
    fn removes(self, look: Look) -> bool {
        match self {
            Self::Remove(removed) => removed == look,
            Self::Set(family, color) => family.has(look) && color.map(|c| family.look(c)) != Some(look),
            _ => false,
        }
    }
}

/// One touched leaf.
struct Work<'a> {
    leaf: &'a Leaf,
    joined: String,
    nodes: Vec<Node>,
    selection: Range<usize>,
    /// All pieces of the leaf, with all their styles.
    pieces: Vec<Piece>,
}

impl<'a> Work<'a> {
    fn new(text: &str, leaf: &'a Leaf, defs: &Defs, selection: &Range<usize>) -> Self {
        let joined = leaf.joined(text).into_owned();
        let nodes = parse(&joined, defs);
        let selection = leaf.to_joined(selection.start)..leaf.to_joined(selection.end);
        let pieces = pieces_of(&joined, &nodes, &selection);
        Self {
            leaf,
            joined,
            nodes,
            selection,
            pieces,
        }
    }
}

fn pieces_of(joined: &str, nodes: &[Node], selection: &Range<usize>) -> Vec<Piece> {
    let mut pieces = Vec::new();
    flatten(joined, nodes, selection, &mut Vec::new(), &mut pieces);
    let mut counts: HashMap<Range<usize>, usize> = HashMap::new();
    for node in pieces.iter().flat_map(|p| &p.marks).filter_map(|m| m.node.clone()) {
        *counts.entry(node).or_default() += 1;
    }
    for mark in pieces.iter_mut().flat_map(|p| &mut p.marks) {
        if let Some(node) = &mark.node {
            mark.count = counts.get(node).copied().unwrap_or(0);
        }
    }
    for piece in &mut pieces {
        piece.selected = piece.src.start < selection.end && selection.start < piece.src.end;
    }
    pieces
}

/// The new source of one leaf part.
struct Change {
    /// The old source.
    old: Range<usize>,
    new: String,
    /// For each piece: its old source in the joined text of the leaf, and its new source in `new`. The flag says
    /// whether the piece is written as it was.
    pieces: Vec<(Range<usize>, Range<usize>, bool)>,
    /// The leaf, to map a source offset into its joined text.
    leaf: Leaf,
}

fn change(text: &str, body: &Body, selection: &Range<usize>, format: Format) -> Option<Vec<Change>> {
    let leaves = body.leaves();
    let first = leaves.partition_point(|leaf| leaf.range().end < selection.start);
    let works: Vec<Work<'_>> = leaves[first..]
        .iter()
        .take_while(|leaf| leaf.range().start <= selection.end)
        .map(|leaf| Work::new(text, leaf, body.defs(), selection))
        .filter(|work| work.pieces.iter().any(|p| p.selected && !p.class.is_blank()))
        .collect();
    let selected = || {
        works
            .iter()
            .flat_map(|w| &w.pieces)
            .filter(|p| p.selected && !p.class.is_blank())
    };
    selected().next()?;
    let action = match (format, format.look()) {
        (Format::TextColor(color), _) => Action::Set(Family::Color, color),
        (Format::HighlightColor(color), _) => Action::Set(Family::Highlight, color),
        // Ctrl+H on any highlight removes it, so that a piece never gets two highlights.
        (_, Some(look)) if selected().all(|p| p.has_kin(look)) => match look.family() {
            Some(family) => Action::Set(family, None),
            None => Action::Remove(look),
        },
        (_, Some(look)) => Action::Add(look),
        _ if selected().all(|p| p.class == Class::Code) => Action::RemoveCode,
        _ => Action::AddCode,
    };
    let mut changes = Vec::with_capacity(works.len());
    for work in &works {
        if let Rewrite::Changed(change) = leaf_change(text, work, body, action)? {
            changes.push(change);
        }
    }
    (!changes.is_empty()).then_some(changes)
}

/// A leaf written again with the wanted styles.
enum Rewrite {
    Unchanged,
    Changed(Change),
}

/// The change to one leaf. It is `None` when no valid Markdown gives the wanted styles.
fn leaf_change(text: &str, work: &Work<'_>, body: &Body, action: Action) -> Option<Rewrite> {
    let (path, range) = region(&work.nodes, &work.selection, action)?;
    let whole = path.is_empty() && range == (0..work.nodes.len());
    write_region(text, work, body, action, &path, range).or_else(|| {
        // Marker chars in the text outside the part can pair with new markers. The last try writes the whole leaf, so
        // that they can get escapes.
        if whole {
            return None;
        }
        write_region(text, work, body, action, &[], 0..work.nodes.len())
    })
}

/// The change from writing `range` of the node list at `path` again.
fn write_region(
    text: &str,
    work: &Work<'_>,
    body: &Body,
    action: Action,
    path: &[usize],
    range: Range<usize>,
) -> Option<Rewrite> {
    let mut list = work.nodes.as_slice();
    let mut outer = Vec::new();
    for &k in path {
        let node = &list[k];
        if let Some(look) = Look::of(&node.kind) {
            outer.push(Mark {
                look,
                spelling: Spelling::Delim(""),
                node: None,
                count: 0,
            });
        }
        list = &node.children;
    }
    let region = list[range.start].source.start..list[range.end - 1].source.end;
    let mut pieces = pieces_of(&work.joined, &list[range], &work.selection);
    let start = if region.start == 0 || work.joined.as_bytes()[region.start - 1] == b'\n' {
        Start::LineStart
    } else {
        Start::MidLine
    };
    act(&mut pieces, &outer, action, &work.joined, start, work.leaf.container())?;
    trim_edges(&mut pieces);
    let mut wanted: Vec<Piece> = work
        .pieces
        .iter()
        .filter(|p| p.src.end <= region.start)
        .cloned()
        .collect();
    wanted.extend(pieces.iter().map(|p| {
        let mut p = p.clone();
        p.marks.splice(0..0, outer.iter().cloned());
        p
    }));
    wanted.extend(work.pieces.iter().filter(|p| p.src.start >= region.end).cloned());
    trim_edges(&mut wanted);
    let wanted = flat(&work.joined, &wanted);
    let target = match action {
        Action::Add(look) | Action::Remove(look) => Some(look),
        Action::Set(family, color) => color.map(|c| family.look(c)),
        _ => None,
    };
    let breaks = work.joined[region.clone()].matches('\n').count();
    let mut tried = HashSet::new();
    for policy in Policy::all(target) {
        let mut writer = Writer {
            joined: &work.joined,
            pieces: &pieces,
            policy,
            out: String::new(),
            at: vec![0..0; pieces.len()],
        };
        writer.write(0, pieces.len(), &mut Vec::new());
        let Writer { out, at, .. } = writer;
        if !tried.insert(out.clone()) || out.matches('\n').count() != breaks {
            continue;
        }
        let mut joined = work.joined.clone();
        joined.replace_range(region.clone(), &out);
        let nodes = parse(&joined, body.defs());
        let mut got = pieces_of(&joined, &nodes, &(0..0));
        trim_edges(&mut got);
        if flat(&joined, &got) != wanted {
            continue;
        }
        if out == work.joined[region.clone()] {
            return Some(Rewrite::Unchanged);
        }
        let exact = policy.escape == Escape::None;
        let change = source_change(text, work, &region, &out, &pieces, &at, exact);
        if !keeps_blocks(text, body, &join(text, std::slice::from_ref(&change), 0..0)) {
            continue;
        }
        return Some(Rewrite::Changed(change));
    }
    None
}

/// The part of the tree to write again: the path to a list of nodes, and a range in it.
fn region(nodes: &[Node], selection: &Range<usize>, action: Action) -> Option<(Vec<usize>, Range<usize>)> {
    let overlaps = |n: &Node| n.source.start < selection.end && selection.start < n.source.end;
    let mut path = Vec::new();
    let mut list = nodes;
    let range = loop {
        let first = list.iter().position(overlaps)?;
        let end = list.iter().rposition(overlaps)? + 1;
        let node = &list[first];
        // Go into a style node only when the selection holds a part of its text. A key on all of it may need to write
        // its markers again.
        let inside = node.source.start <= selection.start && selection.end <= node.source.end;
        let all = selection.start <= node.content.start && node.content.end <= selection.end;
        if first + 1 == end && Look::of(&node.kind).is_some() && inside && !all {
            path.push(first);
            list = &node.children;
        } else {
            break first..end;
        }
    };
    if matches!(action, Action::Remove(_) | Action::Set(..)) {
        let mut outer = nodes;
        for (depth, &k) in path.iter().enumerate() {
            if Look::of(&outer[k].kind).is_some_and(|look| action.removes(look)) {
                path.truncate(depth);
                return Some((path, widen(outer, k..k + 1)));
            }
            outer = &outer[k].children;
        }
    }
    Some((path, widen(list, range)))
}

/// The range with the style nodes next to it, so that runs of one style can join. Then the text nodes next to it join
/// too, so that the last try can escape their marker chars.
fn widen(list: &[Node], mut range: Range<usize>) -> Range<usize> {
    while range.start > 0 && Look::of(&list[range.start - 1].kind).is_some() {
        range.start -= 1;
    }
    while list.get(range.end).is_some_and(|n| Look::of(&n.kind).is_some()) {
        range.end += 1;
    }
    if range.start > 0 && list[range.start - 1].kind == Kind::Text {
        range.start -= 1;
    }
    if list.get(range.end).is_some_and(|n| n.kind == Kind::Text) {
        range.end += 1;
    }
    range
}

/// Where a text starts. At the start of a line, `#`, `-`, `+`, `>`, `|` and `1.` could start a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Start {
    LineStart,
    MidLine,
}

/// Changes the selected pieces for `action`. `outer` are the styles around all of them.
fn act(
    pieces: &mut Vec<Piece>,
    outer: &[Mark],
    action: Action,
    joined: &str,
    start: Start,
    container: Container,
) -> Option<()> {
    match action {
        Action::Add(look) => add(pieces, outer, look),
        Action::Remove(_) | Action::Set(..) => {
            for piece in pieces.iter_mut().filter(|p| p.selected) {
                piece.marks.retain(|m| !action.removes(m.look));
            }
            if let Action::Set(family, Some(color)) = action {
                add(pieces, outer, family.look(color));
            }
        }
        Action::RemoveCode => {
            for piece in pieces.iter_mut().filter(|p| p.selected && p.class == Class::Code) {
                let start = if piece.src.start == 0 { start } else { Start::MidLine };
                let code = &joined[piece.inner.clone()];
                piece.text = Some(escape(code, start, container));
                piece.class = Class::Punct;
            }
        }
        Action::AddCode => {
            let first = pieces.iter().position(|p| p.selected && !p.class.is_blank())?;
            let last = pieces.iter().rposition(|p| p.selected && !p.class.is_blank())?;
            let part = &pieces[first..=last];
            if part.iter().any(|p| matches!(p.class, Class::Link | Class::Break)) {
                return None;
            }
            let mut code = String::new();
            for piece in part {
                match piece.class {
                    Class::Code => code.push_str(&joined[piece.inner.clone()]),
                    Class::Html => code.push_str(&joined[piece.src.clone()]),
                    _ if container == Container::TableCell => {
                        code.push_str(&unescape_but_pipes(&joined[piece.src.clone()]));
                    }
                    _ => code.push_str(&scan::unescape(&joined[piece.src.clone()])),
                }
            }
            let marks: Vec<Mark> = part[0]
                .marks
                .iter()
                .filter(|m| part.iter().all(|p| p.has(m.look)))
                .cloned()
                .collect();
            let src = part[0].src.start..part[part.len() - 1].src.end;
            let piece = Piece {
                inner: src.clone(),
                src,
                class: Class::Code,
                text: Some(fence(&code)),
                marks,
                selected: true,
            };
            pieces.splice(first..=last, [piece]);
        }
    }
    Some(())
}

/// Adds `look` to the selected pieces that have no look of its kind, unless a style around them has one.
fn add(pieces: &mut [Piece], outer: &[Mark], look: Look) {
    if outer.iter().any(|m| look.is_kin(m.look)) {
        return;
    }
    for piece in pieces.iter_mut().filter(|p| p.selected && !p.has_kin(look)) {
        piece.marks.push(Mark {
            look,
            spelling: look.default_spelling(),
            node: None,
            count: 0,
        });
    }
}

/// Writes `text` with a backslash before each char that could pair with a marker.
fn escape_markers(text: &str, out: &mut String) {
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            out.push(c);
            // A backslash that escapes nothing gets a backslash. Then it cannot escape a marker.
            match chars.next_if(char::is_ascii_punctuation) {
                Some(next) => out.push(next),
                None => out.push('\\'),
            }
            continue;
        }
        if "*_~=<[]".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
}

/// Code with ticks around it. A tick inside gives a longer fence.
fn fence(code: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in code.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let ticks = "`".repeat(longest + 1);
    let pad = code.starts_with('`')
        || code.ends_with('`')
        || (code.starts_with(' ') && code.ends_with(' ') && !code.trim().is_empty());
    let space = if pad { " " } else { "" };
    format!("{ticks}{space}{code}{space}{ticks}")
}

/// Text that shows `code` as it is: each char that Markdown reads as syntax gets a backslash.
fn escape(code: &str, start: Start, container: Container) -> String {
    let mut out = String::with_capacity(code.len() + 8);
    let mut chars = code.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if container == Container::TableCell && c == '\\' && chars.peek().is_some_and(|&(_, n)| n == '|') {
            out.push_str("\\|");
            chars.next();
            continue;
        }
        let line_start = i == 0 && start == Start::LineStart;
        if "\\`*_~=[]<&!".contains(c) || (line_start && "#-+>|".contains(c)) {
            out.push('\\');
        }
        out.push(c);
    }
    if start == Start::LineStart {
        let digits = out.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 && matches!(out.as_bytes().get(digits), Some(b'.' | b')')) {
            out.insert(digits, '\\');
        }
    }
    out
}

fn unescape_but_pipes(text: &str) -> String {
    text.split("\\|").map(scan::unescape).collect::<Vec<_>>().join("\\|")
}

/// Takes a style off the blank pieces at the ends of each of its runs. A run that is still exactly one old node stays
/// as it is.
fn trim_edges(pieces: &mut [Piece]) {
    let mut looks: Vec<Look> = Vec::new();
    for mark in pieces.iter().flat_map(|p| &p.marks) {
        if !looks.contains(&mark.look) {
            looks.push(mark.look);
        }
    }
    for look in looks {
        let mut i = 0;
        while i < pieces.len() {
            if !pieces[i].has(look) {
                i += 1;
                continue;
            }
            let end = (i..pieces.len())
                .find(|&k| !pieces[k].has(look))
                .unwrap_or(pieces.len());
            if !is_old_node(pieces, i, end, look) {
                let text = |k: &usize| !pieces[*k].class.is_blank();
                let (first, last) = match ((i..end).find(text), (i..end).rfind(text)) {
                    (Some(first), Some(last)) => (first, last + 1),
                    _ => (end, end),
                };
                for k in (i..first).chain(last..end) {
                    pieces[k].marks.retain(|m| m.look != look);
                }
            }
            i = end;
        }
    }
}

/// Whether the run `a..b` of `look` is exactly the pieces of one old node.
fn is_old_node(pieces: &[Piece], a: usize, b: usize, look: Look) -> bool {
    let Some(mark) = pieces[a].mark(look).filter(|m| m.node.is_some()) else {
        return false;
    };
    let same = |p: &Piece| p.mark(look).and_then(|m| m.node.as_ref()) == mark.node.as_ref();
    mark.count == b - a && pieces[a..b].iter().all(same)
}

/// The pieces as the check compares them: text with the same styles joins into one piece.
#[derive(Debug, PartialEq, Eq)]
struct Flat {
    text: String,
    class: Option<Class>,
    looks: Vec<Look>,
}

fn flat(joined: &str, pieces: &[Piece]) -> Vec<Flat> {
    let mut out: Vec<Flat> = Vec::new();
    for (i, piece) in pieces.iter().enumerate() {
        let mut looks: Vec<Look> = Vec::new();
        for mark in &piece.marks {
            if !looks.contains(&mark.look) {
                looks.push(mark.look);
            }
        }
        if piece.class.is_blank() {
            // Bold and italic count on a space only when the text on both sides has them.
            let before = pieces[..i].iter().rfind(|p| !p.class.is_blank());
            let after = pieces[i + 1..].iter().find(|p| !p.class.is_blank());
            looks.retain(|look| look.shows_on_space());
            if let (Some(before), Some(after)) = (before, after) {
                for look in [Look::Emphasis, Look::Strong] {
                    if before.has(look) && after.has(look) {
                        looks.push(look);
                    }
                }
            }
        }
        // Raw HTML shows as it is written, like text.
        let class = (!piece.class.is_text() && piece.class != Class::Html).then_some(piece.class);
        let text = if class.is_none() {
            scan::unescape(piece.source(joined)).into_owned()
        } else {
            piece.source(joined).to_owned()
        };
        if let Some(last) = out.last_mut()
            && class.is_none()
            && last.class.is_none()
            && same_looks(&last.looks, &looks)
        {
            last.text.push_str(&text);
            continue;
        }
        out.push(Flat { text, class, looks });
    }
    for item in &mut out {
        item.looks.sort_by_key(|look| format!("{look:?}"));
    }
    out
}

fn same_looks(a: &[Look], b: &[Look]) -> bool {
    a.len() == b.len() && a.iter().all(|look| b.contains(look))
}

/// How a try writes the markers.
#[derive(Clone, Copy, Debug)]
struct Policy {
    html: Html,
    cut: Cut,
    tie: Tie,
    emphasis: Spell,
    strong: Spell,
    escape: Escape,
}

/// Which runs a try writes as HTML tags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Html {
    Never,
    /// The changed runs of one style.
    Changed(Look),
    Always,
}

/// Which of two crossing runs a try cuts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cut {
    /// The new or changed run.
    Changed,
    Old,
}

/// Which style a try puts outside, for runs of the same length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tie {
    OuterFirst,
    InnerFirst,
}

/// How a try spells the markers of a changed run of one style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spell {
    Keep,
    /// `*` for `_`, and `_` for `*`.
    Swap,
}

/// Whether a try puts a backslash before each marker char in the text, so that it cannot pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Escape {
    None,
    Markers,
}

impl Policy {
    fn all(target: Option<Look>) -> Vec<Self> {
        let mut all = Vec::new();
        for (html, escape) in [
            (Html::Never, Escape::None),
            (target.map_or(Html::Never, Html::Changed), Escape::None),
            (Html::Always, Escape::None),
            (Html::Never, Escape::Markers),
            (Html::Always, Escape::Markers),
        ] {
            for cut in [Cut::Changed, Cut::Old] {
                for tie in [Tie::OuterFirst, Tie::InnerFirst] {
                    for (emphasis, strong) in [
                        (Spell::Keep, Spell::Keep),
                        (Spell::Swap, Spell::Keep),
                        (Spell::Keep, Spell::Swap),
                        (Spell::Swap, Spell::Swap),
                    ] {
                        all.push(Self {
                            html,
                            cut,
                            tie,
                            emphasis,
                            strong,
                            escape,
                        });
                    }
                }
            }
        }
        all
    }
}

struct Writer<'a> {
    joined: &'a str,
    pieces: &'a [Piece],
    policy: Policy,
    out: String,
    /// The new source of each piece.
    at: Vec<Range<usize>>,
}

impl Writer<'_> {
    /// Writes pieces `a..b`. `open` are the styles of the nodes around them.
    fn write(&mut self, a: usize, b: usize, open: &mut Vec<Look>) {
        let mut i = a;
        while i < b {
            let piece = &self.pieces[i];
            // A run starts on a space only where a cut put it. A style that does not show there stays off.
            let candidates: Vec<(usize, Look)> = piece
                .marks
                .iter()
                .enumerate()
                .filter(|(_, m)| !open.contains(&m.look))
                .filter(|(_, m)| !piece.class.is_blank() || m.look.shows_on_space())
                .map(|(k, m)| (k, m.look))
                .collect();
            let best = candidates.iter().max_by_key(|(k, look)| {
                let order = match self.policy.tie {
                    Tie::OuterFirst => -k.cast_signed(),
                    Tie::InnerFirst => k.cast_signed(),
                };
                (self.run_end(i, *look, b), order)
            });
            let Some(&(_, look)) = best else {
                let start = self.out.len();
                let source = piece.source(self.joined);
                let escape = self.policy.escape == Escape::Markers;
                if escape && piece.class == Class::Punct && piece.text.is_none() {
                    escape_markers(source, &mut self.out);
                } else if escape && piece.class == Class::Html && html_tag::read(source).is_some() {
                    // A lone Holm tag could pair with a new one.
                    self.out.push('\\');
                    self.out.push_str(source);
                } else {
                    self.out.push_str(piece.source(self.joined));
                }
                self.at[i] = start..self.out.len();
                i += 1;
                continue;
            };
            let mut end = self.run_end(i, look, b);
            let changed = self.changed(i, end, look);
            if self.policy.cut == Cut::Changed
                && changed
                && let Some(cut) = self.crossing(i, end, look, open, b)
            {
                end = cut;
                while end > i + 1 && !look.shows_on_space() && self.pieces[end - 1].class.is_blank() {
                    end -= 1;
                }
            }
            let spelling = self.spelling(i, end, look, changed);
            self.out.push_str(spelling.open());
            open.push(look);
            self.write(i, end, open);
            open.pop();
            self.out.push_str(spelling.close());
            i = end;
        }
    }

    fn run_end(&self, from: usize, look: Look, b: usize) -> usize {
        (from..b).find(|&k| !self.pieces[k].has(look)).unwrap_or(b)
    }

    fn changed(&self, a: usize, b: usize, look: Look) -> bool {
        !is_old_node(self.pieces, a, b, look)
    }

    /// The first piece in `a..b` where a run of another style starts and goes on past `b`.
    fn crossing(&self, a: usize, b: usize, look: Look, open: &[Look], end: usize) -> Option<usize> {
        (a + 1..b).find(|&k| {
            self.pieces[k].marks.iter().any(|m| {
                m.look != look
                    && !open.contains(&m.look)
                    && !self.pieces[k - 1].has(m.look)
                    && self.run_end(k, m.look, end) > b
            })
        })
    }

    fn spelling(&self, a: usize, b: usize, look: Look, changed: bool) -> Spelling {
        let old = self.pieces[a..b]
            .iter()
            .filter_map(|p| p.mark(look))
            .find(|m| m.node.is_some())
            .or_else(|| self.pieces[a].mark(look))
            .map_or_else(|| look.default_spelling(), |m| m.spelling.clone());
        let policy = self.policy;
        if matches!(old, Spelling::Delim(_))
            && (policy.html == Html::Always || (changed && policy.html == Html::Changed(look)))
            && let Some(html) = look.html()
        {
            return html;
        }
        if !changed {
            return old;
        }
        match look {
            Look::Emphasis if policy.emphasis == Spell::Swap => old.swapped(),
            Look::Strong if policy.strong == Spell::Swap => old.swapped(),
            _ => old,
        }
    }
}

/// The change in source offsets for the new text `out` of `region` in a leaf.
fn source_change(
    text: &str,
    work: &Work<'_>,
    region: &Range<usize>,
    out: &str,
    pieces: &[Piece],
    at: &[Range<usize>],
    exact: bool,
) -> Change {
    let leaf = work.leaf;
    let mut line = work.joined[..region.start].matches('\n').count();
    let mut new = String::with_capacity(out.len());
    // The source offset of each offset in `out`.
    let mut offsets = Vec::with_capacity(out.len() + 1);
    for (i, c) in out.char_indices() {
        offsets.resize(i + 1, new.len());
        if c == '\n' {
            new.push_str(leaf.gap(text, line));
            line += 1;
        } else {
            new.push(c);
        }
    }
    offsets.resize(out.len() + 1, new.len());
    let pieces = pieces
        .iter()
        .zip(at)
        .map(|(p, at)| {
            // A piece written as it was keeps the offsets inside it.
            let text = exact && p.text.is_none();
            (p.src.clone(), offsets[at.start]..offsets[at.end], text)
        })
        .collect();
    Change {
        old: leaf.to_source(region.start)..leaf.to_source(region.end),
        new,
        pieces,
        leaf: leaf.clone(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum End {
    Start,
    End,
}

/// Where the source offset `pos` lands in the new text. An offset in a changed part keeps its place in the visible
/// text.
fn map(changes: &[Change], pos: usize, end: End) -> usize {
    let mut delta = 0_isize;
    for change in changes {
        if pos < change.old.start {
            break;
        }
        if pos <= change.old.end {
            let start = change.old.start.saturating_add_signed(delta);
            return start + map_in(change, change.leaf.to_joined(pos), end);
        }
        delta += signed(change.new.len()) - signed(change.old.len());
    }
    pos.saturating_add_signed(delta)
}

fn signed(n: usize) -> isize {
    isize::try_from(n).unwrap_or(isize::MAX)
}

/// The offset in `change.new` of the joined offset `pos` of its leaf.
fn map_in(change: &Change, pos: usize, end: End) -> usize {
    match end {
        End::Start => {
            let Some((old, new, text)) = change.pieces.iter().find(|(old, _, _)| old.end > pos) else {
                return change.new.len();
            };
            if *text && old.start <= pos {
                new.start + (pos - old.start)
            } else {
                new.start
            }
        }
        End::End => {
            let Some((old, new, text)) = change.pieces.iter().rev().find(|(old, _, _)| old.start < pos) else {
                return 0;
            };
            if *text && pos <= old.end {
                new.start + (pos - old.start)
            } else {
                new.end
            }
        }
    }
}

fn join(text: &str, changes: &[Change], selection: Range<usize>) -> Edit {
    let start = changes[0].old.start;
    let end = changes[changes.len() - 1].old.end;
    let mut new = String::new();
    let mut at = start;
    for change in changes {
        new.push_str(&text[at..change.old.start]);
        new.push_str(&change.new);
        at = change.old.end;
    }
    let old = &text[start..end];
    let prefix = common_len(old.chars(), new.chars());
    let suffix = common_len(old[prefix..].chars().rev(), new[prefix..].chars().rev());
    Edit {
        range: start + prefix..end - suffix,
        text: new[prefix..new.len() - suffix].to_owned(),
        selection,
    }
}

/// The byte length of the chars that two char sequences share at their start.
fn common_len(a: impl Iterator<Item = char>, b: impl Iterator<Item = char>) -> usize {
    a.zip(b).take_while(|(x, y)| x == y).map(|(c, _)| c.len_utf8()).sum()
}

/// A key with no selection: on the word at `cursor`, or an empty marker pair between words.
fn at_cursor(text: &str, body: &Body, cursor: usize, format: Format) -> Option<Edit> {
    let leaves = body.leaves();
    let i = leaves.partition_point(|leaf| leaf.range().end < cursor);
    let leaf = leaves.get(i).filter(|leaf| leaf.range().start <= cursor);
    let Some(leaf) = leaf else {
        return pair(text, body, cursor, format);
    };
    let joined = leaf.joined(text);
    let nodes = parse(&joined, body.defs());
    let pieces = pieces_of(&joined, &nodes, &(0..0));
    let pos = leaf.to_joined(cursor);
    let inside = pieces
        .iter()
        .find(|p| p.src.start < pos && pos < p.src.end && !p.class.is_text());
    let word = if let Some(atom) = inside {
        Some(atom.src.clone())
    } else {
        let containing = pieces.iter().position(|p| p.src.start < pos && pos < p.src.end);
        let left = pieces.iter().rposition(|p| p.src.end <= pos);
        let right = pieces.iter().position(|p| p.src.start >= pos);
        let near = if containing.is_some() {
            [containing, None]
        } else {
            [left, right]
        };
        near.into_iter()
            .flatten()
            .find(|&k| pieces[k].class == Class::Word)
            .map(|k| {
                let mut a = k;
                while a > 0 && pieces[a - 1].class == Class::Word {
                    a -= 1;
                }
                let mut b = k;
                while b + 1 < pieces.len() && pieces[b + 1].class == Class::Word {
                    b += 1;
                }
                pieces[a].src.start..pieces[b].src.end
            })
    };
    let Some(word) = word else {
        return pair(text, body, cursor, format);
    };
    let range = leaf.to_source(word.start)..leaf.to_source(word.end);
    let changes = change(text, body, &range, format)?;
    let end = if cursor <= range.start { End::Start } else { End::End };
    let cursor = map(&changes, cursor, end);
    let edit = join(text, &changes, cursor..cursor);
    keeps_blocks(text, body, &edit).then_some(edit)
}

/// An empty marker pair at `cursor`, when spaces or line ends stand on both sides.
fn pair(text: &str, body: &Body, cursor: usize, format: Format) -> Option<Edit> {
    let before = text[..cursor].chars().next_back();
    let after = text[cursor..].chars().next();
    let blank = |c: Option<char>| c.is_none_or(char::is_whitespace);
    if !blank(before) || !blank(after) || in_raw_block(body.blocks(), cursor) {
        return None;
    }
    let (pair, half) = format.pair()?;
    let inner = cursor + half;
    Some(Edit {
        range: cursor..cursor,
        text: pair,
        selection: inner..inner,
    })
}

fn in_raw_block(blocks: &[crate::Block], cursor: usize) -> bool {
    blocks.iter().any(|block| {
        (block.range.start <= cursor && cursor <= block.range.end)
            && (matches!(block.kind, BlockKind::Code { .. } | BlockKind::Html) || in_raw_block(&block.children, cursor))
    })
}
