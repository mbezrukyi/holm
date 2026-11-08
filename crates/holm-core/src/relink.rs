//! File link paths: where they point, and how to write them after a note moves.

use std::path::{Component, Path, PathBuf};

use holm_markdown::{Body, Note as Text, Target};

/// The path of a file that a note in `folder` links as `file`.
///
/// It is relative to the notes folder `root` when the file is inside it, else absolute.
pub(crate) fn resolve(root: &Path, folder: &Path, file: &str) -> PathBuf {
    let full = clean(&root.join(folder).join(file));
    match full.strip_prefix(root) {
        Ok(rel) => rel.to_owned(),
        Err(_) => full,
    }
}

/// A folder move: its old path and its new path, relative to the notes folder.
#[derive(Clone, Copy)]
pub(crate) struct Moved<'a> {
    pub(crate) from: &'a Path,
    pub(crate) to: &'a Path,
}

/// The note text with its relative file links fixed, after the note moved
/// from the folder `from` to the folder `to`.
///
/// With `moved`, the files inside that folder moved too.
/// A link that still points to the same file stays as it is.
/// It returns `None` when no link changes.
pub(crate) fn relink(
    text: &str,
    root: &Path,
    from: &Path,
    to: &Path,
    moved: Option<Moved<'_>>,
) -> Option<String> {
    let mut note = Text::parse(text);
    let body = note.body();
    let mut edits = Vec::new();
    for link in Body::parse(body).links() {
        let (Target::File(file), Some(range)) = (&link.target, &link.target_range) else {
            continue;
        };
        if Path::new(file).is_absolute() {
            continue;
        }
        let mut target = clean(&root.join(from).join(file));
        if let Some(moved) = moved
            && let Ok(rest) = target.strip_prefix(root.join(moved.from))
        {
            target = root.join(moved.to).join(rest);
        }
        if clean(&root.join(to).join(file)) == target {
            continue;
        }
        let path = relative(&root.join(to), &target);
        let Some(path) = path.to_str() else {
            tracing::warn!(path = %path.display(), "kept a file link that is not UTF-8");
            continue;
        };
        edits.push((range.clone(), destination(path)));
    }
    if edits.is_empty() {
        return None;
    }
    // A link can hold an embedded file, so the list is not always in text order.
    edits.sort_by_key(|(range, _)| range.start);
    let mut out = String::with_capacity(body.len());
    let mut last = 0;
    for (range, dest) in edits {
        out.push_str(&body[last..range.start]);
        out.push_str(&dest);
        last = range.end;
    }
    out.push_str(&body[last..]);
    note.set_body(&out);
    Some(note.to_string())
}

/// The path with no `.` and `..` parts. It does not look at the disk.
fn clean(path: &Path) -> PathBuf {
    let mut clean = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                clean.pop();
            }
            Component::CurDir => {}
            part => clean.push(part),
        }
    }
    clean
}

/// The path from the folder `base` to `target`. Both are absolute and clean.
fn relative(base: &Path, target: &Path) -> PathBuf {
    let base: Vec<_> = base.components().collect();
    let target: Vec<_> = target.components().collect();
    let common = base.iter().zip(&target).take_while(|(a, b)| a == b).count();
    let mut out = PathBuf::new();
    for _ in common..base.len() {
        out.push("..");
    }
    out.extend(&target[common..]);
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// A link destination for `path`. A path with spaces or brackets goes in `< >`.
fn destination(path: &str) -> String {
    let angle = path.contains(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '<' | '>'));
    let mut out = String::with_capacity(path.len() + 2);
    if angle {
        out.push('<');
    }
    for c in path.chars() {
        if c == '\\' || (angle && matches!(c, '<' | '>')) {
            out.push('\\');
        }
        out.push(c);
    }
    if angle {
        out.push('>');
    }
    out
}
