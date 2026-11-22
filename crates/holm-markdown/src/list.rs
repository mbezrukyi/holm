//! The list keys: Tab, Shift+Tab and Enter in a list.
//!
//! A key reads the list tree around the selection from the parse, makes its move on the tree,
//! and writes the lists out again. Each line keeps its text and gets the indent and the number
//! of its new place.

use std::ops::Range;

use crate::inline::Edit;
use crate::{Block, BlockKind, Body};

/// What a list key does. See [`relist`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    /// Tab: each item that the selection touches becomes the last child of the item above it.
    In,
    /// Shift+Tab: each item that the selection touches becomes the next sibling of its parent.
    /// The items below it in its list become its children.
    Out,
    /// Enter: starts a new item at the cursor, or ends the list on an empty item.
    Next,
}

/// The edit of the list key `key` on `selection` in `text`. `body` is the parse of `text`.
///
/// An item moves whole, with its children and its later paragraphs. A moved item takes the
/// marker of the list it joins. Each ordered list that the key changes counts up from its first
/// number, and a new list starts at 1. Other lists keep their numbers. The edit is empty when
/// the key does nothing in a list, like Tab on the first item of a list.
///
/// Gives `None` outside a list item, and for Enter before the text of the item.
///
/// ```
/// use holm_markdown::Body;
/// use holm_markdown::list::{Move, relist};
///
/// let text = "1. Milk\n2. Eggs\n3. Bread";
/// let edit = relist(text, &Body::parse(text), 20..20, Move::In).unwrap();
/// let mut new = text.to_owned();
/// new.replace_range(edit.range, &edit.text);
/// assert_eq!(new, "1. Milk\n2. Eggs\n   1. Bread");
/// assert_eq!(edit.selection, 23..23);
/// ```
pub fn relist(text: &str, body: &Body, selection: Range<usize>, key: Move) -> Option<Edit> {
    Some(plan(text, body, selection, key)?.edit)
}

/// The edit of a key, with the tree it plans for the text after the edit.
struct Plan {
    edit: Edit,
    /// The new tree. `None` when the edit writes no tree.
    #[cfg_attr(not(test), expect(dead_code, reason = "the tests compare it with the parse"))]
    tree: Option<Planned>,
}

/// A tree that a key writes.
#[cfg_attr(not(test), expect(dead_code, reason = "the tests compare it with the parse"))]
struct Planned {
    parts: Vec<Part>,
    /// The lines of the tree in the old text.
    old: Range<usize>,
    /// The lines of the tree in the new text.
    new: Range<usize>,
    /// The number of quotes around the tree.
    quotes: usize,
}

fn plan(text: &str, body: &Body, selection: Range<usize>, key: Move) -> Option<Plan> {
    let end = floor(text, selection.end.min(text.len()));
    let selection = floor(text, selection.start.min(end))..end;
    if key == Move::Next {
        next(text, body, selection)
    } else {
        shift(text, body, selection, key == Move::Out)
    }
}

