//! This module has no GPUI. The element maps each [`Token`] to a theme color.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;
use std::sync::LazyLock;

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

/// The load takes tens of milliseconds, so it happens once.
static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);

/// The cache drops all entries when it grows past this count.
const CACHE_LIMIT: usize = 256;

/// Each kind has one color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Token {
    Comment,
    String,
    Number,
    Constant,
    Keyword,
    Function,
    Type,
    Tag,
    Attribute,
    Punctuation,
}

/// Byte ranges in the code of one block, sorted, with no overlaps.
pub(crate) type Tokens = Rc<[(Range<usize>, Token)]>;

struct Entry {
    language: String,
    code: String,
    tokens: Tokens,
}

/// The tokens of the code blocks seen lately.
#[derive(Default)]
pub(crate) struct Highlighter {
    cache: HashMap<u64, Vec<Entry>>,
    len: usize,
}

/// Loads the syntaxes. Call it on a background thread, so the first code block does not wait.
pub(crate) fn warm_up() {
    LazyLock::force(&SYNTAXES);
}

impl Highlighter {
    /// An unknown language has none.
    pub(crate) fn tokens(&mut self, language: &str, code: &str) -> Tokens {
        let mut hasher = DefaultHasher::new();
        (language, code).hash(&mut hasher);
        let key = hasher.finish();
        if let Some(entry) = self
            .cache
            .get(&key)
            .and_then(|entries| entries.iter().find(|e| e.language == language && e.code == code))
        {
            return entry.tokens.clone();
        }
        if self.len >= CACHE_LIMIT {
            self.cache.clear();
            self.len = 0;
        }
        let tokens: Tokens = find_syntax(language).map_or_else(|| Rc::from([]), |s| parse(s, code));
        self.cache.entry(key).or_default().push(Entry {
            language: language.to_owned(),
            code: code.to_owned(),
            tokens: tokens.clone(),
        });
        self.len += 1;
        tokens
    }
}

fn find_syntax(language: &str) -> Option<&'static SyntaxReference> {
    let lower = language.to_ascii_lowercase();
    let alias = match lower.as_str() {
        "" | "text" | "plain" => return None,
        "shell" | "sh" | "zsh" => "bash",
        "c++" => "cpp",
        "c#" | "csharp" => "cs",
        "js" => "javascript",
        "ts" => "typescript",
        "py" => "python",
        "rs" => "rust",
        "yml" => "yaml",
        "md" => "markdown",
        other => other,
    };
    [language, lower.as_str(), alias]
        .into_iter()
        .find_map(|token| SYNTAXES.find_syntax_by_token(token))
        .filter(|syntax| syntax.name != "Plain Text")
}

fn parse(syntax: &SyntaxReference, code: &str) -> Tokens {
    let mut parser = ParseState::new(syntax);
    let mut scopes = ScopeStack::new();
    let mut names = HashMap::<Scope, Option<Token>>::new();
    let mut tokens: Vec<(Range<usize>, Token)> = Vec::new();
    let mut push = |range: Range<usize>, scopes: &ScopeStack| {
        if range.is_empty() {
            return;
        }
        // The innermost scope with a kind wins.
        let token = scopes
            .scopes
            .iter()
            .rev()
            .find_map(|scope| *names.entry(*scope).or_insert_with(|| token_for(*scope)));
        let Some(token) = token else { return };
        match tokens.last_mut() {
            Some((last, kind)) if last.end == range.start && *kind == token => {
                last.end = range.end;
            }
            _ => tokens.push((range, token)),
        }
    };
    let mut offset = 0;
    for line in LinesWithEndings::from(code) {
        // A line that does not parse keeps its scopes. The next line goes on from them.
        if let Ok(operations) = parser.parse_line(line, &SYNTAXES) {
            let mut at = 0;
            for (index, operation) in operations {
                push(offset + at..offset + index, &scopes);
                let _ = scopes.apply(&operation);
                at = index;
            }
            push(offset + at..offset + line.len(), &scopes);
        }
        offset += line.len();
    }
    tokens.into()
}

/// `None` lets an outer scope decide.
fn token_for(scope: Scope) -> Option<Token> {
    let scope = scope.build_string();
    let is = |prefix: &str| {
        scope
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
    };
    let token = if is("punctuation.definition.string")
        || is("punctuation.definition.comment")
        || is("meta")
        || is("source")
        || is("text")
    {
        return None;
    } else if is("comment") {
        Token::Comment
    } else if is("string") {
        Token::String
    } else if is("constant.numeric") {
        Token::Number
    } else if is("constant") || is("support.constant") || is("variable.language") {
        Token::Constant
    } else if is("keyword.operator") || is("punctuation") {
        Token::Punctuation
    } else if is("keyword") || is("storage") {
        Token::Keyword
    } else if is("entity.name.function") || is("support.function") || is("variable.function") {
        Token::Function
    } else if is("entity.name.tag") {
        Token::Tag
    } else if is("entity.name") || is("support.type") || is("support.class") {
        Token::Type
    } else if is("entity.other.attribute-name") {
        Token::Attribute
    } else {
        return None;
    };
    Some(token)
}

#[cfg(test)]
mod tests {
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
}
