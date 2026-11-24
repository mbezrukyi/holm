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
            Part::Line(line) => Shape::Line(text[line.kept.bytes()].trim().to_owned()),
            Part::List(list) => Shape::List(
                list.items
                    .iter()
                    .zip(numbers(list))
                    .map(|(item, number)| {
                        let kept = match &item.kept {
                            Src::Old(range) => &text[range.bytes()],
                            Src::New(new) => new.as_str(),
                        };
                        (list.mark.text(number), kept.trim().to_owned(), shape(text, &item.parts))
                    })
                    .collect(),
            ),
        })
        .collect()
}

/// The tree of the lists that the parse of `text` has in `region`.
fn parsed(text: &str, region: &Range<SourceOffset>, quotes: usize) -> Result<Vec<Part>, String> {
    let body = Body::parse(text);
    let mut blocks = body.blocks();
    let overlaps = |b: &Block| {
        let end = if matches!(b.kind, BlockKind::List { .. }) {
            list_end(b)
        } else {
            b.range.end
        };
        b.range.start <= region.end && region.start <= end
    };
    for _ in 0..quotes {
        blocks = &blocks.iter().find(|b| overlaps(b)).ok_or("no quote")?.children;
    }
    let source = Source::lines(text, region.clone(), quotes);
    let blocks: Vec<&Block> = blocks.iter().filter(|b| overlaps(b)).collect();
    if let Some(block) = blocks.iter().find(|b| !matches!(b.kind, BlockKind::List { .. })) {
        return Err(format!("not a list: {:?}", block.kind));
    }
    source.tree(&blocks).ok_or_else(|| "no tree".to_owned())
}

/// Where parts stand in the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Level {
    Top,
    Nested,
}

/// Whether a list after the first starts with an empty item. Markdown cannot write it, since such an item cannot
/// start a list after a paragraph. The parser lets an empty bullet through one level down, but no deeper. The key
/// writes it anyway, and it becomes a list once the item has text.
fn has_empty_head(text: &str, parts: &[Part], level: Level) -> bool {
    parts.iter().enumerate().any(|(k, part)| {
        let Part::List(list) = part else {
            return false;
        };
        let head = &list.items[0];
        let empty = matches!(&head.kept, Src::Old(r) if text[r.bytes()].trim().is_empty())
            || matches!(&head.kept, Src::New(new) if new.trim().is_empty());
        let empty_head = empty && !(level == Level::Top && k == 0);
        empty_head
            || list
                .items
                .iter()
                .any(|item| has_empty_head(text, &item.parts, Level::Nested))
    })
}

/// The text of each line without its quote markers, indent and list marker.
fn words(text: &str, region: &Range<usize>) -> Vec<String> {
    text[region.clone()]
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

/// A text of list lines: level, marker, text, a second line (none, indented, lazy), a blank line after, and the
/// spaces before the marker.
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
    /// The parse after the edit gives the tree that the key planned, and the text of each line stays.
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
        let Some(plan) = plan(&text, &body, SourceOffset::range(selection), key) else {
            return Ok(());
        };
        let mut new = text.clone();
        new.replace_range(plan.edit.range.bytes(), &plan.edit.text);
        prop_assert!(new.is_char_boundary(plan.edit.selection.start.get()));
        prop_assert!(plan.edit.selection.end.get() <= new.len());
        let Some(planned) = plan.tree else {
            return Ok(());
        };
        if has_empty_head(&text, &planned.parts, Level::Top) {
            return Ok(());
        }
        let after = parsed(&new, &SourceOffset::range(planned.new_region.bytes()), planned.quotes);
        prop_assert!(after.is_ok(), "{:?} in {new:?} after {key:?} on {text:?}", after.err());
        let after = after.unwrap_or_default();
        prop_assert_eq!(
            shape(&text, &planned.parts),
            shape(&new, &after),
            "{:?} on {:?} gives {:?}", key, text, new
        );
        if key != Move::Next {
            prop_assert_eq!(words(&text, &planned.old_region.bytes()), words(&new, &planned.new_region.bytes()));
        }
    }
}
