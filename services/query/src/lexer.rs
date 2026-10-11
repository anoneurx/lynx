//! The lexer: splits a raw query into words and phrases.
//!
//! A single escape character, `\`, is resolved here, before any other interpretation.
//! The lexer also records whether a token contained an escaped colon or an escaped
//! leading sign, because that decides whether the parser may treat it as an operator or
//! a negation — `site\:example.com` is a literal token, not a `site:` filter.

/// A lexical token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// An unquoted word, escapes resolved.
    Word {
        /// The word, escapes resolved.
        text: String,
        /// Whether a `:` in this word was escaped; if so it is never an operator.
        escaped_colon: bool,
        /// Whether the first character was escaped; if so a leading `-`/`+` is literal.
        leading_sign_escaped: bool,
    },
    /// A quoted phrase, escapes resolved.
    Phrase(String),
    /// The disjunction token `OR` / `or` / `|`.
    Or,
}

/// Split `input` into tokens.
#[must_use]
pub fn tokenize(input: &str) -> Vec<Token> {
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < n {
        while i < n && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }

        if chars[i] == '"' {
            if let Some(close) = find_closing_quote(&chars, i) {
                let content: String = chars[i + 1..close].iter().collect();
                tokens.push(Token::Phrase(unescape(&content)));
                i = close + 1;
                continue;
            }
            // Unbalanced quote: fall through and treat `"` as a literal character.
        }

        let mut text = String::new();
        let mut escaped_colon = false;
        let mut leading_sign_escaped = false;
        let mut first = true;

        while i < n && !chars[i].is_whitespace() {
            if chars[i] == '\\' {
                if i + 1 < n {
                    let next = chars[i + 1];
                    if next == ':' {
                        escaped_colon = true;
                    }
                    if first {
                        leading_sign_escaped = true;
                    }
                    text.push(next);
                    i += 2;
                } else {
                    text.push('\\');
                    i += 1;
                }
            } else {
                text.push(chars[i]);
                i += 1;
            }
            first = false;
        }

        if matches!(text.as_str(), "OR" | "or" | "|") {
            tokens.push(Token::Or);
        } else {
            tokens.push(Token::Word {
                text,
                escaped_colon,
                leading_sign_escaped,
            });
        }
    }

    tokens
}

/// Find the index of the closing quote for a phrase starting at `open`, honouring `\`.
fn find_closing_quote(chars: &[char], open: usize) -> Option<usize> {
    let mut j = open + 1;
    let mut escaped = false;
    while j < chars.len() {
        let c = chars[j];
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            return Some(j);
        }
        j += 1;
    }
    None
}

/// Resolve `\x` escapes: `\"` is a quote, `\\` a backslash, `\c` a literal `c`.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            } else {
                out.push('\\');
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn word(text: &str) -> Token {
        Token::Word {
            text: text.to_string(),
            escaped_colon: false,
            leading_sign_escaped: false,
        }
    }

    #[test]
    fn splits_on_whitespace() {
        assert_eq!(tokenize("rust async"), vec![word("rust"), word("async")]);
    }

    #[test]
    fn extracts_phrase() {
        assert_eq!(
            tokenize(r#"foo "async runtime" bar"#),
            vec![
                word("foo"),
                Token::Phrase("async runtime".into()),
                word("bar")
            ]
        );
    }

    #[test]
    fn phrase_keeps_operators_literal() {
        assert_eq!(
            tokenize(r#""site:example.com""#),
            vec![Token::Phrase("site:example.com".into())]
        );
    }

    #[test]
    fn escaped_quote_inside_phrase() {
        assert_eq!(
            tokenize(r#""say \"hi\"""#),
            vec![Token::Phrase(r#"say "hi""#.into())]
        );
    }

    #[test]
    fn unbalanced_quote_is_literal() {
        assert_eq!(tokenize(r#"foo"bar"#), vec![word(r#"foo"bar"#)]);
    }

    #[test]
    fn escaped_colon_is_marked() {
        assert_eq!(
            tokenize(r"site\:example"),
            vec![Token::Word {
                text: "site:example".into(),
                escaped_colon: true,
                leading_sign_escaped: false,
            }]
        );
    }

    #[test]
    fn escaped_leading_hyphen_is_marked() {
        assert_eq!(
            tokenize(r"\-foo"),
            vec![Token::Word {
                text: "-foo".into(),
                escaped_colon: false,
                leading_sign_escaped: true,
            }]
        );
    }

    #[test]
    fn or_variants() {
        assert_eq!(
            tokenize("a OR b or c | d"),
            vec![
                word("a"),
                Token::Or,
                word("b"),
                Token::Or,
                word("c"),
                Token::Or,
                word("d"),
            ]
        );
    }

    #[test]
    fn ordinary_is_not_or() {
        assert_eq!(tokenize("ORDINARY"), vec![word("ORDINARY")]);
    }

    #[test]
    fn escaping_resolves_to_plain_text() {
        assert_eq!(tokenize(r"a\\b"), vec![word(r"a\b")]);
    }
}
