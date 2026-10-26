//! `lynx-suggestions` — query completions.
//!
//! Completions are derived from index terms and document titles in the corpus. There
//! is no query log to mine, by design: the raw candidates come from
//! [`lynx_indexer::SearchIndexReader::suggest_candidates`], and this crate only ranks,
//! deduplicates, and truncates them.
//!
//! The score follows `docs/search/suggestions.md`:
//!
//! ```text
//! score = 0.40·corpus_frequency_prior
//!       + 0.25·title_match_bonus
//!       + 0.20·co_occurrence_prior        (0.0 in v0.1; related is page-level)
//!       + 0.15·length_prior
//! ```
//!
//! Every component is index-derived. The consequence is worth stating: LYNX's
//! autocomplete cannot know what people actually type, only what exists in the corpus.
//!
//! This crate never persists anything. It takes candidates and returns an ordering.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

use lynx_indexer::{SuggestionCandidate, SuggestionKind};

/// The service name, used in logs and metrics.
pub const SERVICE_NAME: &str = "suggestions";

/// Weight of the corpus-frequency prior in the completion score.
pub const CORPUS_FREQUENCY_WEIGHT: f32 = 0.40;
/// Weight of the title-match bonus in the completion score.
pub const TITLE_MATCH_WEIGHT: f32 = 0.25;
/// Weight of the co-occurrence prior in the completion score. v0.1: fixed at zero;
/// related queries are a page-level feature computed after results exist.
pub const CO_OCCURRENCE_WEIGHT: f32 = 0.20;
/// Weight of the length prior in the completion score.
pub const LENGTH_WEIGHT: f32 = 0.15;

/// A ranked completion, ready for the API response.
#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    /// The full suggested string, ready to replace what the user typed.
    pub text: String,
    /// Which corpus dictionary produced it.
    pub kind: SuggestionKind,
    /// Corpus frequency prior, 0…1. **Not** query frequency, which does not exist.
    pub prior: f32,
    /// Corpus support: how many documents back this string.
    pub doc_count: u32,
    /// The weighted score, 0…1, used for the final ordering.
    pub score: f32,
}

/// The curated denylist applied to *suggestion output* only, as defence in depth —
/// never on the index itself.
///
/// The structural filter (the crawler refuses unindexable pages before they reach the
/// dictionary) is the real control; this list exists so a bug in that pipeline cannot
/// surface an obvious category of string through typeahead. Matching is
/// case-insensitive on the lowercased candidate text.
const OUTPUT_DENYLIST: [&str; 6] = ["porn", "xxx", "escort", "casino", "bitcoin doubler", "nsfw"];

/// Rank, deduplicate, and truncate `candidates` into at most `limit` completions.
///
/// `prefix` is the user's current input, used only for the response metadata and as a
/// sanity check: below two characters there is nothing to rank, so the result is empty
/// (the readers already enforce this; repeating it here keeps the crate correct even
/// when handed raw material from elsewhere).
#[must_use]
pub fn complete(prefix: &str, candidates: &[SuggestionCandidate], limit: usize) -> Vec<Completion> {
    let prefix = prefix.trim().to_ascii_lowercase();
    if limit == 0 || prefix.chars().count() < lynx_indexer::reader::MIN_SUGGESTION_PREFIX_LEN {
        return Vec::new();
    }

    // corpus_frequency_prior is document frequency relative to the strongest candidate
    // in this result, so the prior is a ranking among what actually answers the prefix
    // rather than an absolute number that means nothing without corpus size.
    let max_df = candidates.iter().map(|c| c.doc_count).max().unwrap_or(0);

    #[allow(clippy::cast_precision_loss)] // priors are 0…1 floats; precision loss is the point
    let prior_of = |df: u32, max_df: u32| -> f32 { df as f32 / max_df as f32 };

    let mut completions: Vec<(Completion, f32)> = candidates
        .iter()
        .filter(|c| !contains_denied(c.text.as_str()))
        .map(|candidate| {
            let frequency_prior = if max_df == 0 {
                0.0
            } else {
                prior_of(candidate.doc_count, max_df)
            };
            let title_bonus = match candidate.kind {
                SuggestionKind::Title => 1.0,
                SuggestionKind::Term => 0.0,
            };
            let length_prior = length_prior(&candidate.text);
            let score = CORPUS_FREQUENCY_WEIGHT * frequency_prior
                + TITLE_MATCH_WEIGHT * title_bonus
                + CO_OCCURRENCE_WEIGHT * 0.0
                + LENGTH_WEIGHT * length_prior;
            (
                Completion {
                    text: candidate.text.clone(),
                    kind: candidate.kind,
                    prior: frequency_prior,
                    doc_count: candidate.doc_count,
                    score,
                },
                frequency_prior,
            )
        })
        .collect();

    // Deduplicate by text case-insensitively, keeping the first (highest-score)
    // occurrence — same text from two dictionaries, or the same text in different
    // case — and canonicalise the winner to the corpus's lowercase form.
    completions.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.0.score.total_cmp(&a.0.score))
            .then_with(|| a.0.text.cmp(&b.0.text))
    });
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    completions.retain(|(c, _)| seen.insert(c.text.to_ascii_lowercase()));
    for (c, _) in &mut completions {
        c.text = c.text.to_ascii_lowercase();
    }

    // Final order: score descending, ties broken lexicographically for determinism —
    // identical inputs always produce an identical response.
    completions.sort_by(|a, b| {
        b.0.score
            .total_cmp(&a.0.score)
            .then_with(|| a.0.text.cmp(&b.0.text))
    });
    completions
        .into_iter()
        .take(limit)
        .map(|(c, _)| c)
        .collect()
}

