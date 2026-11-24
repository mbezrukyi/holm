use super::*;

#[test]
fn rust_code_gets_tokens() {
    let mut highlighter = Highlighter::default();
    let code = "fn main() {\n    // hi\n    let s = \"x\";\n}\n";
    let tokens = highlighter.tokens("rust", code);
    let kind_of = |text: &str| {
        let start = code.find(text).unwrap_or(usize::MAX);
        tokens
            .iter()
            .find(|(range, _)| range.contains(&start))
            .map(|(_, token)| *token)
    };
    assert_eq!(kind_of("fn"), Some(Token::Keyword));
    assert_eq!(kind_of("// hi"), Some(Token::Comment));
    assert_eq!(kind_of("\"x\""), Some(Token::String));
    assert!(tokens.windows(2).all(|w| w[0].0.end <= w[1].0.start));
}

#[test]
fn an_unknown_language_has_no_tokens() {
    let mut highlighter = Highlighter::default();
    assert!(highlighter.tokens("no-such-language", "x = 1").is_empty());
    assert!(highlighter.tokens("", "x = 1").is_empty());
}

#[test]
fn the_same_code_comes_from_the_cache() {
    let mut highlighter = Highlighter::default();
    let first = highlighter.tokens("rust", "let x = 1;");
    let second = highlighter.tokens("rust", "let x = 1;");
    assert!(Rc::ptr_eq(&first, &second));
}