fn floor(text: &str, mut offset: usize) -> usize {
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// Tab and Shift+Tab.
fn shift(text: &str, body: &Body, selection: Range<usize>, out: bool) -> Option<Plan> {
    let lines = line_start(text, selection.start)..line_end(text, selection.end);
    let found = path_to(body.blocks(), &|b| {
        is_item(b) && lines.start <= b.range.start && b.range.start <= lines.end
    });
    let path = if let Some(path) = found {
        path
    } else {
        let path = chain_at(body.blocks(), selection.start);
        let item = path.iter().rposition(|b| is_item(b))?;
        let in_code = path[item..]
            .iter()
            .any(|b| matches!(b.kind, BlockKind::Code { .. } | BlockKind::Html));
        if in_code {
            return None;
        }
        path[..=item].to_vec()
    };
    let (source, lists, item) = Source::around(text, body.blocks(), &path)?;
    let mut tree = source.tree(&lists)?;
    let touched = if lines.end < source.region.start || source.region.end < lines.start {
        Vec::new()
    } else {
        let first = source.line_of(lines.start.max(source.region.start));
        let last = source.line_of(lines.end.min(source.region.end));
        let mut touched = Vec::new();
        top_items(&tree, &(first..=last), &mut touched);
        touched
    };
    let touched = if touched.is_empty() {
        vec![source.line_of(item.range.start)]
    } else {
        touched
    };
    let mut changed = false;
    for id in touched {
        let moved = if out {
            move_out(&mut tree, id)
        } else {
            move_in(&mut tree, id)
        };
        changed |= moved.is_some();
    }
    if !changed {
        return Some(nothing(selection));
    }
    merge_lists(&mut tree);
    let writer = Writer::write(text, &tree, source.after_text());
    if writer.broken || source.captures(&writer) {
        return Some(nothing(selection));
    }
    let start = source.region.start;
    let selection = start + writer.map(selection.start)..start + writer.map(selection.end);
    Some(source.finish(writer, tree, selection))
}

/// The edit that changes nothing.
fn nothing(selection: Range<usize>) -> Plan {
    Plan {
        edit: Edit {
            range: selection.start..selection.start,
            text: String::new(),
            selection,
        },
        tree: None,
    }
}

/// Enter.
fn next(text: &str, body: &Body, selection: Range<usize>) -> Option<Plan> {
    let line = line_start(text, selection.start)..line_end(text, selection.start);
    if selection.end > line.end {
        return None;
    }
    let path = path_to(body.blocks(), &|b| {
        is_item(b) && line.start <= b.range.start && b.range.start <= line.end
    })?;
    let (source, lists, item) = Source::around(text, body.blocks(), &path)?;
    let mut tree = source.tree(&lists)?;
    let id = source.line_of(item.range.start);
    let at = find(&tree, id)?;
    let (&(p, i), up) = at.split_last()?;
    let Part::List(list) = parts_mut(&mut tree, up)?.get_mut(p)? else {
        return None;
    };
    let current = list.items.get_mut(i)?;
    let Src::Old(kept) = current.kept.clone() else {
        return None;
    };
    let after = &text[kept.start..line.end];
    let spaces = after.bytes().take_while(|b| *b == b' ').count();
    let task = ["[ ] ", "[x] ", "[X] "]
        .iter()
        .any(|task| after[spaces..].starts_with(task));
    let content = kept.start + spaces + if task { 4 } else { 0 };
    if selection.start < content {
        return None;
    }
    if content == line.end && selection.end == line.end {
        // A blank line ends the list. Without it, typed text joins the item above.
        let quote = &text[line.start..source.prefix_end(id)];
        let lines = format!("{}\n{quote}", quote.trim_end());
        let cursor = line.start + lines.len();
        return Some(Plan {
            edit: Edit {
                range: line,
                text: lines,
                selection: cursor..cursor,
            },
            tree: None,
        });
    }
    let rest = text[selection.end..line.end].trim().is_empty();
    let parts = &current.parts;
    let lazy_or_blank = |part: &Part| matches!(part, Part::Line(line) if line.extra.is_none());
    if rest && parts.first().is_some_and(lazy_or_blank) && !parts.iter().all(Part::is_blank) {
        // The new item would have no text. A blank line would end it before its parts, and a
        // lazy line would have no paragraph to go on.
        return None;
    }
    current.kept = Src::Old(kept.start..selection.start);
    current.empty = no_text(&text[kept.start..selection.start]);
    let task = if task { "[ ] " } else { "" };
    let kept = format!(" {task}{}", &text[selection.end..line.end]);
    let new = Item {
        id: NEW,
        prefix: current.prefix.clone(),
        mark: current.mark,
        number: current.number,
        first: false,
        rel: current.rel,
        gap: gap(&kept)?,
        kept: Src::New(kept),
        empty: rest,
        parts: std::mem::take(&mut current.parts),
    };
    list.items.insert(i + 1, new);
    list.renumber = true;
    let writer = Writer::write(text, &tree, source.after_text());
    if writer.broken || source.captures(&writer) {
        return None;
    }
    let cursor = writer
        .keeps
        .iter()
        .find(|keep| keep.old_keep.is_none())
        .map(|keep| source.region.start + keep.new_keep + 1 + task.len())?;
    Some(source.finish(writer, tree, cursor..cursor))
}

/// Where the last item of the list `block` ends. The range of a list can end in the indent of
/// the next line.
fn list_end(block: &Block) -> usize {
    block.children.last().map_or(block.range.end, |item| item.range.end)
}

fn is_item(block: &Block) -> bool {
    matches!(block.kind, BlockKind::Item { .. })
}

fn line_start(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map_or(0, |at| at + 1)
}

fn line_end(text: &str, offset: usize) -> usize {
    text[offset..].find('\n').map_or(text.len(), |at| offset + at)
}

/// The blocks from the top level down to the first block in document order that `pick` takes.
fn path_to<'b>(blocks: &'b [Block], pick: &dyn Fn(&Block) -> bool) -> Option<Vec<&'b Block>> {
    for block in blocks {
        if pick(block) {
            return Some(vec![block]);
        }
        if let Some(mut path) = path_to(&block.children, pick) {
            path.insert(0, block);
            return Some(path);
        }
    }
    None
}

/// The blocks that hold `offset`, from the top level inward.
fn chain_at(blocks: &[Block], offset: usize) -> Vec<&Block> {
    let mut chain = Vec::new();
    let mut level = blocks;
    while let Some(block) = level.iter().find(|b| b.range.start <= offset && offset <= b.range.end) {
        chain.push(block);
        level = &block.children;
    }
    chain
}

/// The width of leading space and tabs, with tab stops of 4.
fn columns(lead: &str) -> usize {
    lead.bytes()
        .fold(0, |col, b| if b == b'\t' { col + 4 - col % 4 } else { col + 1 })
}

