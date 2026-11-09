//! Scanners for the inline syntax that has a fixed form: autolinks, raw HTML, link labels,
//! link destinations and titles. Each takes the text and the offset of its first char.

use std::ops::Range;

/// An inline link after its `]`: `(dest "title")`.
pub(super) struct InlineLink {
    /// The offset after the `)`.
    pub(super) end: usize,
    /// The destination as written, with its `< >`.
    pub(super) dest: Range<usize>,
}

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')
}

fn skip_spaces(b: &[u8], mut i: usize) -> usize {
    while b.get(i).copied().is_some_and(is_space) {
        i += 1;
    }
    i
}

/// Whether `c` is punctuation for the delimiter rules: an ASCII punctuation char, or a char
/// of the Unicode P or S classes. The Unicode part covers the common blocks only.
pub(super) fn is_punctuation(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_punctuation();
    }
    matches!(u32::from(c),
        0xA1..=0xA9 | 0xAB | 0xAC | 0xAE..=0xB1 | 0xB4 | 0xB6..=0xB8 | 0xBB | 0xBF | 0xD7 | 0xF7
        | 0x2C2..=0x2C5 | 0x2D2..=0x2DF | 0x37E | 0x387
        | 0x55A..=0x55F | 0x589 | 0x58A | 0x5BE | 0x5C0 | 0x5C3 | 0x5C6 | 0x5F3 | 0x5F4
        | 0x609..=0x60D | 0x61B | 0x61D..=0x61F | 0x66A..=0x66D | 0x6D4
        | 0x964 | 0x965 | 0x970 | 0xE3F | 0xE4F | 0xE5A | 0xE5B
        | 0x2010..=0x2027 | 0x2030..=0x205E | 0x207A..=0x207E | 0x208A..=0x208E
        | 0x20A0..=0x20C0 | 0x2190..=0x2426 | 0x2440..=0x244A | 0x2500..=0x2775
        | 0x2794..=0x2BFF | 0x2E00..=0x2E7F
        | 0x3001..=0x3003 | 0x3008..=0x3020 | 0x3030 | 0x303D | 0x30A0 | 0x30FB
        | 0xFE10..=0xFE19 | 0xFE30..=0xFE6B
        | 0xFF01..=0xFF0F | 0xFF1A..=0xFF20 | 0xFF3B..=0xFF40 | 0xFF5B..=0xFF65 | 0xFFE0..=0xFFEE
        | 0x1F000..=0x1FAFF
    )
}

/// An autolink like `<https://a.b>` or `<me@a.b>` at `start`. Gives its end and whether it is
/// an email address.
pub(super) fn autolink(text: &str, start: usize) -> Option<(usize, bool)> {
    let b = text.as_bytes();
    uri_autolink(b, start)
        .map(|end| (end, false))
        .or_else(|| email_autolink(b, start).map(|end| (end, true)))
}

fn uri_autolink(b: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    if !b.get(i)?.is_ascii_alphabetic() {
        return None;
    }
    while b
        .get(i)
        .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-'))
    {
        i += 1;
    }
    let scheme = i - start - 1;
    if !(2..=32).contains(&scheme) || b.get(i) != Some(&b':') {
        return None;
    }
    i += 1;
    loop {
        match *b.get(i)? {
            b'>' => return Some(i + 1),
            b'<' | b'\x00'..=b' ' | b'\x7f' => return None,
            _ => i += 1,
        }
    }
}

fn email_autolink(b: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    let local = i;
    while b
        .get(i)
        .is_some_and(|c| c.is_ascii_alphanumeric() || b".!#$%&'*+/=?^_`{|}~-".contains(c))
    {
        i += 1;
    }
    if i == local || b.get(i) != Some(&b'@') {
        return None;
    }
    i += 1;
    loop {
        let label = i;
        while b
            .get(i)
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'-')
        {
            i += 1;
        }
        let len = i - label;
        if len == 0 || len > 63 || b[label] == b'-' || b[i - 1] == b'-' {
            return None;
        }
        match b.get(i)? {
            b'.' => i += 1,
            b'>' => return Some(i + 1),
            _ => return None,
        }
    }
}

/// Raw inline HTML at `start`: a tag, a comment, a processing instruction, a declaration or a
/// CDATA section. Gives its end.
pub(super) fn raw_html(text: &str, start: usize) -> Option<usize> {
    let b = text.as_bytes();
    let rest = &text[start..];
    let after = |pattern: &str, from: usize| {
        text[from..]
            .find(pattern)
            .map(|at| from + at + pattern.len())
    };
    match *b.get(start + 1)? {
        b'!' => {
            if rest.starts_with("<!-->") {
                Some(start + 5)
            } else if rest.starts_with("<!--->") {
                Some(start + 6)
            } else if rest.starts_with("<!--") {
                after("-->", start + 4)
            } else if rest.starts_with("<![CDATA[") {
                after("]]>", start + 9)
            } else if b.get(start + 2)?.is_ascii_alphabetic() {
                after(">", start + 2)
            } else {
                None
            }
        }
        b'?' => after("?>", start + 2),
        b'/' => {
            let i = tag_name(b, start + 2)?;
            let i = skip_spaces(b, i);
            (b.get(i) == Some(&b'>')).then_some(i + 1)
        }
        _ => open_tag(b, start),
    }
}

