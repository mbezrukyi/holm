use pulldown_cmark::{Event, Parser};

/// Longest preview line in chars. A note list shows only the start of it.
const MAX_PREVIEW_CHARS: usize = 200;

/// The first line of text in a note body, with no Markdown syntax. It is empty when the
/// body has no text.
pub(crate) fn preview(body: &str) -> String {
    body.lines()
        .filter(|line| !line.trim().is_empty())
        .map(line_text)
        .find(|text| !text.is_empty())
        .map(|text| cut(text, MAX_PREVIEW_CHARS))
        .unwrap_or_default()
}

/// The text of one line, parsed alone, with no Markdown syntax.
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
