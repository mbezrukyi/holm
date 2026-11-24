use super::*;

fn tag(name: &str) -> TagName {
    TagName::new(name).unwrap()
}

#[test]
fn names_go_up_to_the_top() {
    let tag = tag("a/b/c");
    assert_eq!(tag.names().collect::<Vec<_>>(), ["a/b/c", "a/b", "a"]);
}
