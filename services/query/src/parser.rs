//! The parser: tokens in, [`QueryPlan`] out.
//!
//! Precedence (from `docs/search/query-parsing.md`):
//!
//! ```text
//! 1  operators        parsed anywhere, on token boundaries
//! 2  phrases          "…" — content inside is never an operator
//! 3  OR               splits the surrounding term group into disjuncts
//! 4  negation         -term and NOT term, collected into must_not
//! 5  default          all remaining terms are must
//! ```
//!
//! The parser never errors on user input. Anything it cannot interpret is kept as a
//! literal term; anything it truncates produces a notice.

use time::{Date, Month};

use crate::ast::{Filters, FreshnessWindow, Operator, Phrase, QueryPlan, Term};
use crate::lexer::Token;

/// Parse tokens into a plan. `raw_len` is the byte length of the original query.
#[must_use]
#[allow(clippy::too_many_lines)] // the precedence rules read top-to-bottom as one pass
pub fn parse_tokens(tokens: &[Token], raw_len: u16) -> QueryPlan {
    let mut must: Vec<Term> = Vec::new();
    let mut literal_groups: Vec<Vec<Term>> = vec![Vec::new()];
    let mut must_not: Vec<Term> = Vec::new();
    let mut phrases: Vec<Phrase> = Vec::new();
    let mut filters = Filters::default();
    let mut operators: Vec<Operator> = Vec::new();
    let mut pending_not = false;
    let mut saw_or = false;

    for token in tokens {
        match token {
            Token::Or => {
                saw_or = true;
                push_operator(&mut operators, Operator::Or);
                if literal_groups.last().is_some_and(|g| !g.is_empty()) {
                    literal_groups.push(Vec::new());
                }
            }
            Token::Phrase(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    continue;
                }
                push_operator(&mut operators, Operator::Phrase);
                if std::mem::take(&mut pending_not) {
                    must_not.push(Term::Exact(trimmed.to_string()));
                } else {
                    phrases.push(Phrase {
                        text: trimmed.to_string(),
                        slop: 0,
                    });
                }
            }
            Token::Word {
                text,
                escaped_colon,
                leading_sign_escaped,
            } => {
                if text.eq_ignore_ascii_case("NOT") {
                    pending_not = true;
                    push_operator(&mut operators, Operator::Exclude);
                    continue;
                }

                let mut negated = std::mem::take(&mut pending_not);
                let mut body = text.as_str();
                let mut forced = false;

                if !leading_sign_escaped {
                    if let Some(rest) = body.strip_prefix('-') {
                        let is_number = rest.chars().next().is_some_and(|c| c.is_ascii_digit());
                        if !rest.is_empty() && !is_number {
                            negated = true;
                            body = rest;
                        }
                    } else if let Some(rest) = body.strip_prefix('+') {
                        if !rest.is_empty() {
                            forced = true;
                            body = rest;
                        }
                    }
                }

                if !escaped_colon {
                    if let Some((name, value)) = split_operator(body) {
                        if let Some(op) = apply_operator(name, value, &mut filters) {
                            push_operator(&mut operators, op);
                            continue;
                        }
                        // Unknown or invalid operator: keep the whole original token
                        // as a literal term, not the sign-stripped body.
                        add_literal(
                            &mut must,
                            &mut literal_groups,
                            &mut must_not,
                            negated,
                            forced,
                            text.clone(),
                        );
                        if negated {
                            push_operator(&mut operators, Operator::Exclude);
                        } else if forced {
                            push_operator(&mut operators, Operator::Force);
                        }
                        continue;
                    }
                }

                add_literal(
                    &mut must,
                    &mut literal_groups,
                    &mut must_not,
                    negated,
                    forced,
                    body.to_string(),
                );
                if negated {
                    push_operator(&mut operators, Operator::Exclude);
                } else if forced {
                    push_operator(&mut operators, Operator::Force);
                }
            }
        }
    }

    let non_empty: Vec<Vec<Term>> = literal_groups
        .into_iter()
        .filter(|g| !g.is_empty())
        .collect();
    let disjunction = saw_or;
    let mut should = Vec::new();
    let literal_flat: Vec<Term> = non_empty.into_iter().flatten().collect();
    if disjunction {
        should = literal_flat;
    } else {
        must.extend(literal_flat);
    }

    QueryPlan {
        raw_len,
        must,
        should,
        must_not,
        phrases,
        disjunction,
        filters,
        operators_used: operators,
        truncated: false,
        notices: Vec::new(),
    }
}

