//! `compare_paths` gives the order of `Path::cmp` for note paths.

use std::path::Path;

use holm_core::model::compare_paths;
use proptest::prelude::*;

/// A relative path with no `.` or `..` part, no `//` and no `/` at the end.
fn note_path() -> impl Strategy<Value = String> {
    let part =
        "[ !#+,.a-bA-Bé-]{1,4}".prop_filter("not . or ..", |part| part != "." && part != "..");
    prop::collection::vec(part, 1..4).prop_map(|parts| parts.join("/"))
}

proptest! {
    #[test]
    fn same_order_as_path_cmp(a in note_path(), b in note_path()) {
        let (a, b) = (Path::new(&a), Path::new(&b));
        prop_assert_eq!(compare_paths(a, b), a.cmp(b));
    }

    #[test]
    fn same_order_with_a_shared_start(start in note_path(), a in note_path(), b in note_path()) {
        let (a, b) = (format!("{start}{a}"), format!("{start}/{b}"));
        let (a, b) = (Path::new(&a), Path::new(&b));
        prop_assert_eq!(compare_paths(a, b), a.cmp(b));
    }
}
