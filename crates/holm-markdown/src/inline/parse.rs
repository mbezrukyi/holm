//! Holm's inline parser. It follows the `CommonMark` inline rules. It adds `~` and `~~` from
//! GFM and `==` as delimiters. Holm's HTML tags become style nodes.

use std::collections::HashMap;
use std::ops::Range;

use super::html_tag::{self, HtmlTag};
use super::scan::{self, InlineLink};
use super::{Destination, Kind, Node};
use crate::body::link;

/// The link reference definitions of a note: each normalized label with its destination.
#[derive(Clone, Debug, Default)]
pub(crate) struct Defs(HashMap<String, String>);

impl Defs {
    pub(crate) fn insert(&mut self, label: &str, dest: String) {
        self.0.entry(scan::normalize_label(label)).or_insert(dest);
    }

    fn get(&self, label: &str) -> Option<&str> {
        self.0.get(&scan::normalize_label(label)).map(String::as_str)
    }
}

/// Parses the text of one leaf block into its inline nodes.
pub(crate) fn parse(text: &str, defs: &Defs) -> Vec<Node> {
    parse_with(text, defs, &mut Scratch::default())
}

/// The buffers of the parser, kept from one leaf to the next.
#[derive(Default)]
pub(crate) struct Scratch {
    slots: Vec<Slot>,
    delims: Vec<usize>,
    brackets: Vec<usize>,
    no_closer: Vec<usize>,
}

/// Parses like [`parse`], with buffers from an earlier parse.
pub(crate) fn parse_with(text: &str, defs: &Defs, scratch: &mut Scratch) -> Vec<Node> {
    if is_plain(text) {
        return if text.is_empty() {
            Vec::new()
        } else {
            vec![text_node(0..text.len())]
        };
    }
    let mut slots = std::mem::take(&mut scratch.slots);
    slots.clear();
    slots.push(Slot {
        item: Item::Gone,
        prev: NONE,
        next: NONE,
    });
    let mut parser = Parser {
        text,
        defs,
        slots,
        tail: HEAD,
        delims: std::mem::take(&mut scratch.delims),
        brackets: std::mem::take(&mut scratch.brackets),
        no_closer: std::mem::take(&mut scratch.no_closer),
    };
    parser.delims.clear();
    parser.brackets.clear();
    parser.no_closer.clear();
    parser.run();
    parser.process_emphasis(0);
    let nodes = parser.take_after(HEAD, NONE);
    scratch.slots = parser.slots;
    scratch.delims = parser.delims;
    scratch.brackets = parser.brackets;
    scratch.no_closer = parser.no_closer;
    finish(text, nodes)
}

/// Whether `text` has no inline syntax, so its tree is one text node. A line break counts only
/// after two spaces.
pub(crate) fn is_plain(text: &str) -> bool {
    let b = text.as_bytes();
    b.iter().enumerate().all(|(i, &c)| match c {
        b'\n' => !b[..i].ends_with(b"  "),
        c => !is_special(c),
    })
}

fn is_special(c: u8) -> bool {
    SPECIAL[usize::from(c)]
}

/// The bytes that can start inline syntax. A table is faster than a match in the plain text
/// scan, which runs over the whole note.
static SPECIAL: [bool; 256] = {
    let mut table = [false; 256];
    let chars = b"\\`*_~=[]<\n";
    let mut i = 0;
    while i < chars.len() {
        table[chars[i] as usize] = true;
        i += 1;
    }
    table
};

const HEAD: usize = 0;
const NONE: usize = usize::MAX;

enum Item {
    Node(Node),
    Delim(Delim),
    /// `[` or `![`.
    Bracket(Bracket),
    Gone,
}

struct Delim {
    ch: u8,
    /// The chars of the run that are not used yet.
    range: Range<usize>,
    /// The length of the whole run.
    len: usize,
    open: bool,
    close: bool,
    dead: bool,
}

struct Bracket {
    image: bool,
    start: usize,
    active: bool,
    /// How many delimiters stood before it.
    delims: usize,
}

struct Slot {
    item: Item,
    prev: usize,
    next: usize,
}