/// The marker at the start of `line`: its kind, its number and its length.
fn marker(line: &str) -> Option<(Mark, u64, usize)> {
    let bytes = line.as_bytes();
    match bytes.first()? {
        b @ (b'-' | b'*' | b'+') => Some((Mark::Bullet(char::from(*b)), 0, 1)),
        b'0'..=b'9' => {
            let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
            if digits > 9 {
                return None;
            }
            let number = line[..digits].parse().ok()?;
            match bytes.get(digits)? {
                b @ (b'.' | b')') => Some((Mark::Number(char::from(*b)), number, digits + 1)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A line id of an item that a key adds.
const NEW: usize = usize::MAX;

/// The marker kind of a list. Another bullet or delimiter starts another list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark {
    Bullet(char),
    /// The delimiter after the number.
    Number(char),
}

impl Mark {
    /// Whether an item with this marker can join a list with `other` by taking its marker.
    fn joins(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Bullet(_), Self::Bullet(_)) | (Self::Number(_), Self::Number(_))
        )
    }

    fn text(self, number: u64) -> String {
        match self {
            Self::Bullet(bullet) => bullet.to_string(),
            Self::Number(delimiter) => format!("{number}{delimiter}"),
        }
    }
}

/// Text from the old source, or new text.
#[derive(Clone, Debug)]
enum Src {
    Old(Range<usize>),
    New(String),
}

/// One line of an item that is not its marker line.
#[derive(Clone, Debug)]
struct Line {
    /// The quote markers.
    prefix: Range<usize>,
    /// The indent past the content column of the item. `None` for a blank line or a lazy line:
    /// it is written as it is.
    extra: Option<usize>,
    /// The text after the indent.
    kept: Range<usize>,
    blank: bool,
    /// The width of the indent.
    lead: usize,
    /// Whether the text could start a block, like a list item, if it were not lazy.
    risky: bool,
    /// Whether the line goes on with the text of the line above, and would start a block, like a
    /// list item or indented code, after a line with no text.
    continues: bool,
}

#[derive(Clone, Debug)]
struct Item {
    /// The index of the marker line in the source, or [`NEW`].
    id: usize,
    /// The quote markers of the marker line.
    prefix: Range<usize>,
    mark: Mark,
    number: u64,
    /// Whether the item was the first of its list in the parse.
    first: bool,
    /// The indent of the marker past the content column of the parent.
    rel: usize,
    /// The columns from the end of the marker to the content column.
    gap: usize,
    /// The marker line after the marker.
    kept: Src,
    /// Whether the marker line has no text.
    empty: bool,
    parts: Vec<Part>,
}

#[derive(Clone, Debug)]
struct List {
    mark: Mark,
    /// Whether the key changed the list, so it counts up again.
    renumber: bool,
    items: Vec<Item>,
}

#[derive(Clone, Debug)]
enum Part {
    Line(Line),
    List(List),
}

impl Part {
    fn is_blank(&self) -> bool {
        matches!(self, Self::Line(line) if line.blank)
    }
}

/// The lines of the outermost list around an item.
struct Source<'a> {
    text: &'a str,
    region: Range<usize>,
    lines: Vec<Range<usize>>,
    /// The number of quotes around the list.
    quotes: usize,
}

impl<'a> Source<'a> {
    /// The source of the list tree around the last block of `path`, an item, with the item. The
    /// tree holds the outermost list of the item, and the lists right before and after it.
    fn around<'b>(text: &'a str, top: &'b [Block], path: &[&'b Block]) -> Option<(Self, Vec<&'b Block>, &'b Block)> {
        let item = *path.last()?;
        let mut root = path.len() - 1;
        while root > 0 && matches!(path[root - 1].kind, BlockKind::List { .. } | BlockKind::Item { .. }) {
            root -= 1;
        }
        let list = path[root];
        if !matches!(list.kind, BlockKind::List { .. }) {
            return None;
        }
        let quotes = path[..root]
            .iter()
            .filter(|b| matches!(b.kind, BlockKind::Quote | BlockKind::Callout(_)))
            .count();
        let siblings = match root {
            0 => top,
            _ => &path[root - 1].children,
        };
        let lists = match siblings.iter().position(|b| std::ptr::eq(b, list)) {
            Some(at) => {
                let is_list = |b: &Block| matches!(b.kind, BlockKind::List { .. });
                let from = siblings[..at].iter().rposition(|b| !is_list(b)).map_or(0, |k| k + 1);
                let to = siblings[at..]
                    .iter()
                    .position(|b| !is_list(b))
                    .map_or(siblings.len(), |k| at + k);
                siblings[from..to].iter().collect()
            }
            None => vec![list],
        };
        let start = line_start(text, lists.first()?.range.start);
        let end = line_end(text, list_end(lists.last()?));
        Some((Self::lines(text, start..end, quotes), lists, item))
    }

    /// The tree of `lists`, the lists of the source in order.
    fn tree(&self, lists: &[&Block]) -> Option<Vec<Part>> {
        let mut parts = Vec::new();
        let mut next = 0;
        for list in lists {
            let start = self.line_of(list.range.start);
            let end = self.line_of(list_end(list));
            if start < next {
                return None;
            }
            parts.extend((next..start).map(|l| Part::Line(self.line(l, 0))));
            parts.push(Part::List(self.list(list, end, 0)?));
            next = end + 1;
        }
        Some(parts)
    }

    fn lines(text: &'a str, region: Range<usize>, quotes: usize) -> Self {
        let mut lines = Vec::new();
        let mut start = region.start;
        for (at, _) in text[region.clone()].match_indices('\n') {
            lines.push(start..region.start + at);
            start = region.start + at + 1;
        }
        lines.push(start..region.end);
        Self {
            text,
            region,
            lines,
            quotes,
        }
    }

    fn line_of(&self, offset: usize) -> usize {
        self.lines
            .partition_point(|line| line.start <= offset)
            .saturating_sub(1)
    }

    /// Where the quote markers of `line` end.
    fn prefix_end(&self, line: usize) -> usize {
        self.quotes_end(&self.lines[line]).0
    }

    /// Where the quote markers of the line `range` end, and how many there are.
    fn quotes_end(&self, range: &Range<usize>) -> (usize, usize) {
        let bytes = self.text.as_bytes();
        let mut at = range.start;
        let mut count = 0;
        for _ in 0..self.quotes {
            let mut p = at;
            while p < range.end && bytes[p] == b' ' {
                p += 1;
            }
            if p < range.end && bytes[p] == b'>' {
                at = p + 1;
                count += 1;
                if at < range.end && bytes[at] == b' ' {
                    at += 1;
                }
            } else {
                break;
            }
        }
        (at, count)
    }

    /// Whether the line before the source has text.
    fn after_text(&self) -> bool {
        if self.region.start == 0 {
            return false;
        }
        let line = line_start(self.text, self.region.start - 1)..self.region.start - 1;
        let (at, count) = self.quotes_end(&line);
        count >= self.quotes && !self.text[at..line.end].trim().is_empty()
    }

    /// Whether the written tree would take in the text after the source.
    fn captures(&self, writer: &Writer<'_>) -> bool {
        matches!((writer.last, self.next_indent()), (Some(last), Some(next)) if next >= last)
    }

    /// The indent of the first line with text after the source, when it is in the same quotes.
    fn next_indent(&self) -> Option<usize> {
        let mut start = self.region.end + 1;
        while start < self.text.len() {
            let line = start..line_end(self.text, start);
            let (at, count) = self.quotes_end(&line);
            if count < self.quotes {
                return None;
            }
            let rest = &self.text[at..line.end];
            let ws = rest.bytes().take_while(|b| *b == b' ' || *b == b'\t').count();
            if ws < rest.len() {
                return Some(columns(&rest[..ws]));
            }
            start = line.end + 1;
        }
        None
    }

    /// The tree of the list `block` that ends on line `last`. Its parent has its content at column `col`.
    fn list(&self, block: &Block, last: usize, col: usize) -> Option<List> {
        if !matches!(block.kind, BlockKind::List { .. }) {
            return None;
        }
        let items: Vec<&Block> = block.children.iter().filter(|b| is_item(b)).collect();
        let firsts: Vec<usize> = items.iter().map(|item| self.line_of(item.range.start)).collect();
        let mut built = Vec::with_capacity(items.len());
        for (k, item) in items.iter().enumerate() {
            let end = firsts.get(k + 1).map_or(last, |next| next - 1);
            built.push(self.item(item, firsts[k]..=end, k == 0, col)?);
        }
        Some(List {
            mark: built.first()?.mark,
            renumber: false,
            items: built,
        })
    }

    fn item(&self, block: &Block, lines: std::ops::RangeInclusive<usize>, first: bool, col: usize) -> Option<Item> {
        let (head, last) = (*lines.start(), *lines.end());
        let line = self.lines[head].clone();
        let prefix_end = self.prefix_end(head);
        // The range of an item can start in its indent.
        let from = block.range.start.max(prefix_end);
        let at = from
            + self.text[from..line.end.max(from)]
                .bytes()
                .take_while(|b| *b == b' ' || *b == b'\t')
                .count();
        if !(prefix_end..line.end).contains(&at) {
            return None;
        }
        let lead = &self.text[prefix_end..at];
        if !lead.bytes().all(|b| b == b' ' || b == b'\t') {
            return None;
        }
        let indent = columns(lead);
        let rel = indent.checked_sub(col)?;
        let (mark, number, width) = marker(&self.text[at..line.end])?;
        let after = &self.text[at + width..line.end];
        let gap = gap(after)?;
        let content = indent + width + gap;
        let mut parts = Vec::new();
        let mut next = head + 1;
        for child in block
            .children
            .iter()
            .filter(|b| matches!(b.kind, BlockKind::List { .. }))
        {
            let start = self.line_of(child.range.start);
            let end = self.line_of(list_end(child)).min(last);
            if start < next {
                return None;
            }
            parts.extend((next..start).map(|l| Part::Line(self.line(l, content))));
            parts.push(Part::List(self.list(child, end, content)?));
            next = end + 1;
        }
        parts.extend((next..=last).map(|l| Part::Line(self.line(l, content))));
        Some(Item {
            id: head,
            prefix: line.start..prefix_end,
            mark,
            number,
            first,
            rel,
            gap,
            kept: Src::Old(at + width..line.end),
            empty: no_text(after),
            parts,
        })
    }

    fn line(&self, line: usize, content: usize) -> Line {
        let range = self.lines[line].clone();
        let prefix_end = self.prefix_end(line);
        let rest = &self.text[prefix_end..range.end];
        let ws = rest.bytes().take_while(|b| *b == b' ' || *b == b'\t').count();
        let lead = columns(&rest[..ws]);
        let blank = ws == rest.len();
        let risky = !rest[ws..].starts_with(|c: char| c.is_alphabetic());
        let (extra, kept) = if !blank && lead >= content {
            (Some(lead - content), prefix_end + ws..range.end)
        } else {
            (None, prefix_end..range.end)
        };
        let starts_item = marker(&rest[ws..])
            .is_some_and(|(_, _, width)| rest[ws + width..].is_empty() || rest[ws + width..].starts_with([' ', '\t']));
        let after_text = line > 0 && {
            let above = self.prefix_end(line - 1);
            !self.text[above..self.lines[line - 1].end].trim().is_empty()
        };
        let continues = !blank && after_text && (starts_item || extra.is_some_and(|e| e >= 4));
        Line {
            prefix: range.start..prefix_end,
            extra,
            kept,
            blank,
            lead,
            risky,
            continues,
        }
    }

    /// The edit that puts the written tree in place of the region. Only the changed middle goes
    /// in the edit.
    fn finish(self, writer: Writer<'_>, tree: Vec<Part>, selection: Range<usize>) -> Plan {
        let old = &self.text[self.region.clone()];
        let new = writer.out;
        let mut prefix = old.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count();
        while !old.is_char_boundary(prefix) {
            prefix -= 1;
        }
        let max = old.len().min(new.len()) - prefix;
        let mut suffix = old
            .bytes()
            .rev()
            .zip(new.bytes().rev())
            .take(max)
            .take_while(|(a, b)| a == b)
            .count();
        while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
            suffix -= 1;
        }
        let start = self.region.start;
        let planned = Planned {
            parts: tree,
            old: self.region.clone(),
            new: start..start + new.len(),
            quotes: self.quotes,
        };
        Plan {
            edit: Edit {
                range: start + prefix..self.region.end - suffix,
                text: new[prefix..new.len() - suffix].to_owned(),
                selection,
            },
            tree: Some(planned),
        }
    }
}

