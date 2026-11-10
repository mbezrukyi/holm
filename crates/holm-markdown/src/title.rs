use std::ops::Range;

use pulldown_cmark::{Event, Parser};

/// Longest file stem in bytes. It leaves room for a clash number and `.md`
/// under the 255 byte name limit of Linux.
const MAX_STEM_BYTES: usize = 240;

/// Longest preview line in chars. A note list shows only the start of it.
const MAX_PREVIEW_CHARS: usize = 200;

/// The title of a note body: its first block, when that is a `# heading`.
pub(crate) fn title(body: &str) -> Option<String> {
    let (_, line) = title_line(body)?;
    // A `#` heading is always one line, so the line alone parses the same.
    let text = line_text(line);
    (!text.is_empty()).then_some(text)
}

/// Where the title text is in a note body, as a byte range. The title is a `# heading` that
/// is the first block. The range leaves out the `#` marks and the spaces around the text.
/// It can be empty, for a `#` line with no text.
///
/// ```
/// use holm_markdown::title_range;
///
/// let body = "\n# Plans ##\ntext\n";
/// assert_eq!(title_range(body).map(|range| &body[range]), Some("Plans"));
/// assert_eq!(title_range("## Plans\n"), None);
/// ```
pub fn title_range(body: &str) -> Option<Range<usize>> {
    let (start, line) = title_line(body)?;
    let after = line.trim_start_matches(' ').strip_prefix('#')?;
    let mut text = after.trim();
    // A closing run of `#` after a space is not part of the text.
    let open = text.trim_end_matches('#');
    if open.is_empty() || open.ends_with([' ', '\t']) {
        text = open.trim_end();
    }
    let from = start + line.len() - after.trim_start().len();
    Some(from..from + text.len())
}

/// The first line with text and its byte offset, when that line is a `# heading`.
/// The line has no line break.
fn title_line(body: &str) -> Option<(usize, &str)> {
    let mut start = 0;
    for line in body.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        if !text.trim().is_empty() {
            return is_h1(text).then_some((start, text));
        }
        start += line.len();
    }
    None
}

/// The first line of text after the title, with no Markdown syntax. It is empty when the
/// body has no such line.
pub(crate) fn preview(body: &str) -> String {
    let mut lines = body
        .lines()
        .filter(|line| !line.trim().is_empty())
        .peekable();
    if lines.peek().is_some_and(|line| is_h1(line)) {
        lines.next();
    }
    lines
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

fn is_h1(line: &str) -> bool {
    let rest = line.trim_start_matches(' ');
    if line.len() - rest.len() > 3 {
        return false;
    }
    rest.strip_prefix('#')
        .is_some_and(|after| after.is_empty() || after.starts_with([' ', '\t']))
}

/// The file name for a title, without `.md`.
///
/// It keeps the title as it is, but replaces `/` and a leading `.` with `-`.
/// It also cuts a very long title to 240 bytes.
///
/// ```
/// use holm_markdown::file_stem;
///
/// assert_eq!(file_stem("Plans 1/2"), "Plans 1-2");
/// assert_eq!(file_stem(".hidden"), "-hidden");
/// ```
pub fn file_stem(title: &str) -> String {
    let mut stem = title.replace('/', "-");
    if stem.starts_with('.') {
        stem.replace_range(..1, "-");
    }
    if stem.len() > MAX_STEM_BYTES {
        let mut end = MAX_STEM_BYTES;
        while !stem.is_char_boundary(end) {
            end -= 1;
        }
        stem.truncate(end);
    }
    stem
}