struct Parser<'a> {
    text: &'a str,
    defs: &'a Defs,
    /// A linked list of items. Slot 0 is its head and holds no item.
    slots: Vec<Slot>,
    tail: usize,
    /// The delimiter slots, in order.
    delims: Vec<usize>,
    /// The open bracket slots, in order.
    brackets: Vec<usize>,
    /// The backtick run lengths with no closing run after the last search.
    no_closer: Vec<usize>,
}

impl Parser<'_> {
    fn run(&mut self) {
        let b = self.text.as_bytes();
        let mut p = 0;
        let mut text_start = 0;
        while p < b.len() {
            match b[p] {
                b'\\' => match b.get(p + 1) {
                    Some(b'\n') => {
                        self.flush(text_start..p);
                        self.push_node(Kind::HardBreak, p..p + 2, p..p);
                        p += 2;
                        text_start = p;
                    }
                    Some(c) if c.is_ascii_punctuation() => p += 2,
                    _ => p += 1,
                },
                b'`' => {
                    let n = run_len(b, p);
                    if let Some(close) = self.closing_ticks(p + n, n) {
                        self.flush(text_start..p);
                        let content = code_content(b, p + n..close);
                        self.push_node(Kind::Code, p..close + n, content);
                        p = close + n;
                        text_start = p;
                    } else {
                        p += n;
                    }
                }
                c @ (b'*' | b'_' | b'~' | b'=') => {
                    let n = run_len(b, p);
                    let is_delim = match c {
                        b'~' => n <= 2,
                        b'=' => n == 2,
                        _ => true,
                    };
                    if is_delim {
                        self.flush(text_start..p);
                        self.push_delim(c, p..p + n);
                        text_start = p + n;
                    }
                    p += n;
                }
                b'[' => {
                    self.flush(text_start..p);
                    self.push_bracket(false, p);
                    p += 1;
                    text_start = p;
                }
                b'!' if b.get(p + 1) == Some(&b'[') => {
                    self.flush(text_start..p);
                    self.push_bracket(true, p);
                    p += 2;
                    text_start = p;
                }
                b']' => {
                    if let Some(end) = self.close_bracket(text_start, p) {
                        p = end;
                        text_start = p;
                    } else {
                        p += 1;
                    }
                }
                b'<' => {
                    if let Some((end, email)) = scan::autolink(self.text, p) {
                        self.flush(text_start..p);
                        self.push_autolink(p..end, email);
                        p = end;
                        text_start = p;
                    } else if let Some(end) = scan::raw_html(self.text, p) {
                        self.flush(text_start..p);
                        self.push_node(Kind::Html, p..end, p..end);
                        p = end;
                        text_start = p;
                    } else {
                        p += 1;
                    }
                }
                b'\n' => {
                    let spaces = b[text_start..p].iter().rev().take_while(|&&c| c == b' ').count();
                    if spaces >= 2 {
                        self.flush(text_start..p - spaces);
                        self.push_node(Kind::HardBreak, p - spaces..p + 1, p..p);
                        text_start = p + 1;
                    }
                    p += 1;
                }
                _ => p += 1,
            }
        }
        self.flush(text_start..b.len());
    }

    fn push(&mut self, item: Item) -> usize {
        let at = self.slots.len();
        self.slots.push(Slot {
            item,
            prev: self.tail,
            next: NONE,
        });
        self.slots[self.tail].next = at;
        self.tail = at;
        at
    }

    fn push_node(&mut self, kind: Kind, source: Range<usize>, content: Range<usize>) {
        self.push(Item::Node(Node {
            kind,
            source,
            content,
            children: Vec::new(),
        }));
    }

    fn flush(&mut self, range: Range<usize>) {
        if !range.is_empty() {
            self.push_node(Kind::Text, range.clone(), range);
        }
    }

    fn push_delim(&mut self, ch: u8, range: Range<usize>) {
        let before = self.text[..range.start].chars().next_back();
        let after = self.text[range.end..].chars().next();
        let space = |c: Option<char>| c.is_none_or(char::is_whitespace);
        let punct = |c: Option<char>| c.is_some_and(scan::is_punctuation);
        let left = !space(after) && (!punct(after) || space(before) || punct(before));
        let right = !space(before) && (!punct(before) || space(after) || punct(after));
        let (open, close) = if ch == b'_' {
            (left && (!right || punct(before)), right && (!left || punct(after)))
        } else {
            (left, right)
        };
        let len = range.len();
        let at = self.push(Item::Delim(Delim {
            ch,
            range,
            len,
            open,
            close,
            dead: false,
        }));
        self.delims.push(at);
    }

    fn push_bracket(&mut self, image: bool, start: usize) {
        let delims = self.delims.len();
        let at = self.push(Item::Bracket(Bracket {
            image,
            start,
            active: true,
            delims,
        }));
        self.brackets.push(at);
    }

    fn push_autolink(&mut self, source: Range<usize>, email: bool) {
        let content = source.start + 1..source.end - 1;
        let dest = &self.text[content.clone()];
        let target = if email {
            Some(link::Target::Web(format!("mailto:{dest}")))
        } else {
            link::target(dest)
        };
        let destination = Destination {
            target,
            range: Some(content.clone()),
        };
        self.push(Item::Node(Node {
            kind: Kind::Link(Box::new(destination)),
            source,
            content: content.clone(),
            children: vec![Node {
                kind: Kind::Text,
                source: content.clone(),
                content,
                children: Vec::new(),
            }],
        }));
    }

    /// The start of the next run of exactly `n` backticks at or after `from`.
    fn closing_ticks(&mut self, from: usize, n: usize) -> Option<usize> {
        if self.no_closer.contains(&n) {
            return None;
        }
        let b = self.text.as_bytes();
        let mut i = from;
        while i < b.len() {
            if b[i] == b'`' {
                let len = run_len(b, i);
                if len == n {
                    return Some(i);
                }
                i += len;
            } else {
                i += 1;
            }
        }
        self.no_closer.push(n);
        None
    }

    /// Handles a `]` at `p`. Gives the end of the link it closes, or `None` when the `]` is text.
    fn close_bracket(&mut self, text_start: usize, p: usize) -> Option<usize> {
        let &at = self.brackets.last()?;
        let Item::Bracket(bracket) = &self.slots[at].item else {
            self.brackets.pop();
            return None;
        };
        let (image, start, active, delims) = (bracket.image, bracket.start, bracket.active, bracket.delims);
        if !active {
            self.brackets.pop();
            return None;
        }
        let content = start + if image { 2 } else { 1 }..p;
        let Some((end, destination)) = self.link_after(content.clone(), p) else {
            self.brackets.pop();
            return None;
        };
        self.flush(text_start..p);
        self.process_emphasis(delims);
        let children = finish(self.text, self.take_after(at, NONE));
        let kind = if image {
            Kind::Image(Box::new(destination))
        } else {
            Kind::Link(Box::new(destination))
        };
        self.slots[at].item = Item::Node(Node {
            kind,
            source: start..end,
            content,
            children,
        });
        self.brackets.pop();
        if !image {
            for &earlier in &self.brackets {
                if let Item::Bracket(b) = &mut self.slots[earlier].item
                    && !b.image
                {
                    b.active = false;
                }
            }
        }
        Some(end)
    }

    /// The link that follows the link text `content` and its `]` at `p`.
    fn link_after(&self, content: Range<usize>, p: usize) -> Option<(usize, Destination)> {
        let text = self.text;
        let b = text.as_bytes();
        if b.get(p + 1) == Some(&b'(')
            && let Some(InlineLink { end, dest }) = scan::inline_link(text, p + 1)
        {
            let written = &text[dest.clone()];
            let bare = written
                .strip_prefix('<')
                .and_then(|d| d.strip_suffix('>'))
                .unwrap_or(written);
            let destination = Destination {
                target: link::target(&scan::unescape(bare)),
                range: Some(dest),
            };
            return Some((end, destination));
        }
        let (label, end) = if b.get(p + 1) == Some(&b'[') {
            if b.get(p + 2) == Some(&b']') {
                (content, p + 3)
            } else {
                match scan::link_label(text, p + 1) {
                    Some(end) => (p + 2..end - 1, end),
                    None => (content, p + 1),
                }
            }
        } else {
            (content, p + 1)
        };
        let label = &text[label];
        if !scan::is_label(label) {
            return None;
        }
        let dest = self.defs.get(label)?;
        let destination = Destination {
            target: link::target(dest),
            range: None,
        };
        Some((end, destination))
    }

    /// Matches the delimiters after the first `bottom` ones into style nodes.
    fn process_emphasis(&mut self, bottom: usize) {
        // For each kind of closer: the lowest delimiter index to look at for an opener.
        let mut lows = [bottom; 24];
        let mut pos = bottom;
        while pos < self.delims.len() {
            let ci = self.delims[pos];
            let (ch, len, cur, open, close) = {
                let c = self.delim(ci);
                (c.ch, c.len, c.range.len(), c.open, c.close)
            };
            if self.delim(ci).dead || !close {
                pos += 1;
                continue;
            }
            let key = match ch {
                b'*' => usize::from(open) * 3 + len % 3,
                b'_' => 6 + usize::from(open) * 3 + len % 3,
                b'~' => 12 + cur.min(2),
                _ => 18 + cur.min(2),
            };
            let low = lows[key];
            let found = (low..pos).rev().find(|&j| {
                let o = self.delim(self.delims[j]);
                !o.dead && o.ch == ch && o.open && matches(o, self.delim(ci))
            });
            let Some(j) = found else {
                lows[key] = pos;
                if !open {
                    self.kill(ci);
                }
                pos += 1;
                continue;
            };
            let oi = self.delims[j];
            let n = match ch {
                b'*' | b'_' if self.delim(oi).range.len() >= 2 && cur >= 2 => 2,
                b'*' | b'_' => 1,
                _ => cur,
            };
            for k in j + 1..pos {
                self.kill(self.delims[k]);
            }
            let o_end = self.delim(oi).range.end;
            let c_start = self.delim(ci).range.start;
            let kind = match (ch, n) {
                (b'*' | b'_', 1) => Kind::Emphasis,
                (b'*' | b'_', _) => Kind::Strong,
                (b'~', _) => Kind::Strikethrough,
                _ => Kind::Highlight(None),
            };
            let children = finish(self.text, self.take_after(oi, ci));
            let node = Node {
                kind,
                source: o_end - n..c_start + n,
                content: o_end..c_start,
                children,
            };
            self.insert_after(oi, Item::Node(node));
            if self.use_chars(oi, n, true) {
                self.unlink(oi);
            }
            if self.use_chars(ci, n, false) {
                self.unlink(ci);
                pos += 1;
            }
        }
        for k in bottom..self.delims.len() {
            self.kill(self.delims[k]);
        }
        self.delims.truncate(bottom);
    }

    fn delim(&self, at: usize) -> &Delim {
        match &self.slots[at].item {
            Item::Delim(d) => d,
            _ => &GONE,
        }
    }

    /// Uses `n` chars at the inner end of the delimiter at `at`. Gives whether it is used up.
    fn use_chars(&mut self, at: usize, n: usize, opener: bool) -> bool {
        let Item::Delim(d) = &mut self.slots[at].item else {
            return true;
        };
        if opener {
            d.range.end -= n;
        } else {
            d.range.start += n;
        }
        d.dead |= d.range.is_empty();
        d.range.is_empty()
    }

    fn kill(&mut self, at: usize) {
        if let Item::Delim(d) = &mut self.slots[at].item {
            d.dead = true;
        }
    }

    fn insert_after(&mut self, at: usize, item: Item) {
        let next = self.slots[at].next;
        let new = self.slots.len();
        self.slots.push(Slot { item, prev: at, next });
        self.slots[at].next = new;
        if next == NONE {
            self.tail = new;
        } else {
            self.slots[next].prev = new;
        }
    }

    fn unlink(&mut self, at: usize) {
        let Slot { prev, next, .. } = self.slots[at];
        self.slots[prev].next = next;
        if next == NONE {
            self.tail = prev;
        } else {
            self.slots[next].prev = prev;
        }
        self.slots[at].item = Item::Gone;
    }

    /// Takes the items between `from` and `to` out of the list, as nodes. `NONE` is the end.
    fn take_after(&mut self, from: usize, to: usize) -> Vec<Node> {
        let mut nodes = Vec::new();
        let mut at = self.slots[from].next;
        while at != to {
            let slot = &mut self.slots[at];
            match std::mem::replace(&mut slot.item, Item::Gone) {
                Item::Node(node) => nodes.push(node),
                Item::Delim(d) if !d.range.is_empty() => nodes.push(text_node(d.range)),
                Item::Bracket(b) => {
                    let len = if b.image { 2 } else { 1 };
                    nodes.push(text_node(b.start..b.start + len));
                }
                Item::Delim(_) | Item::Gone => {}
            }
            at = slot.next;
        }
        self.slots[from].next = to;
        if to == NONE {
            self.tail = from;
        } else {
            self.slots[to].prev = from;
        }
        nodes
    }
}