/// Pushes the ids of the items that start on `lines` and have no such item above them.
fn top_items(parts: &[Part], lines: &std::ops::RangeInclusive<usize>, ids: &mut Vec<usize>) {
    for part in parts {
        if let Part::List(list) = part {
            for item in &list.items {
                if lines.contains(&item.id) {
                    ids.push(item.id);
                } else {
                    top_items(&item.parts, lines, ids);
                }
            }
        }
    }
}

/// The way to the item `id`: the part of each list and the item in it, from the top.
fn find(parts: &[Part], id: usize) -> Option<Vec<(usize, usize)>> {
    for (p, part) in parts.iter().enumerate() {
        if let Part::List(list) = part {
            for (i, item) in list.items.iter().enumerate() {
                if item.id == id {
                    return Some(vec![(p, i)]);
                }
                if let Some(mut path) = find(&item.parts, id) {
                    path.insert(0, (p, i));
                    return Some(path);
                }
            }
        }
    }
    None
}

/// The parts that hold the last list of `path`.
fn parts_mut<'p>(mut parts: &'p mut Vec<Part>, path: &[(usize, usize)]) -> Option<&'p mut Vec<Part>> {
    for &(p, i) in path {
        let Part::List(list) = parts.get_mut(p)? else {
            return None;
        };
        parts = &mut list.items.get_mut(i)?.parts;
    }
    Some(parts)
}

