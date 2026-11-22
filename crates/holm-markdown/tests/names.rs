use holm_markdown::normalize_tag;

#[test]
fn tag_names() {
    let cases = [
        ("work", Some("work")),
        ("two words", Some("two-words")),
        ("  Work  ", Some("Work")),
        ("work/holm", Some("work/holm")),
        ("/work/holm/", Some("work/holm")),
        ("a//b", Some("a/b")),
        ("a / b c", Some("a/b-c")),
        ("", None),
        (" / / ", None),
    ];
    for (input, want) in cases {
        assert_eq!(normalize_tag(input).as_deref(), want, "input {input:?}");
    }
}
