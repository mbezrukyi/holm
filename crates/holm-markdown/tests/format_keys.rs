//! Tests of the format keys through `inline::restyle`.

use std::fmt::Write as _;
use std::ops::Range;

use holm_markdown::inline::{Format, Kind, Node, restyle};
use holm_markdown::{BlockKind, Body, Color, Style};
use proptest::prelude::*;

const FORMATS: [Format; 6] = [
    Format::Bold,
    Format::Italic,
    Format::Underline,
    Format::Strikethrough,
    Format::Highlight,
    Format::Code,
];

fn style_of(format: Format) -> Style {
    match format {
        Format::Bold => Style::Strong,
        Format::Italic => Style::Emphasis,
        Format::Underline => Style::Underline,
        Format::Strikethrough => Style::Strikethrough,
        Format::Highlight => Style::Highlight(None),
        Format::Code => Style::Code,
        Format::TextColor(_) | Format::HighlightColor(_) => {
            unreachable!("color keys set a color, they do not toggle")
        }
    }
}

/// One visible char: its source offset, the char, and its styles.
#[derive(Debug)]
struct Shown {
    at: usize,
    c: char,
    styles: Vec<Style>,
}

/// The chars that live preview shows away from the cursor, with their styles.
fn shown(text: &str) -> Vec<Shown> {
    let body = Body::parse(text);
    text.char_indices()
        .filter(|(at, _)| !body.hidden().iter().any(|r| r.contains(at)))
        .map(|(at, c)| Shown {
            at,
            c,
            styles: body
                .spans()
                .iter()
                .filter(|s| s.range.contains(&at))
                .map(|s| s.style)
                .collect(),
        })
        .collect()
}

fn visible(text: &str) -> String {
    shown(text).iter().map(|s| s.c).collect()
}

/// The text after the key, or `None` when the key does nothing.
fn press(text: &str, selection: Range<usize>, format: Format) -> Option<(String, Range<usize>)> {
    let edit = restyle(text, &Body::parse(text), selection, format)?;
    let mut new = text.to_owned();
    new.replace_range(edit.range.clone(), &edit.text);
    assert!(new.is_char_boundary(edit.selection.start) && new.is_char_boundary(edit.selection.end));
    assert!(edit.selection.start <= edit.selection.end && edit.selection.end <= new.len());
    Some((new, edit.selection))
}

/// The selection of the first `part` in `text`.
fn find(text: &str, part: &str) -> Range<usize> {
    let start = text
        .find(part)
        .unwrap_or_else(|| panic!("no {part:?} in {text:?}"));
    start..start + part.len()
}

/// Presses the keys in order, each on its part of the text. It shows each step.
fn steps(text: &str, keys: &[(&str, Format)]) -> String {
    let mut out = format!("{text}\n");
    let mut text = text.to_owned();
    for (part, format) in keys {
        let selection = if part.is_empty() {
            0..0
        } else {
            find(&text, part)
        };
        match press(&text, selection, *format) {
            Some((new, selection)) => {
                let _ = writeln!(out, "{format:?} on {part:?}: {new}   [{}]", &new[selection]);
                text = new;
            }
            None => {
                let _ = writeln!(out, "{format:?} on {part:?}: no change");
            }
        }
    }
    out
}

#[test]
fn known_cases() {
    use Format::{Bold, Code, Highlight, Italic, Strikethrough, Underline};
    let cases = [
        // The five cases that broke the old keys.
        steps(
            "==Write the report==",
            &[("Writ", Highlight), ("Writ", Highlight)],
        ),
        steps("**==Writ==**==e the report==", &[("Writ", Bold)]),
        steps("==*Writ*==*e the report*", &[("Writ", Highlight)]),
        steps("*==Writ==e the report*", &[("Writ", Italic)]),
        steps("A **tip with strong** text", &[("A **t", Italic)]),
        // The cases of the research table.
        steps(
            "A tip with strong text",
            &[("tip with strong", Bold), ("A **t", Italic)],
        ),
        steps("x\"y\"z", &[("\"y\"", Bold)]),
        steps("a b c", &[(" b ", Italic)]),
        steps("a `b`c", &[("a `b`", Bold)]),
        steps("a.b", &[("a.", Italic)]),
        steps("Write", &[("t", Bold)]),
        steps("ab", &[("a", Italic), ("b", Bold)]),
        // More shapes.
        steps("a **b** c", &[("a **b** c", Bold), ("a b c", Bold)]),
        steps("a *b* _c_ d", &[("a *b* _c_ d", Italic)]),
        steps("a word", &[("word", Underline), ("word", Strikethrough)]),
        steps("x [link](a.md) y", &[("lin", Bold), ("link", Code)]),
        steps("a `b` c", &[("a `b` c", Code), ("a b c", Code)]),
        steps("a `x*y` c", &[("x*y", Code)]),
        steps("tick ` here", &[("tick ` here", Code)]),
        steps("**a b** c", &[("b** c", Code)]),
        steps("<u>a **b</u> c**", &[("a", Underline)]),
        steps("> a *b\n> c* d", &[("a *b\n> c* d", Underline)]),
        steps(
            "| a | b |\n|---|---|\n| c | d |",
            &[("a | b |\n|---|---|\n| c", Bold)],
        ),
        steps("- [ ] task\n- next", &[("task\n- next", Highlight)]),
    ];
    insta::assert_snapshot!(cases.join("\n"));
}