/// Tab on the item `id`. `None` when it is the first item of its list.
fn move_in(tree: &mut Vec<Part>, id: usize) -> Option<()> {
    let path = find(tree, id)?;
    let (&(p, i), up) = path.split_last()?;
    if i == 0 {
        return None;
    }
    let Part::List(list) = parts_mut(tree, up)?.get_mut(p)? else {
        return None;
    };
    let above = &list.items[i - 1];
    if ends_at_blank(above) && !above.parts.is_empty() {
        // An empty item ends at a blank line, so it cannot take children after one.
        return None;
    }
    let mut item = list.items.remove(i);
    list.renumber = true;
    let above = &mut list.items[i - 1];
    let blanks = trailing_blanks(&mut above.parts);
    match above.parts.last_mut() {
        Some(Part::List(child)) if child.mark.joins(item.mark) => {
            let tail = child.items.last_mut()?;
            tail.parts.extend(blanks);
            item.rel = tail.rel;
            item.mark = child.mark;
            child.items.push(item);
            child.renumber = true;
        }
        _ => {
            above.parts.extend(blanks);
            item.rel = 0;
            above.parts.push(Part::List(List {
                mark: item.mark,
                renumber: true,
                items: vec![item],
            }));
        }
    }
    Some(())
}

/// The columns from the end of a marker to the content column, for the marker line text `after`.
fn gap(after: &str) -> Option<usize> {
    let spaces = after.bytes().take_while(|b| *b == b' ').count();
    if spaces == after.len() || spaces > 4 {
        Some(1)
    } else if spaces == 0 {
        None
    } else {
        Some(spaces)
    }
}

/// Whether the marker line text `after` has no text past a task box.
fn no_text(after: &str) -> bool {
    let after = after.trim_start();
    let after = ["[ ]", "[x]", "[X]"]
        .iter()
        .find_map(|task| after.strip_prefix(task))
        .filter(|rest| rest.is_empty() || rest.starts_with(' '))
        .unwrap_or(after);
    after.trim().is_empty()
}

/// Whether `item` has no text, so a blank line ends it.
fn ends_at_blank(item: &Item) -> bool {
    item.empty && item.parts.iter().all(Part::is_blank)
}

/// Takes the blank lines at the end of `parts`.
fn trailing_blanks(parts: &mut Vec<Part>) -> Vec<Part> {
    let blanks = parts.iter().rev().take_while(|p| p.is_blank()).count();
    parts.split_off(parts.len() - blanks)
}