static GONE: Delim = Delim {
    ch: 0,
    range: 0..0,
    len: 0,
    open: false,
    close: false,
    dead: true,
};

/// Whether an opener and a closer of the same char can match.
fn matches(o: &Delim, c: &Delim) -> bool {
    match c.ch {
        b'*' | b'_' => {
            let odd = (o.close || c.open) && (o.len + c.len).is_multiple_of(3);
            !odd || (o.len.is_multiple_of(3) && c.len.is_multiple_of(3))
        }
        _ => o.range.len() == c.range.len(),
    }
}

fn run_len(b: &[u8], start: usize) -> usize {
    b[start..].iter().take_while(|&&c| c == b[start]).count()
}

/// The content of a code span. When both ends have a space or a line break, one comes off each
/// end. Code of only spaces stays as it is.
fn code_content(b: &[u8], inner: Range<usize>) -> Range<usize> {
    let space = |c: u8| matches!(c, b' ' | b'\n');
    let code = &b[inner.clone()];
    if code.len() >= 2 && space(code[0]) && space(code[code.len() - 1]) && !code.iter().all(|&c| space(c)) {
        inner.start + 1..inner.end - 1
    } else {
        inner
    }
}

fn text_node(range: Range<usize>) -> Node {
    Node {
        kind: Kind::Text,
        source: range.clone(),
        content: range,
        children: Vec::new(),
    }
}