fn add_literal(
    must: &mut Vec<Term>,
    groups: &mut [Vec<Term>],
    must_not: &mut Vec<Term>,
    negated: bool,
    forced: bool,
    text: String,
) {
    if text.is_empty() {
        return;
    }
    if negated {
        must_not.push(Term::Exact(text));
    } else if forced {
        // A forced term is always required, regardless of any OR.
        must.push(Term::Exact(text));
    } else {
        // Plain terms live in the current group; the group is promoted to `must` or
        // `should` when the plan is assembled. `groups` always has at least one entry.
        if let Some(group) = groups.last_mut() {
            group.push(Term::Exact(text));
        }
    }
}

/// Push an operator once, preserving first-appearance order.
fn push_operator(operators: &mut Vec<Operator>, op: Operator) {
    if !operators.contains(&op) {
        operators.push(op);
    }
}

/// Split a `name:value` operator on the first colon.
fn split_operator(s: &str) -> Option<(&str, &str)> {
    let (name, value) = s.split_once(':')?;
    if name.is_empty() {
        None
    } else {
        Some((name, value))
    }
}

/// Apply a recognised operator. Returns the operator on success; `None` when the name is
/// unknown or the value is invalid, in which case the caller keeps the token literal.
fn apply_operator(name: &str, value: &str, filters: &mut Filters) -> Option<Operator> {
    match name.to_ascii_lowercase().as_str() {
        "site" => {
            let domain = value.to_ascii_lowercase();
            if valid_domain(&domain) {
                filters.site = Some(domain.trim_start_matches('.').to_string());
                Some(Operator::Site)
            } else {
                None
            }
        }
        "inurl" => {
            if value.is_empty() {
                None
            } else {
                filters.inurl = Some(value.to_string());
                Some(Operator::Inurl)
            }
        }
        "intitle" => {
            if value.is_empty() {
                None
            } else {
                filters.intitle.push(value.to_string());
                Some(Operator::Intitle)
            }
        }
        "ext" => {
            let ext = value.trim_start_matches('.').to_ascii_lowercase();
            if ext.is_empty() {
                None
            } else {
                filters.ext.push(ext);
                Some(Operator::Ext)
            }
        }
        "filetype" => {
            let ft = value.to_ascii_lowercase();
            if ft.is_empty() {
                None
            } else {
                filters.filetype.push(ft);
                Some(Operator::Filetype)
            }
        }
        "lang" => {
            if valid_language(value) {
                filters.lang = Some(value.to_string());
                Some(Operator::Lang)
            } else {
                None
            }
        }
        "before" => {
            let date = parse_date(value)?;
            filters.before = Some(date);
            Some(Operator::Before)
        }
        "after" => {
            let date = parse_date(value)?;
            filters.after = Some(date);
            Some(Operator::After)
        }
        "freshness" => {
            let window = FreshnessWindow::parse(value)?;
            filters.freshness = Some(window);
            Some(Operator::Freshness)
        }
        _ => None,
    }
}

/// A permissive host/domain check: enough to reject obvious typos and URL fragments.
fn valid_domain(s: &str) -> bool {
    let s = s.trim_start_matches('.');
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
        && s.chars().any(|c| c.is_ascii_alphanumeric())
        && !s.contains("..")
}