/// Shift+Tab on the item `id`. `None` when it is at the top level.
fn move_out(tree: &mut Vec<Part>, id: usize) -> Option<()> {
    let path = find(tree, id)?;
    let (&(p, i), up) = path.split_last()?;
    let (&(pp, pi), top) = up.split_last()?;
    let Part::List(outer) = parts_mut(tree, top)?.get_mut(pp)? else {
        return None;
    };
    let parent = outer.items.get_mut(pi)?;
    let rel = parent.rel;
    let Part::List(list) = parent.parts.get(p)? else {
        return None;
    };
    let gets = list.items.len() > i + 1 || parent.parts[p + 1..].iter().any(|p| !p.is_blank());
    let starts_blank = match list.items[i].parts.first() {
        Some(first) => first.is_blank(),
        None => list.items.len() == i + 1 && parent.parts.get(p + 1).is_some_and(Part::is_blank),
    };
    if gets && starts_blank && ends_at_blank(&list.items[i]) {
        return None;
    }
    let Some(Part::List(list)) = parent.parts.get_mut(p) else {
        return None;
    };
    let mut below = list.items.split_off(i);
    list.renumber = true;
    let (empty, mark) = (list.items.is_empty(), list.mark);
    let mut item = below.remove(0);
    let later = parent.parts.split_off(p + 1);
    if empty {
        parent.parts.remove(p);
    }
    if !below.is_empty() {
        let blanks = trailing_blanks(&mut item.parts);
        match item.parts.last_mut() {
            Some(Part::List(child)) if child.mark == mark => {
                child.items.last_mut()?.parts.extend(blanks);
                child.items.extend(below);
                child.renumber = true;
            }
            _ => {
                item.parts.extend(blanks);
                item.parts.push(Part::List(List {
                    mark,
                    renumber: true,
                    items: below,
                }));
            }
        }
    }
    for part in later {
        if let Part::List(mut list) = part {
            // A list after the text of its new parent must start at 1 to be a list.
            list.renumber = true;
            if let Some(head) = list.items.first_mut() {
                head.first = false;
            }
            item.parts.push(Part::List(list));
        } else {
            item.parts.push(part);
        }
    }
    item.rel = rel;
    outer.renumber = true;
    if outer.mark.joins(item.mark) {
        item.mark = outer.mark;
        outer.items.insert(pi + 1, item);
        return Some(());
    }
    let rest = outer.items.split_off(pi + 1);
    let outer_mark = outer.mark;
    let parts = parts_mut(tree, top)?;
    parts.insert(
        pp + 1,
        Part::List(List {
            mark: item.mark,
            renumber: true,
            items: vec![item],
        }),
    );
    if !rest.is_empty() {
        parts.insert(
            pp + 2,
            Part::List(List {
                mark: outer_mark,
                renumber: true,
                items: rest,
            }),
        );
    }
    Some(())
}

/// Joins each two lists with the same marker that only blank lines part. The parse reads them
/// as one list.
fn merge_lists(parts: &mut Vec<Part>) {
    for part in parts.iter_mut() {
        if let Part::List(list) = part {
            for item in &mut list.items {
                merge_lists(&mut item.parts);
            }
        }
    }
    let mut k = 0;
    while k < parts.len() {
        let Part::List(list) = &parts[k] else {
            k += 1;
            continue;
        };
        let mark = list.mark;
        let next = (k + 1..parts.len()).find(|&n| !parts[n].is_blank());
        let Some(n) = next.filter(|&n| matches!(&parts[n], Part::List(l) if l.mark == mark)) else {
            k += 1;
            continue;
        };
        let mut taken: Vec<Part> = parts.drain(k + 1..=n).collect();
        let Some(Part::List(second)) = taken.pop() else {
            k += 1;
            continue;
        };
        if let Part::List(first) = &mut parts[k] {
            if let Some(last) = first.items.last_mut() {
                last.parts.extend(taken);
            }
            first.items.extend(second.items);
            first.renumber = true;
        }
    }
}

/// Where a line of the old text went.
struct Placed {
    old_line: usize,
    /// Where its kept text starts. `None` for a new line.
    old_keep: Option<usize>,
    new_line: usize,
    new_keep: usize,
}

/// Writes a tree out as text.
struct Writer<'a> {
    text: &'a str,
    out: String,
    started: bool,
    keeps: Vec<Placed>,
    /// Whether a line would join another item than the tree says, like a lazy line that could
    /// start a block and lost its laziness.
    broken: bool,
    /// The content column of the last top item, when the tree ends with an item.
    last: Option<usize>,
    /// What the last line holds.
    above: Above,
    /// How many items hold the part that is being written.
    depth: usize,
    /// The content column of the top item that is being written.
    top: usize,
}

/// What a written line holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Above {
    /// No text.
    Nothing,
    Text,
    /// Lazy text. An empty item after it would not start an item.
    Lazy,
}

impl<'a> Writer<'a> {
    /// Writes `tree`. `after_text` tells whether a line with text comes right before it.
    fn write(text: &'a str, tree: &[Part], after_text: bool) -> Self {
        let mut writer = Self {
            text,
            out: String::new(),
            started: false,
            keeps: Vec::new(),
            broken: false,
            last: None,
            above: if after_text { Above::Lazy } else { Above::Nothing },
            depth: 0,
            top: 0,
        };
        writer.last = writer.parts(tree, 0);
        writer.keeps.sort_by_key(|keep| keep.old_line);
        writer
    }

    /// Writes `parts` at the content column `col`. Gives the content column of the last item,
    /// past `col`, when the parts end with an item.
    fn parts(&mut self, parts: &[Part], col: usize) -> Option<usize> {
        // The content column of the item above, past `col`. An item at it or past it would be
        // a child of that item. An empty item ends at a blank line, and then takes nothing.
        let mut above: Option<(usize, bool)> = None;
        for part in parts {
            match part {
                Part::Line(line) => {
                    if line.blank && above.is_some_and(|(_, empty)| empty) {
                        above = None;
                    }
                    if !line.blank {
                        if let (Some(extra), Some((above, _))) = (line.extra, above)
                            && extra >= above
                        {
                            // The line would join the item above.
                            self.broken = true;
                        }
                        above = None;
                    }
                    self.line(line, col);
                }
                Part::List(list) => {
                    for (item, number) in list.items.iter().zip(numbers(list)) {
                        let rel = above.map_or(item.rel, |(above, _)| item.rel.min(above - 1));
                        let content = self.item(item, rel, list.mark, number, col);
                        above = if ends_at_blank(item) && !item.parts.is_empty() {
                            None
                        } else {
                            Some((content, ends_at_blank(item)))
                        };
                    }
                }
            }
        }
        above.map(|(content, _)| content)
    }