/// Joins text nodes that touch, then pairs Holm's HTML tags into style nodes.
fn finish(text: &str, mut nodes: Vec<Node>) -> Vec<Node> {
    nodes.dedup_by(|next, last| {
        let touch =
            matches!((&last.kind, &next.kind), (Kind::Text, Kind::Text)) && last.source.end == next.source.start;
        if touch {
            last.source.end = next.source.end;
            last.content.end = next.source.end;
        }
        touch
    });
    pair_tags(text, nodes)
}

/// Pairs Holm's HTML tags among sibling nodes. Tags that cross another node do not pair. Both
/// stay raw HTML.
fn pair_tags(text: &str, nodes: Vec<Node>) -> Vec<Node> {
    if !nodes.iter().any(|n| matches!(n.kind, Kind::Html)) {
        return nodes;
    }
    let mut out: Vec<Node> = Vec::with_capacity(nodes.len());
    let mut open: Vec<(&'static str, usize, Kind)> = Vec::new();
    for node in nodes {
        if !matches!(node.kind, Kind::Html) {
            out.push(node);
            continue;
        }
        match html_tag::read(&text[node.source.clone()]) {
            Some(HtmlTag::Open(name, kind)) => {
                open.push((name, out.len(), kind));
                out.push(node);
            }
            Some(HtmlTag::Close(name)) => {
                let Some(k) = open.iter().rposition(|(n, _, _)| *n == name) else {
                    out.push(node);
                    continue;
                };
                let (_, at, kind) = open.swap_remove(k);
                open.truncate(k);
                let children: Vec<Node> = out.drain(at + 1..).collect();
                let Some(tag) = out.pop() else {
                    continue;
                };
                out.push(Node {
                    kind,
                    source: tag.source.start..node.source.end,
                    content: tag.source.end..node.source.start,
                    children,
                });
            }
            None => out.push(node),
        }
    }
    out
}