fn tag_name(b: &[u8], start: usize) -> Option<usize> {
    if !b.get(start)?.is_ascii_alphabetic() {
        return None;
    }
    let mut i = start + 1;
    while b
        .get(i)
        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'-')
    {
        i += 1;
    }
    Some(i)
}

fn open_tag(b: &[u8], start: usize) -> Option<usize> {
    let mut i = tag_name(b, start + 1)?;
    loop {
        let spaced = skip_spaces(b, i);
        let name_start = |c: &u8| c.is_ascii_alphabetic() || matches!(c, b'_' | b':');
        if spaced == i || !b.get(spaced).is_some_and(name_start) {
            i = spaced;
            break;
        }
        i = spaced + 1;
        while b
            .get(i)
            .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b':' | b'-'))
        {
            i += 1;
        }
        let eq = skip_spaces(b, i);
        if b.get(eq) == Some(&b'=') {
            i = attribute_value(b, skip_spaces(b, eq + 1))?;
        }
    }
    if b.get(i) == Some(&b'/') {
        i += 1;
    }
    (b.get(i) == Some(&b'>')).then_some(i + 1)
}

fn attribute_value(b: &[u8], start: usize) -> Option<usize> {
    let quote = *b.get(start)?;
    if matches!(quote, b'"' | b'\'') {
        let len = b[start + 1..].iter().position(|&c| c == quote)?;
        return Some(start + 1 + len + 1);
    }
    let len = b[start..]
        .iter()
        .take_while(|&&c| !is_space(c) && !b"\"'=<>`".contains(&c))
        .count();
    (len > 0).then_some(start + len)
}

/// A link label `[label]` at `start`. Gives its end, after the `]`.
pub(super) fn link_label(text: &str, start: usize) -> Option<usize> {
    let b = text.as_bytes();
    let mut i = start + 1;
    loop {
        match *b.get(i)? {
            b'\\' if b.get(i + 1).is_some_and(u8::is_ascii_punctuation) => i += 2,
            b'[' => return None,
            b']' => break,
            _ => i += 1,
        }
    }
    let label = &text[start + 1..i];
    (is_label(label)).then_some(i + 1)
}

/// Whether `label` can be the inside of a link label.
pub(super) fn is_label(label: &str) -> bool {
    let b = label.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'[' | b']' => return false,
            _ => i += 1,
        }
    }
    label.chars().count() <= 999 && !label.trim().is_empty()
}

/// The form of a label that two labels share when they match: lower case, with each run of
/// spaces as one space. `ß` becomes `ss`, as in Unicode case folding.
pub(super) fn normalize_label(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .replace('ß', "ss")
}

/// An inline link destination and title after `](`, from the `(` at `start`.
pub(super) fn inline_link(text: &str, start: usize) -> Option<InlineLink> {
    let b = text.as_bytes();
    let mut i = skip_spaces(b, start + 1);
    let dest_start = i;
    if b.get(i) == Some(&b'<') {
        i += 1;
        loop {
            match *b.get(i)? {
                b'>' => break,
                b'\\' => i += 2,
                b'<' | b'\n' | b'\r' => return None,
                _ => i += 1,
            }
        }
        i += 1;
    } else {
        let mut depth = 0_usize;
        while let Some(&c) = b.get(i) {
            match c {
                b'\\' if b.get(i + 1).is_some_and(u8::is_ascii_punctuation) => i += 1,
                b'(' => {
                    depth += 1;
                    if depth > 32 {
                        return None;
                    }
                }
                b')' if depth == 0 => break,
                b')' => depth -= 1,
                b'\x00'..=b' ' | b'\x7f' => break,
                _ => {}
            }
            i += 1;
        }
        if depth != 0 {
            return None;
        }
    }
    let dest = dest_start..i;
    let spaced = skip_spaces(b, i);
    if spaced > i
        && let Some(&open) = b.get(spaced)
        && matches!(open, b'"' | b'\'' | b'(')
    {
        let close = if open == b'(' { b')' } else { open };
        let mut j = spaced + 1;
        loop {
            match *b.get(j)? {
                b'\\' => j += 2,
                c if c == close => break,
                b'(' if open == b'(' => return None,
                _ => j += 1,
            }
        }
        i = j + 1;
    } else {
        i = spaced;
    }
    let i = skip_spaces(b, i);
    (b.get(i) == Some(&b')')).then_some(InlineLink { end: i + 1, dest })
}

/// The text with each backslash escape replaced by its char.
pub(super) fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(&next) = chars.peek()
            && next.is_ascii_punctuation()
        {
            out.push(next);
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}