    fn begin(&mut self, prefix: &Range<usize>) -> usize {
        if self.started {
            self.out.push('\n');
        }
        self.started = true;
        let start = self.out.len();
        self.out.push_str(&self.text[prefix.clone()]);
        start
    }

    fn spaces(&mut self, count: usize) {
        self.out.extend(std::iter::repeat_n(' ', count));
    }

    /// Writes `item` with its marker `rel` columns past `col`, and gives its content column past
    /// `col`.
    fn item(&mut self, item: &Item, rel: usize, mark: Mark, number: u64, col: usize) -> usize {
        if item.empty && self.above == Above::Lazy {
            self.broken = true;
        }
        self.above = if item.empty { Above::Nothing } else { Above::Text };
        let new_line = self.begin(&item.prefix);
        let indent = col + rel;
        self.spaces(indent);
        let marker = mark.text(number);
        self.out.push_str(&marker);
        let new_keep = self.out.len();
        let old_keep = match &item.kept {
            Src::Old(range) => {
                self.out.push_str(&self.text[range.clone()]);
                Some(range.start)
            }
            Src::New(text) => {
                self.out.push_str(text);
                None
            }
        };
        self.keeps.push(Placed {
            old_line: item.prefix.start,
            old_keep,
            new_line,
            new_keep,
        });
        let content = indent + marker.len() + item.gap;
        if self.depth == 0 {
            self.top = content;
        }
        self.depth += 1;
        self.parts(&item.parts, content);
        self.depth -= 1;
        content - col
    }

    fn line(&mut self, line: &Line, col: usize) {
        let new_line = self.begin(&line.prefix);
        if let Some(extra) = line.extra {
            self.spaces(col + extra);
        } else if !line.blank && line.risky && line.lead >= self.top {
            // Past the content column of the top item, the line would start a block in it.
            self.broken = true;
        }
        if line.continues && self.above == Above::Nothing {
            // Without the text above, the line would start a block.
            self.broken = true;
        }
        self.above = if line.blank {
            Above::Nothing
        } else if line.extra.is_none() {
            Above::Lazy
        } else {
            Above::Text
        };
        let new_keep = self.out.len();
        self.out.push_str(&self.text[line.kept.clone()]);
        self.keeps.push(Placed {
            old_line: line.prefix.start,
            old_keep: Some(line.kept.start),
            new_line,
            new_keep,
        });
    }

    /// The offset in the written text of `offset` in the old text, from the start of the region.
    fn map(&self, offset: usize) -> usize {
        let keeps: Vec<&Placed> = self.keeps.iter().filter(|k| k.old_keep.is_some()).collect();
        let at = keeps.partition_point(|k| k.old_line <= offset).saturating_sub(1);
        let Some(keep) = keeps.get(at) else {
            return 0;
        };
        let old_keep = keep.old_keep.unwrap_or(keep.old_line);
        if offset >= old_keep {
            keep.new_keep + offset - old_keep
        } else {
            keep.new_keep - (old_keep - offset).min(keep.new_keep - keep.new_line)
        }
    }
}

