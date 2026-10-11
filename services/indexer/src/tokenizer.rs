//! A minimal, deterministic tokenizer.
//!
//! This is deliberately simple and lives behind the reader so the fake and any test can
//! share it. The full analysis pipeline (stemming, stopword marking, CJK bigrams) lands
//! with the indexer in Step 4; query-time and index-time analysis must use the *same*
//! tokenizer, so this is the single place both call.

use std::collections::BTreeMap;

use lynx_common::text;

/// Tokenise `text`: NFKC-normalise, fold case, and split on non-alphanumeric characters.
#[must_use]
pub fn tokenize(text: &str) -> Vec<String> {
    let folded = text::fold_for_compare(text);
    let mut tokens = Vec::new();
    let mut current = String::new();
    for c in folded.chars() {
        if c.is_alphanumeric() {
            current.push(c);
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
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
}
