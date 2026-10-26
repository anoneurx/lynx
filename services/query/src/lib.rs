//! `lynx-query` — the query processor.
//!
//! Pipeline: **lex → parse → validate → normalise → compile**. The raw query string is
//! used only long enough to produce a [`QueryPlan`]; after that there is nothing left to
//! log. See `docs/search/query-parsing.md`.
//!
//! ```no_run
//! use lynx_query::parse;
//! let plan = parse(r#"rust site:docs.rs -github "async runtime""#);
//! assert_eq!(plan.filters.site.as_deref(), Some("docs.rs"));
//! ```

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

pub mod ast;
pub mod lexer;
pub mod parser;

pub use ast::{Filters, FreshnessWindow, Operator, Phrase, QueryPlan, Term};

use lynx_common::text;

/// The service name, used in logs and metrics.
pub const SERVICE_NAME: &str = "query";

/// Parsing limits, derived from `config/*.toml`.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Maximum query length in bytes. Longer queries are cut at a term boundary.
    pub max_bytes: usize,
    /// Maximum number of positive terms. Extra terms are dropped.
    pub max_terms: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 512,
            max_terms: 64,
        }
    }
}

/// A compiled query, ready for the index. Terms are normalised and lowercased; filters
/// are carried through on the plan.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledQuery {
    /// The full plan, for explain and filter application.
    pub plan: QueryPlan,
    /// Normalised `must` term texts.
    pub must_terms: Vec<String>,
    /// Normalised `should` term texts.
    pub should_terms: Vec<String>,
    /// Normalised negative term texts.
    pub must_not_terms: Vec<String>,
    /// Normalised phrase texts.
    pub phrases: Vec<String>,
}

impl CompiledQuery {
    /// Whether the query yields no positive clause.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.must_terms.is_empty()
            && self.should_terms.is_empty()
            && self.phrases.is_empty()
            && !self.plan.filters.any()
    }
}

/// Parse with the default limits.
#[must_use]
pub fn parse(input: &str) -> QueryPlan {
    parse_with_limits(input, &Limits::default())
}

/// Parse, validate against `limits`, then normalise. Never errors.
#[must_use]
pub fn parse_with_limits(input: &str, limits: &Limits) -> QueryPlan {
    let mut notices: Vec<String> = Vec::new();
    let mut truncated = false;

    let raw_len = u16::try_from(input.len()).unwrap_or(u16::MAX);
    let slice = truncate_bytes(input, limits.max_bytes, &mut truncated, &mut notices);

    let tokens = lexer::tokenize(slice);
    let mut plan = parser::parse_tokens(&tokens, raw_len);

    truncate_terms(&mut plan, limits.max_terms, &mut truncated, &mut notices);
    normalise(&mut plan);

    plan.truncated = truncated;
    plan.notices = notices;
    plan
}

/// Parse and compile in one step.
#[must_use]
pub fn compile_query(input: &str) -> CompiledQuery {
    compile(parse(input))
}

/// Compile a plan into the shape the index consumes.
#[must_use]
pub fn compile(plan: QueryPlan) -> CompiledQuery {
    let must_terms = plan.must.iter().map(|t| t.text().to_string()).collect();
    let should_terms = plan.should.iter().map(|t| t.text().to_string()).collect();
    let must_not_terms = plan.must_not.iter().map(|t| t.text().to_string()).collect();
    let phrases = plan.phrases.iter().map(|p| p.text.clone()).collect();
    CompiledQuery {
        plan,
        must_terms,
        should_terms,
        must_not_terms,
        phrases,
    }
}

/// Cut a query to `max_bytes` at the last whitespace boundary, recording a notice.
fn truncate_bytes<'a>(
    input: &'a str,
    max_bytes: usize,
    truncated: &mut bool,
    notices: &mut Vec<String>,
) -> &'a str {
    if input.len() <= max_bytes {
        return input;
    }
    let mut cut = max_bytes;
    while cut > 0 && !input.is_char_boundary(cut) {
        cut -= 1;
    }
    let slice = &input[..cut];
    let end = slice.rfind(char::is_whitespace).unwrap_or(cut);
    *truncated = true;
    notices.push("Query was too long and was truncated at a term boundary.".to_string());
    &input[..end]
}

