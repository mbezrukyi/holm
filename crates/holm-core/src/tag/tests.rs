use super::*;

fn tag(name: &str) -> TagName {
    TagName::new(name).unwrap()
}

#[test]
fn new_takes_only_written_names() {
    assert_eq!(tag("work/holm").as_str(), "work/holm");
    for name in [
        "",
        "/",
        "work/",
        "/work",
        "work//holm",
        " work",
        "my work",
        "work/ holm",
    ] {
        assert_eq!(TagName::new(name), None, "{name:?}");
    }
}

#[test]
fn typed_cleans_the_text() {
    assert_eq!(TagName::typed(" my work / holm/ ").unwrap(), tag("my-work/holm"));
    assert!(matches!(TagName::typed(" / "), Err(Error::BadTag { tag }) if tag == " / "));
}

#[test]
fn parent_and_last_part() {
    assert_eq!(tag("work/holm/ui").parent(), Some(tag("work/holm")));
    assert_eq!(tag("work").parent(), None);
    assert_eq!(tag("work/holm").last_part(), "holm");
    assert_eq!(tag("work").last_part(), "work");
}

#[test]
fn a_tag_holds_itself_and_its_children() {
    let work = tag("work");
    assert!(work.holds(&work));
    assert!(work.holds(&tag("work/holm/ui")));
    assert!(!work.holds(&tag("workshop")));
    assert!(!work.holds(&tag("home/work")));
    assert!(!tag("work/holm").holds(&work));
    assert!(!tag("Work").holds(&work));
}

#[test]
fn moved_takes_the_children_along() {
    let (from, to) = (tag("work"), tag("life/home"));
    assert_eq!(tag("work").moved(&from, &to), Some(tag("life/home")));
    assert_eq!(tag("work/holm").moved(&from, &to), Some(tag("life/home/holm")));
    assert_eq!(tag("workshop").moved(&from, &to), None);
}

#[test]
fn names_go_up_to_the_top() {
    let tag = tag("a/b/c");
    assert_eq!(tag.names().collect::<Vec<_>>(), ["a/b/c", "a/b", "a"]);
}
