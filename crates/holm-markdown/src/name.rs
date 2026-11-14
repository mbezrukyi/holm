/// Longest file stem in bytes. It leaves room for the hash and the extension of an
/// attachment under the 255 byte name limit of Linux.
const MAX_STEM_BYTES: usize = 240;

/// A file stem from any text, like the name of a pasted file.
///
/// It keeps the text as it is, but replaces `/` and a leading `.` with `-`.
/// It also cuts a very long text to 240 bytes.
///
/// ```
/// use holm_markdown::file_stem;
///
/// assert_eq!(file_stem("Plans 1/2"), "Plans 1-2");
/// assert_eq!(file_stem(".hidden"), "-hidden");
/// ```
pub fn file_stem(text: &str) -> String {
    let mut stem = text.replace('/', "-");
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