/// The length prior: 1.0 for 2–5 words, 0.7 for a single word, decreasing past 5 (a
/// 12-word completion is noise). Candidates from the readers are already ≤ 4 words, so
/// the long tail is unreachable in v0.1 — the function is total anyway.
#[allow(clippy::cast_precision_loss)] // word counts are small integers; the curve is approximate
fn length_prior(text: &str) -> f32 {
    let words = text.split_whitespace().count();
    match words {
        0 => 0.0,
        1 => 0.7,
        2..=5 => 1.0,
        n => (0.7 - 0.1 * (n - 5) as f32).max(0.0),
    }
}

/// Whether a lowercased candidate contains any denylisted substring.
fn contains_denied(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    OUTPUT_DENYLIST
        .iter()
        .any(|blocked| lower.contains(blocked))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lynx_indexer::SuggestionCandidate;

    fn term(text: &str, doc_count: u32) -> SuggestionCandidate {
        SuggestionCandidate {
            text: text.to_string(),
            kind: SuggestionKind::Term,
            doc_count,
        }
    }

    fn title(text: &str, doc_count: u32) -> SuggestionCandidate {
        SuggestionCandidate {
            text: text.to_string(),
            kind: SuggestionKind::Title,
            doc_count,
        }
    }

    #[test]
    fn short_prefixes_and_zero_limits_answer_nothing() {
        let cands = [term("rust", 3)];
        assert!(complete("", &cands, 8).is_empty());
        assert!(complete("r", &cands, 8).is_empty());
        assert!(complete("ru", &cands, 0).is_empty());
    }

    #[test]
    fn the_highest_support_ranks_first() {
        let out = complete(
            "ru",
            &[term("rust", 10), term("runtime", 2), title("rust guide", 1)],
            8,
        );
        assert_eq!(out[0].text, "rust");
        // Both frequency prior and length prior (2 words) favour `rust guide` over
        // the single word `runtime`.
        assert_eq!(out[1].text, "rust guide");
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn scores_are_the_documented_weights() {
        let out = complete("ru", &[term("rust", 4)], 1);
        let c = &out[0];
        assert_eq!(c.prior, 1.0); // sole candidate: strongest by definition
        assert_eq!(
            c.score,
            CORPUS_FREQUENCY_WEIGHT + LENGTH_WEIGHT * 0.7, // single word
        );
    }

    #[test]
    fn emptiness_is_normal_not_an_error() {
        assert!(complete("zzz", &[], 8).is_empty());
    }

    #[test]
    fn duplicates_collapse_to_the_best_score() {
        let out = complete(
            "ru",
            &[term("rust", 5), term("rust", 2), title("RUST", 9)],
            8,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "rust");
        assert_eq!(out[0].doc_count, 9);
    }

    #[test]
    fn output_is_deterministic_for_identical_input() {
        let cands = [
            term("rust", 4),
            term("rusty", 2),
            title("rust async", 1),
            title("rust arms", 1),
            term("rub", 3),
            term("ruby", 1),
            term("rune", 1),
            term("rural", 2),
            term("rust", 4),
            term("rush", 1),
            term("rustic", 1),
        ];
        let a = complete("ru", &cands, 8);
        let b = complete("ru", &cands, 8);
        assert_eq!(a, b);
        assert_eq!(a.len(), 8);
    }

    #[test]
    fn the_denylist_never_reaches_the_output() {
        let out = complete("po", &[term("podcast", 5), term("pornstar", 2)], 8);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "podcast");
    }

    #[test]
    fn title_kind_delegates_to_the_reader_contract() {
        let out = complete(
            "tok",
            &[term("tokio", 1), title("tokio async runtime", 1)],
            8,
        );
        // Tie on prior and length; `tokio async runtime` wins via title bonus and is
        // 3 words (length prior 1.0) vs `tokio` (single word, 0.7).
        assert_eq!(out[0].text, "tokio async runtime");
        assert_eq!(out[0].kind, SuggestionKind::Title);
        assert_eq!(out[1].text, "tokio");
        assert_eq!(out[1].kind, SuggestionKind::Term);
    }
}