/// Cap the number of positive terms, dropping trailing ones, and record a notice.
fn truncate_terms(
    plan: &mut QueryPlan,
    max_terms: usize,
    truncated: &mut bool,
    notices: &mut Vec<String>,
) {
    if plan.term_count() <= max_terms {
        return;
    }
    let mut remaining = max_terms;
    cap(&mut plan.must, &mut remaining);
    cap(&mut plan.should, &mut remaining);
    cap(&mut plan.phrases, &mut remaining);
    *truncated = true;
    notices.push(format!(
        "Query had too many terms and was truncated to {max_terms}."
    ));
}

fn cap<T>(v: &mut Vec<T>, remaining: &mut usize) {
    if v.len() > *remaining {
        v.truncate(*remaining);
    }
    *remaining = remaining.saturating_sub(v.len());
}

/// Normalise every user-derived string: NFKC, then lowercase.
fn normalise(plan: &mut QueryPlan) {
    for term in plan
        .must
        .iter_mut()
        .chain(plan.should.iter_mut())
        .chain(plan.must_not.iter_mut())
    {
        normalise_term(term);
    }
    for phrase in &mut plan.phrases {
        phrase.text = fold(&phrase.text);
    }
    for t in plan
        .filters
        .intitle
        .iter_mut()
        .chain(plan.filters.inanchor.iter_mut())
    {
        *t = fold(t);
    }
    for e in &mut plan.filters.ext {
        *e = fold(e);
    }
    for f in &mut plan.filters.filetype {
        *f = fold(f);
    }
    if let Some(inurl) = &plan.filters.inurl {
        plan.filters.inurl = Some(text::nfkc(inurl)); // URL paths are case-sensitive
    }
}

fn normalise_term(term: &mut Term) {
    match term {
        Term::Exact(s) | Term::Prefix(s) | Term::Fuzzy(s) => *s = fold(s),
        Term::Expanded { expansion, .. } => *expansion = fold(expansion),
        Term::Corrected { corrected, .. } => *corrected = fold(corrected),
    }
}

fn fold(s: &str) -> String {
    text::fold_for_compare(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn service_name_is_stable() {
        assert_eq!(SERVICE_NAME, "query");
    }

    #[test]
    fn normalisation_folds_case_and_width() {
        let plan = parse("ＲＵＳＴ Café");
        assert_eq!(
            plan.must.iter().map(Term::text).collect::<Vec<_>>(),
            vec!["rust", "café"]
        );
    }

    #[test]
    fn truncates_long_query_at_term_boundary() {
        let limits = Limits {
            max_bytes: 8,
            max_terms: 64,
        };
        let plan = parse_with_limits("alpha beta gamma", &limits);
        assert!(plan.truncated);
        assert_eq!(plan.must, vec![Term::Exact("alpha".into())]);
        assert!(!plan.notices.is_empty());
    }

    #[test]
    fn caps_term_count() {
        let limits = Limits {
            max_bytes: 512,
            max_terms: 2,
        };
        let plan = parse_with_limits("one two three four", &limits);
        assert!(plan.truncated);
        assert_eq!(plan.term_count(), 2);
    }

    #[test]
    fn compile_exposes_normalised_terms() {
        let c = compile_query("Rust async");
        assert_eq!(c.must_terms, vec!["rust".to_string(), "async".to_string()]);
        assert!(!c.is_empty());
    }

    #[test]
    fn empty_query_compiles_empty() {
        assert!(compile_query("   ").is_empty());
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn parsing_never_panics(q in ".{0,400}") {
            let plan = parse(&q);
            prop_assert!(usize::from(plan.raw_len) <= q.len());
        }

        #[test]
        fn parsing_is_deterministic(q in ".{0,200}") {
            prop_assert_eq!(parse(&q), parse(&q));
        }

        #[test]
        fn terms_are_never_empty_strings(q in ".{0,200}") {
            let plan = parse(&q);
            for term in plan.must.iter().chain(&plan.should).chain(&plan.must_not) {
                prop_assert!(!term.text().is_empty());
            }
            for phrase in &plan.phrases {
                prop_assert!(!phrase.text.is_empty());
            }
        }

        #[test]
        fn quotes_are_balanced_without_panicking(q in "\"{0,50}") {
            let _ = parse(&q);
        }
    }
}