/// The number of each item of `list`.
fn numbers(list: &List) -> impl Iterator<Item = u64> + '_ {
    let start = list
        .items
        .first()
        .filter(|item| item.first)
        .map_or(1, |item| item.number);
    list.items
        .iter()
        .zip(start..)
        .map(|(item, count)| if list.renumber { count } else { item.number })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// A list item as the tests compare it: its marker, its text and its parts.
    #[derive(Debug, PartialEq, Eq)]
    enum Shape {
        List(Vec<(String, String, Vec<Shape>)>),
        Line(String),
    }

    fn shape(text: &str, parts: &[Part]) -> Vec<Shape> {
        parts
            .iter()
            .filter(|part| !part.is_blank())
            .map(|part| match part {
                Part::Line(line) => Shape::Line(text[line.kept.clone()].trim().to_owned()),
                Part::List(list) => Shape::List(
                    list.items
                        .iter()
                        .zip(numbers(list))
                        .map(|(item, number)| {
                            let kept = match &item.kept {
                                Src::Old(range) => &text[range.clone()],
                                Src::New(new) => new.as_str(),
                            };
                            (list.mark.text(number), kept.trim().to_owned(), shape(text, &item.parts))
                        })
                        .collect(),
                ),
            })
            .collect()
    }

    /// The tree of the lists that the parse of `text` has on the lines `lines`.
    fn parsed(text: &str, lines: &Range<usize>, quotes: usize) -> Result<Vec<Part>, String> {
        let body = Body::parse(text);
        let mut blocks = body.blocks();
        let overlaps = |b: &Block| {
            let end = if matches!(b.kind, BlockKind::List { .. }) {
                list_end(b)
            } else {
                b.range.end
            };
            b.range.start <= lines.end && lines.start <= end
        };
        for _ in 0..quotes {
            blocks = &blocks.iter().find(|b| overlaps(b)).ok_or("no quote")?.children;
        }
        let source = Source::lines(text, lines.clone(), quotes);
        let blocks: Vec<&Block> = blocks.iter().filter(|b| overlaps(b)).collect();
        if let Some(block) = blocks.iter().find(|b| !matches!(b.kind, BlockKind::List { .. })) {
            return Err(format!("not a list: {:?}", block.kind));
        }
        source.tree(&blocks).ok_or_else(|| "no tree".to_owned())
    }

    /// Whether a list after the first starts with an empty item. Markdown cannot write it, since
    /// such an item cannot start a list after a paragraph. The parser lets an empty bullet
    /// through one level down, but no deeper. The key writes it anyway, and it becomes a list once
    /// the item has text.
    fn has_empty_head(text: &str, parts: &[Part], top: bool) -> bool {
        parts.iter().enumerate().any(|(k, part)| {
            let Part::List(list) = part else {
                return false;
            };
            let head = &list.items[0];
            let empty = matches!(&head.kept, Src::Old(r) if text[r.clone()].trim().is_empty())
                || matches!(&head.kept, Src::New(new) if new.trim().is_empty());
            let empty_head = empty && !(top && k == 0);
            empty_head || list.items.iter().any(|item| has_empty_head(text, &item.parts, false))
        })
    }

    /// The text of each line without its quote markers, indent and list marker.
    fn words(text: &str, lines: &Range<usize>) -> Vec<String> {
        text[lines.clone()]
            .lines()
            .map(|line| {
                let line = line.trim_start_matches(['>', ' ', '\t']);
                let rest = marker(line).map_or(line, |(_, _, width)| &line[width..]);
                rest.trim().to_owned()
            })
            .filter(|line| !line.is_empty())
            .collect()
    }

    const MARKS: [&str; 4] = ["-", "*", ".", ")"];
    const TEXTS: [&str; 5] = ["a", "bb", "", "[ ] t", "c d"];

    /// A text of list lines: level, marker, text, a second line (none, indented, lazy), a blank
    /// line after, and the spaces before the marker.
    fn list_text(lines: &[(usize, usize, usize, usize, bool, usize)], start: u64, quote: bool) -> String {
        let mut out = Vec::new();
        let mut cols: Vec<usize> = Vec::new();
        let mut counts: Vec<u64> = Vec::new();
        for &(level, mark, text, more, blank, pad) in lines {
            let level = level.min(cols.len());
            cols.truncate(level);
            counts.truncate(level + 1);
            if counts.len() <= level {
                counts.push(start);
            }
            let col = cols.last().copied().unwrap_or(0) + pad;
            let marker = match MARKS[mark] {
                bullet @ ("-" | "*") => bullet.to_owned(),
                delimiter => format!("{}{delimiter}", counts[level]),
            };
            counts[level] += 1;
            out.push(format!("{}{marker} {}", " ".repeat(col), TEXTS[text]));
            let content = col + marker.len() + 1;
            match more {
                1 => out.push(format!("{}more", " ".repeat(content))),
                2 => out.push("lazy".to_owned()),
                _ => {}
            }
            if blank {
                out.push(String::new());
            }
            cols.push(content);
        }
        let out: Vec<String> = if quote {
            out.iter()
                .map(|line| format!("> {line}").trim_end().to_owned())
                .collect()
        } else {
            out
        };
        out.join("\n")
    }

    proptest! {
        /// The parse after the edit gives the tree that the key planned, and the text of each
        /// line stays.
        #[test]
        fn the_parse_after_a_key_has_the_planned_tree(
            lines in proptest::collection::vec(
                (0..4usize, 0..MARKS.len(), 0..TEXTS.len(), 0..4usize, any::<bool>(), prop_oneof![4 => Just(0usize), 1 => 1..3usize]),
                1..8,
            ),
            start in 1..12u64,
            quote in any::<bool>(),
            ends in (0..400usize, 0..400usize),
            key in 0..3usize,
            head in any::<bool>(),
        ) {
            let text = list_text(&lines, start, quote);
            let text = if head { format!("Some text\n\n{text}") } else { text };
            let (a, b) = (ends.0.min(text.len()), ends.1.min(text.len()));
            let selection = a.min(b)..a.max(b);
            let key = [Move::In, Move::Out, Move::Next][key];
            let body = Body::parse(&text);
            let Some(plan) = plan(&text, &body, selection, key) else {
                return Ok(());
            };
            let mut new = text.clone();
            new.replace_range(plan.edit.range.clone(), &plan.edit.text);
            prop_assert!(new.is_char_boundary(plan.edit.selection.start));
            prop_assert!(plan.edit.selection.end <= new.len());
            let Some(planned) = plan.tree else {
                return Ok(());
            };
            if has_empty_head(&text, &planned.parts, true) {
                return Ok(());
            }
            let after = parsed(&new, &planned.new, planned.quotes);
            prop_assert!(after.is_ok(), "{:?} in {new:?} after {key:?} on {text:?}", after.err());
            let after = after.unwrap_or_default();
            prop_assert_eq!(
                shape(&text, &planned.parts),
                shape(&new, &after),
                "{:?} on {:?} gives {:?}", key, text, new
            );
            if key != Move::Next {
                prop_assert_eq!(words(&text, &planned.old), words(&new, &planned.new));
            }
        }
    }
}
