//! A minimal, deterministic tokenizer.
//!
//! This is deliberately simple and lives behind the reader so the fake and any test can
//! share it. The full analysis pipeline (stemming, stopword marking, CJK bigrams) lands
//! with the indexer in Step 4; query-time and index-time analysis must use the *same*
//! tokenizer, so this is the single place both call.

use std::collections::BTreeMap;

use lynx_common::text;

/// A token and its byte span within the folded text.
///
/// The span is relative to the `NFKC`-folded form of the input, not the original
/// string; consumers that need original-text offsets treat it as best effort.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenSpan {
    /// The token text.
    pub text: String,
    /// Byte offset of the token's first character in the folded text.
    pub start: usize,
    /// Byte offset one past the token's last character in the folded text.
    pub end: usize,
}

/// Tokenise `text`, also reporting each token's byte span within the folded text.
#[must_use]
pub fn tokenize_with_spans(text: &str) -> Vec<TokenSpan> {
    let folded = text::fold_for_compare(text);
    let mut spans = Vec::new();
    let mut start: Option<usize> = None;
    let mut current = String::new();
    for (i, c) in folded.char_indices() {
        if c.is_alphanumeric() {
            if start.is_none() {
                start = Some(i);
            }
            current.push(c);
        } else if let Some(s) = start.take() {
            spans.push(TokenSpan {
                text: std::mem::take(&mut current),
                start: s,
                end: i,
            });
        }
    }
    if let Some(s) = start {
        spans.push(TokenSpan {
            text: current,
            start: s,
            end: folded.len(),
        });
    }
    spans
}

/// Tokenise `text`: NFKC-normalise, fold case, and split on non-alphanumeric characters.
#[must_use]
pub fn tokenize(text: &str) -> Vec<String> {
    tokenize_with_spans(text)
        .into_iter()
        .map(|t| t.text)
        .collect()
}

/// Term frequencies for a field.
#[must_use]
pub fn term_frequencies(text: &str) -> BTreeMap<String, u32> {
    let mut map = BTreeMap::new();
    for token in tokenize(text) {
        *map.entry(token).or_insert(0) += 1;
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_and_folds() {
        assert_eq!(tokenize("Hello, WORLD!"), vec!["hello", "world"]);
    }

    #[test]
    fn keeps_unicode_letters() {
        assert_eq!(tokenize("Café-Bar"), vec!["café", "bar"]);
    }

    #[test]
    fn frequencies_count_repeats() {
        let f = term_frequencies("the cat the hat");
        assert_eq!(f["the"], 2);
        assert_eq!(f["cat"], 1);
    }

    #[test]
    fn empty_is_empty() {
        assert!(tokenize("   ").is_empty());
    }

    #[test]
    fn spans_cover_the_folded_text() {
        let spans = tokenize_with_spans("Hello, WORLD!");
        assert_eq!(
            spans,
            vec![
                TokenSpan {
                    text: "hello".into(),
                    start: 0,
                    end: 5
                },
                TokenSpan {
                    text: "world".into(),
                    start: 7,
                    end: 12
                },
            ]
        );
    }
}