#[test]
fn keys_with_no_selection() {
    let cases: Vec<String> = [
        ("one two", 1, Format::Bold),
        ("one two", 3, Format::Italic),
        ("one **two**", 8, Format::Bold),
        ("one ", 4, Format::Highlight),
        ("a  b", 2, Format::Underline),
        ("- ", 2, Format::Bold),
        ("x `code` y", 4, Format::Code),
        ("x `code` y", 4, Format::Bold),
        ("```\nx y\n```", 5, Format::Bold),
        ("one two", 1, Format::TextColor(Some(RED))),
        ("one ", 4, Format::TextColor(Some(RED))),
        ("one ", 4, Format::HighlightColor(Some(BLUE))),
        ("one ", 4, Format::TextColor(None)),
        ("==one== two", 3, Format::HighlightColor(None)),
    ]
    .iter()
    .map(|(text, at, format)| match press(text, *at..*at, *format) {
        Some((new, selection)) => {
            format!("{text:?} {at} {format:?}: {new:?} at {}", selection.start)
        }
        None => format!("{text:?} {at} {format:?}: no change"),
    })
    .collect();
    insta::assert_snapshot!(cases.join("\n"));
}

const RED: Color = Color {
    r: 0xb5,
    g: 0x44,
    b: 0x2c,
};
const BLUE: Color = Color {
    r: 0x3a,
    g: 0x66,
    b: 0xa0,
};

#[test]
fn color_keys() {
    use Format::{HighlightColor, TextColor};
    let cases = [
        steps("a word here", &[("word", TextColor(Some(RED)))]),
        steps(
            "a word here",
            &[
                ("word here", TextColor(Some(RED))),
                ("here", TextColor(Some(BLUE))),
                (
                    "ord</span> <span style=\"color: #3a66a0\">he",
                    TextColor(None),
                ),
            ],
        ),
        steps(
            "a <span style=\"color: #b5442c\">word</span>",
            &[
                ("word", TextColor(Some(RED))),
                ("wo", TextColor(Some(BLUE))),
            ],
        ),
        steps("a **word** b", &[("a **word** b", TextColor(Some(RED)))]),
        steps(
            "a ==word== b",
            &[
                ("wor", HighlightColor(Some(BLUE))),
                ("a", HighlightColor(None)),
            ],
        ),
        steps(
            "<mark style=\"background: #3a66a0\">all</mark> of it",
            &[("all", HighlightColor(None))],
        ),
        steps("plain", &[("plain", TextColor(None))]),
        // Ctrl+H removes a colored highlight, and adds `==` only where no highlight is.
        steps(
            "a <mark style=\"background: #3a66a0\">word</mark> b",
            &[("word", Format::Highlight)],
        ),
        steps(
            "<mark style=\"background: #3a66a0\">a</mark> word",
            &[("a</mark> word", Format::Highlight)],
        ),
    ];
    insta::assert_snapshot!(cases.join("\n"));
}

/// Letters, spaces and punctuation inside and outside the markers of each style.
#[test]
fn every_neighbor_of_a_marker_writes_back() {
    let outside = ["", "a", " ", ".", "*", "_", "=", "~", "`", "<"];
    let inside = ["b", ".", "*", "_", "`", "<"];
    for format in FORMATS {
        for before in outside {
            for first in inside {
                for last in inside {
                    for after in outside {
                        let part = format!("{first}x{last}");
                        let text = format!("{before}{part}{after}");
                        let selection = before.len()..before.len() + part.len();
                        check_press(&text, selection, format);
                    }
                }
            }
        }
    }
}

