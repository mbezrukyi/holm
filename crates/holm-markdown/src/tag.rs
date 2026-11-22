/// `None` when nothing is left of what the user typed.
///
/// - A space becomes `-`.
/// - `/` makes a child tag, like `work/holm`.
/// - Spaces around a `/`, a `/` at the start or the end, and empty parts like `a//b` go away.
///
/// Tags match exactly, also in capital letters, so this keeps the case.
///
/// ```
/// use holm_markdown::normalize_tag;
///
/// assert_eq!(normalize_tag(" my work / holm/ "), Some("my-work/holm".to_owned()));
/// assert_eq!(normalize_tag("//"), None);
/// ```
pub fn normalize_tag(name: &str) -> Option<String> {
    let parts: Vec<String> = name
        .split('/')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| part.replace(char::is_whitespace, "-"))
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Normalizes each tag. Drops empty and repeated tags, and keeps the order.
pub(crate) fn clean_tags<I, S>(tags: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out: Vec<String> = Vec::new();
    for tag in tags {
        if let Some(tag) = normalize_tag(tag.as_ref())
            && !out.contains(&tag)
        {
            out.push(tag);
        }
    }
    out
}
