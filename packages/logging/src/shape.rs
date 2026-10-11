//! Query shapes: the only supported description of a query for logs and metrics.
//!
//! A [`QueryShape`] is deliberately lossy. From a query string it keeps:
//!
//! - a coarse **length bucket** (0, 1–4, 5–8, 9–16, 17–32, 33+ characters),
//! - the **token count**,
//! - the set of **operator classes** present (phrase, site, filetype, …).
//!
//! It cannot be inverted to the query text. This is what makes it safe to emit at full
//! fidelity on every request.

use serde::{Deserialize, Serialize};

/// A coarse length bucket. `0` means empty; larger values are wider.
pub type LengthBucket = u8;

/// Bucket a string's length.
#[must_use]
pub fn length_bucket(query: &str) -> LengthBucket {
    let n = query.chars().count();
    match n {
        0 => 0,
        1..=4 => 1,
        5..=8 => 2,
        9..=16 => 3,
        17..=32 => 4,
        _ => 5,
    }
}

/// Operator classes present in a query. Boolean flags, no operands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)] // a flags set is the clearest representation here
pub struct OperatorClasses {
    /// A quoted phrase.
    pub phrase: bool,
    /// A `site:` restriction.
    pub site: bool,
    /// A `filetype:` restriction.
    pub filetype: bool,
    /// An `intitle:`/`inurl:` restriction.
    pub field: bool,
    /// An explicit `OR`.
    pub or: bool,
    /// A `-term` exclusion.
    pub exclude: bool,
    /// A `*` wildcard.
    pub wildcard: bool,
}

/// A lossy, non-invertible description of a query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryShape {
    /// Length bucket of the raw query.
    pub length: LengthBucket,
    /// Number of whitespace-separated tokens.
    pub tokens: usize,
    /// Operator classes present.
    pub operators: OperatorClasses,
}

/// Reduce a query to its [`QueryShape`].
#[must_use]
pub fn shape_of(query: &str) -> QueryShape {
    let tokens = query.split_whitespace();
    let mut count = 0usize;
    let mut operators = OperatorClasses::default();
    for token in tokens {
        count += 1;
        let lower = token.to_ascii_lowercase();
        if token.starts_with('"') || token.ends_with('"') {
            operators.phrase = true;
        }
        if lower.starts_with("site:") {
            operators.site = true;
        }
        if lower.starts_with("filetype:") || lower.starts_with("ext:") {
            operators.filetype = true;
        }
        if lower.starts_with("intitle:")
            || lower.starts_with("inurl:")
            || lower.starts_with("inbody:")
        {
            operators.field = true;
        }
        if token == "OR" || token == "or" || token == "|" {
            operators.or = true;
        }
        if token.starts_with('-') && token.len() > 1 {
            operators.exclude = true;
        }
        if token.contains('*') {
            operators.wildcard = true;
        }
    }
    QueryShape {
        length: length_bucket(query),
        tokens: count,
        operators,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_length() {
        assert_eq!(length_bucket(""), 0);
        assert_eq!(length_bucket("hi"), 1);
        assert_eq!(length_bucket("hello"), 2);
        assert_eq!(length_bucket("a".repeat(9).as_str()), 3);
        assert_eq!(length_bucket("a".repeat(20).as_str()), 4);
        assert_eq!(length_bucket("a".repeat(50).as_str()), 5);
    }

    #[test]
    fn detects_operators() {
        let s = shape_of(r#""exact phrase" site:example.com -spam filetype:pdf a* OR b"#);
        assert!(s.operators.phrase);
        assert!(s.operators.site);
        assert!(s.operators.filetype);
        assert!(s.operators.exclude);
        assert!(s.operators.wildcard);
        assert!(s.operators.or);
        assert_eq!(s.tokens, 8);
    }

    #[test]
    fn shape_does_not_contain_the_query() {
        let q = "my secret search terms";
        let s = shape_of(q);
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("secret"));
        assert_eq!(s.tokens, 4);
    }

    #[test]
    fn empty_query_shape() {
        let s = shape_of("");
        assert_eq!(s.length, 0);
        assert_eq!(s.tokens, 0);
        assert_eq!(s.operators, OperatorClasses::default());
    }
}