/// Checks one key press. The visible text stays, and the selected text gets the wanted style.
/// The other text keeps its styles.
fn check_press(text: &str, selection: Range<usize>, format: Format) {
    let body = Body::parse(text);
    // With no selection a key takes the word at the cursor. `keys_with_no_selection` tests it.
    if selection.is_empty() || body.blocks().iter().any(|b| b.kind != BlockKind::Paragraph) {
        return;
    }
    let mut raw = Vec::new();
    for leaf in body.leaves() {
        atom_ranges(&body.inline(text, leaf), &mut raw);
    }
    // An entity or an escape is never cut either.
    for (at, _) in text.match_indices("&amp;") {
        let backslashes = text[..at].bytes().rev().take_while(|&b| b == b'\\').count();
        if backslashes % 2 == 0 {
            raw.push(at..at + 5);
        }
    }
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'\\' && bytes[i + 1].is_ascii_punctuation() {
            raw.push(i..i + 2);
            i += 2;
        } else {
            i += 1;
        }
    }
    let old = shown(text);
    let style = style_of(format);
    // Ctrl+H sees a colored highlight as a highlight too.
    let is_style = |s: &Style| match style {
        Style::Highlight(_) => matches!(s, Style::Highlight(_)),
        _ => *s == style,
    };
    // A selection end inside an atom moves out to hold all of it.
    let cut = |at: usize| {
        raw.iter()
            .find(|r| r.contains(&at) && r.start < selection.end && selection.start < r.end)
    };
    let selected = |at: usize| selection.contains(&at) || cut(at).is_some();
    // A key takes a link as one unit and leaves the styles inside its text.
    let touches_link = links(&body)
        .iter()
        .any(|r| r.start < selection.end && selection.start < r.end);
    let picked: Vec<&Shown> = old
        .iter()
        .filter(|s| selected(s.at) && !s.c.is_whitespace())
        .collect();
    if picked.is_empty() || touches_link {
        return;
    }
    let Some((new_text, _)) = press(text, selection.clone(), format) else {
        // A lone backtick can pair with a new marker, and no escape stops that. A new block can
        // start after a key on only marker chars and tags. Spaces at the line start can do it too.
        let letters = picked
            .iter()
            .any(|s| s.c.is_alphanumeric() && !raw.iter().any(|r| r.contains(&s.at)));
        assert!(
            text.contains('`') || !letters || text.starts_with(' '),
            "no change for {format:?} on {selection:?} of {text:?}"
        );
        return;
    };
    let new = shown(&new_text);
    assert_eq!(
        visible(&new_text),
        visible(text),
        "{format:?} on {selection:?}: {text:?} became {new_text:?}"
    );
    let add = picked.iter().any(|s| !s.styles.iter().any(is_style));
    let others = |styles: &[Style]| {
        let mut others: Vec<Style> = styles.iter().copied().filter(|s| !is_style(s)).collect();
        others.sort_by_key(|s| format!("{s:?}"));
        others.dedup();
        others
    };
    for (old, new) in old.iter().zip(&new) {
        if old.c.is_whitespace() {
            continue;
        }
        let why = format!(
            "{format:?} on {selection:?}: {text:?} became {new_text:?} at {}",
            old.at
        );
        if selected(old.at) {
            assert_eq!(new.styles.iter().any(is_style), add, "{why}");
            // Code drops the styles that cover only a part of it.
            if format != Format::Code {
                assert_eq!(others(&new.styles), others(&old.styles), "{why}");
            }
        } else if !old.styles.contains(&Style::Code)
            && !old.styles.contains(&Style::Link)
            && !raw.iter().any(|r| r.contains(&old.at))
        {
            // A key on a part of a code span, link, tag, entity or escape takes all of it.
            assert_eq!(others(&new.styles), others(&old.styles), "{why}");
            let kept = |styles: &[Style]| {
                let mut kept: Vec<Style> = styles.iter().copied().filter(is_style).collect();
                kept.dedup();
                kept
            };
            assert_eq!(kept(&new.styles), kept(&old.styles), "{why}");
        }
    }
}

const COLOR_KEYS: [Format; 6] = [
    Format::TextColor(Some(RED)),
    Format::TextColor(Some(BLUE)),
    Format::TextColor(None),
    Format::HighlightColor(Some(RED)),
    Format::HighlightColor(Some(BLUE)),
    Format::HighlightColor(None),
];