/// A language tag like `de`, `en-US`, `zh-Hans`.
fn valid_language(s: &str) -> bool {
    let mut parts = s.split('-');
    let primary = parts.next().unwrap_or("");
    if !(2..=3).contains(&primary.len()) || !primary.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    parts.all(|p| (2..=8).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// Parse `YYYY`, `YYYY-MM`, or `YYYY-MM-DD` into the first day of that period.
fn parse_date(s: &str) -> Option<Date> {
    let digits = |p: &str, n: usize| -> Option<i32> {
        if p.len() == n && p.chars().all(|c| c.is_ascii_digit()) {
            p.parse().ok()
        } else {
            None
        }
    };
    let parts: Vec<&str> = s.split('-').collect();
    match parts.as_slice() {
        [year] => {
            let y = digits(year, 4)?;
            Date::from_calendar_date(y, Month::January, 1).ok()
        }
        [year, month] => {
            let y = digits(year, 4)?;
            let m = digits(month, 2)?;
            let month = Month::try_from(u8::try_from(m).ok()?).ok()?;
            Date::from_calendar_date(y, month, 1).ok()
        }
        [year, month, day] => {
            let y = digits(year, 4)?;
            let m = digits(month, 2)?;
            let d = digits(day, 2)?;
            let month = Month::try_from(u8::try_from(m).ok()?).ok()?;
            Date::from_calendar_date(y, month, u8::try_from(d).ok()?).ok()
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;
    use pretty_assertions::assert_eq;
    use time::macros::date;

    fn plan(q: &str) -> QueryPlan {
        parse_tokens(&tokenize(q), u16::try_from(q.len()).unwrap_or(u16::MAX))
    }

    fn exact(s: &str) -> Term {
        Term::Exact(s.to_string())
    }

    #[test]
    fn all_terms_are_must_by_default() {
        let p = plan("rust async runtime");
        assert_eq!(
            p.must,
            vec![exact("rust"), exact("async"), exact("runtime")]
        );
        assert!(!p.disjunction);
        assert!(p.should.is_empty());
    }

    #[test]
    fn or_makes_terms_should() {
        let p = plan("rust OR zig");
        assert!(p.disjunction);
        assert_eq!(p.should, vec![exact("rust"), exact("zig")]);
        assert!(p.must.is_empty());
        assert!(p.operators_used.contains(&Operator::Or));
    }

    #[test]
    fn phrase_is_collected() {
        let p = plan(r#"foo "async runtime""#);
        assert_eq!(p.must, vec![exact("foo")]);
        assert_eq!(
            p.phrases,
            vec![Phrase {
                text: "async runtime".into(),
                slop: 0
            }]
        );
    }

    #[test]
    fn negation_is_a_negative_term() {
        let p = plan("rust -golang");
        assert_eq!(p.must, vec![exact("rust")]);
        assert_eq!(p.must_not, vec![exact("golang")]);
        assert!(p.operators_used.contains(&Operator::Exclude));
    }

    #[test]
    fn not_keyword_negates() {
        let p = plan("rust NOT golang");
        assert_eq!(p.must_not, vec![exact("golang")]);
    }

    #[test]
    fn force_include() {
        let p = plan("+the rust");
        assert_eq!(p.must, vec![exact("the"), exact("rust")]);
        assert!(p.operators_used.contains(&Operator::Force));
    }

    #[test]
    fn site_filter() {
        let p = plan("rust site:docs.rs");
        assert_eq!(p.filters.site.as_deref(), Some("docs.rs"));
        assert_eq!(p.must, vec![exact("rust")]);
    }

    #[test]
    fn invalid_domain_is_literal() {
        let p = plan("site:bad//domain");
        assert!(p.filters.site.is_none());
        assert_eq!(p.must, vec![exact("site:bad//domain")]);
    }

    #[test]
    fn empty_operator_value_is_literal() {
        let p = plan("site:");
        assert!(p.filters.site.is_none());
        assert_eq!(p.must, vec![exact("site:")]);
    }

    #[test]
    fn repeated_operator_last_wins() {
        let p = plan("site:a.com site:b.com");
        assert_eq!(p.filters.site.as_deref(), Some("b.com"));
    }

    #[test]
    fn date_filters() {
        let p = plan("rust after:2025-01 before:2026-03-15");
        assert_eq!(p.filters.after, Some(date!(2025 - 01 - 01)));
        assert_eq!(p.filters.before, Some(date!(2026 - 03 - 15)));
    }

    #[test]
    fn year_only_date() {
        let p = plan("after:2024");
        assert_eq!(p.filters.after, Some(date!(2024 - 01 - 01)));
    }

    #[test]
    fn freshness_window() {
        let p = plan("news freshness:week");
        assert_eq!(p.filters.freshness, Some(FreshnessWindow::Week));
    }

    #[test]
    fn ext_strips_dot() {
        let p = plan("report ext:.pdf");
        assert_eq!(p.filters.ext, vec!["pdf".to_string()]);
    }

    #[test]
    fn lang_filter_validates() {
        assert_eq!(plan("café lang:de").filters.lang.as_deref(), Some("de"));
        assert!(plan("x lang:!!").filters.lang.is_none());
    }

    #[test]
    fn leading_hyphen_on_number_is_literal() {
        let p = plan("-5");
        assert_eq!(p.must, vec![exact("-5")]);
        assert!(p.must_not.is_empty());
    }

    #[test]
    fn trailing_hyphen_is_literal() {
        let p = plan("foo-");
        assert_eq!(p.must, vec![exact("foo-")]);
    }

    #[test]
    fn escaped_colon_prevents_operator() {
        let p = plan(r"site\:example");
        assert!(p.filters.site.is_none());
        assert_eq!(p.must, vec![exact("site:example")]);
    }

    #[test]
    fn operators_are_deduplicated_in_order() {
        let p = plan("site:a.com after:2020 ext:pdf site:b.com");
        assert_eq!(
            p.operators_used,
            vec![Operator::Site, Operator::After, Operator::Ext]
        );
    }

    #[test]
    fn empty_query_is_empty_plan() {
        let p = plan("   ");
        assert!(p.is_empty());
        assert_eq!(p.operators_used, Vec::new());
    }
}
