//! The small part of YAML that Holm's own fields need.

/// The value at the start of `s`, and the text after it.
///
/// In a flow list, `,` and `]` end a plain scalar.
pub(crate) fn read_scalar(s: &str, in_flow: bool) -> (String, &str) {
    let s = s.trim_start_matches([' ', '\t']);
    if let Some(rest) = s.strip_prefix('"') {
        read_double_quoted(rest)
    } else if let Some(rest) = s.strip_prefix('\'') {
        read_single_quoted(rest)
    } else {
        read_plain(s, in_flow)
    }
}

fn read_double_quoted(s: &str) -> (String, &str) {
    let mut out = String::new();
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return (out, &s[i + 1..]),
            '\\' => match chars.next() {
                Some((_, 'n')) => out.push('\n'),
                Some((_, 't')) => out.push('\t'),
                Some((_, c)) => out.push(c),
                None => break,
            },
            c => out.push(c),
        }
    }
    (out, "")
}

fn read_single_quoted(s: &str) -> (String, &str) {
    let mut out = String::new();
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c != '\'' {
            out.push(c);
        } else if chars.next_if(|&(_, c)| c == '\'').is_some() {
            out.push('\'');
        } else {
            return (out, &s[i + 1..]);
        }
    }
    (out, "")
}

fn read_plain(s: &str, in_flow: bool) -> (String, &str) {
    let mut end = s.len();
    // A `#` starts a comment only after a space, or at the start.
    let mut after_space = true;
    for (i, c) in s.char_indices() {
        if (c == '#' && after_space) || (in_flow && (c == ',' || c == ']')) {
            end = i;
            break;
        }
        after_space = c.is_whitespace();
    }
    (s[..end].trim().to_owned(), &s[end..])
}

/// `s` starts after the `[`.
pub(crate) fn read_flow_list(s: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut rest = s;
    loop {
        let r = rest.trim_start();
        if r.is_empty() || r.starts_with(']') {
            return items;
        }
        if let Some(r) = r.strip_prefix(',') {
            rest = r;
            continue;
        }
        let (value, after) = read_scalar(r, true);
        if after.len() == r.len() {
            // Nothing was read, for example at a `#`. Skip one char.
            let skip = r.chars().next().map_or(0, char::len_utf8);
            rest = &r[skip..];
            continue;
        }
        items.push(value);
        rest = after;
    }
}

/// Writes `s` as a scalar that YAML reads back as the same string.
pub(crate) fn write_scalar(s: &str) -> String {
    if is_plain_safe(s) {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn is_plain_safe(s: &str) -> bool {
    let Some(first) = s.chars().next() else {
        return false;
    };
    let reserved = matches!(
        s.to_ascii_lowercase().as_str(),
        "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "y" | "n"
    );
    (first.is_alphanumeric() || first == '_')
        && s.chars().all(|c| c.is_alphanumeric() || "-_/.".contains(c))
        && !reserved
        && s.parse::<f64>().is_err()
}