/// Checks one color key on plain words and styles away from links and code. The visible text
/// stays, the selected letters get only the new color, and the other styles stay.
fn check_color(text: &str, selection: Range<usize>, format: Format) {
    let body = Body::parse(text);
    if selection.is_empty() || body.blocks().iter().any(|b| b.kind != BlockKind::Paragraph) {
        return;
    }
    let mut raw = Vec::new();
    for leaf in body.leaves() {
        atom_ranges(&body.inline(text, leaf), &mut raw);
    }
    let touches_raw = raw
        .iter()
        .any(|r| r.start < selection.end && selection.start < r.end);
    if touches_raw || text.contains(['\\', '&', '`']) {
        return;
    }
    let Some((new_text, _)) = press(text, selection.clone(), format) else {
        return;
    };
    let why = format!("{format:?} on {selection:?}: {text:?} became {new_text:?}");
    assert_eq!(visible(&new_text), visible(text), "{why}");
    let (family, wanted): (fn(&Style) -> bool, Option<Style>) = match format {
        Format::TextColor(color) => (|s| matches!(s, Style::Color(_)), color.map(Style::Color)),
        Format::HighlightColor(color) => (
            |s| matches!(s, Style::Highlight(_)),
            color.map(|c| Style::Highlight(Some(c))),
        ),
        _ => unreachable!("only color keys"),
    };
    let others = |styles: &[Style]| {
        let mut others: Vec<Style> = styles.iter().copied().filter(|s| !family(s)).collect();
        others.sort_by_key(|s| format!("{s:?}"));
        others.dedup();
        others
    };
    for (old, new) in shown(text).iter().zip(&shown(&new_text)) {
        if old.c.is_whitespace() || !selection.contains(&old.at) {
            continue;
        }
        let mut colors: Vec<Style> = new.styles.iter().copied().filter(family).collect();
        colors.dedup();
        assert_eq!(
            colors,
            wanted.into_iter().collect::<Vec<_>>(),
            "{why} at {}",
            old.at
        );
        assert_eq!(
            others(&new.styles),
            others(&old.styles),
            "{why} at {}",
            old.at
        );
    }
}

/// The source of each link and image.
fn links(body: &Body) -> Vec<Range<usize>> {
    body.links().iter().map(|l| l.range.clone()).collect()
}

fn atom_ranges(nodes: &[Node], out: &mut Vec<Range<usize>>) {
    for node in nodes {
        if matches!(
            node.kind,
            Kind::Html | Kind::Link(_) | Kind::Image(_) | Kind::Code
        ) {
            out.push(node.source.clone());
        }
        atom_ranges(&node.children, out);
    }
}

fn piece() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "a",
        "b",
        "word",
        "é",
        " ",
        " ",
        ".",
        "\"",
        "*",
        "**",
        "_",
        "__",
        "~~",
        "~",
        "==",
        "`",
        "``",
        "<u>",
        "</u>",
        "<b>",
        "</b>",
        "<i>",
        "</i>",
        "<mark>",
        "</mark>",
        "<span style=\"color: #c4553a\">",
        "</span>",
        "[",
        "]",
        "](x.md)",
        "\\",
        "\\*",
        "&amp;",
        "<br>",
    ])
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    /// A random block, selection and key: the output keeps the visible text and has the wanted
    /// styles.
    #[test]
    fn a_key_keeps_the_text_and_sets_the_style(
        pieces in prop::collection::vec(piece(), 1..14),
        a in 0..200_usize,
        b in 0..200_usize,
        key in 0..FORMATS.len(),
    ) {
        let text = pieces.concat();
        let mut bounds: Vec<usize> = (0..=text.len()).filter(|&i| text.is_char_boundary(i)).collect();
        bounds.dedup();
        let start = bounds[a % bounds.len()].min(bounds[b % bounds.len()]);
        let end = bounds[a % bounds.len()].max(bounds[b % bounds.len()]);
        check_press(&text, start..end, FORMATS[key]);
    }

    /// A random block, selection and color key: the output keeps the visible text, and the
    /// selected text has only the new color.
    #[test]
    fn a_color_key_keeps_the_text_and_sets_the_color(
        pieces in prop::collection::vec(piece(), 1..14),
        a in 0..200_usize,
        b in 0..200_usize,
        key in 0..6_usize,
    ) {
        let text = pieces.concat();
        let mut bounds: Vec<usize> = (0..=text.len()).filter(|&i| text.is_char_boundary(i)).collect();
        bounds.dedup();
        let start = bounds[a % bounds.len()].min(bounds[b % bounds.len()]);
        let end = bounds[a % bounds.len()].max(bounds[b % bounds.len()]);
        check_color(&text, start..end, COLOR_KEYS[key]);
    }

    /// The same key twice on the new selection gives back text with the old styles.
    #[test]
    fn a_key_pressed_twice_gives_back_the_styles(
        words in prop::collection::vec("[a-z]{1,4}", 1..6),
        keys in prop::collection::vec(0..FORMATS.len() - 1, 1..5),
    ) {
        let text = words.join(" ");
        let mut now = text.clone();
        let mut selection = 0..text.len();
        for key in keys.iter().chain(&keys) {
            if let Some((new, sel)) = press(&now, selection.clone(), FORMATS[*key]) {
                now = new;
                selection = sel;
            }
        }
        prop_assert_eq!(visible(&now), text.clone());
        prop_assert!(Body::parse(&now).spans().is_empty(), "{:?} has styles", now);
    }
}
