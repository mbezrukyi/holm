use pulldown_cmark::{Event, Parser};

/// A note list shows only the start of the line.
const MAX_PREVIEW_CHARS: usize = 200;

/// The first line that has text, without its Markdown syntax.
pub(crate) fn preview(body: &str) -> String {
    body.lines()
        .filter(|line| !line.trim().is_empty())
        .map(line_text)
        .find(|text| !text.is_empty())
        .map(|text| cut(text, MAX_PREVIEW_CHARS))
        .unwrap_or_default()
}

/// Parsed alone, without its Markdown syntax.
fn line_text(line: &str) -> String {
    let mut text = String::new();
    for event in Parser::new(line) {
        if let Event::Text(t) | Event::Code(t) = event {
            text.push_str(&t);
        }
    }
    text.trim().to_owned()
}

fn cut(mut text: String, max: usize) -> String {
    if let Some((end, _)) = text.char_indices().nth(max) {
        text.truncate(end);
    }
    text
}
