//! `lynx-ranker` — transparent multi-signal ranking.
//!
//! Scores a candidate set with an explicit, explainable combination of BM25F, freshness,
//! quality, authority, and context, then applies a multiplicative spam penalty. Every
//! result carries its full signal vector, so a ranking complaint is answerable from the
//! result itself (`docs/ranking/explainability.md`).
//!
//! ```text
//! score_raw   = lexical + 0.18·freshness + 0.12·quality + 0.15·authority + 0.10·context
//! score_final = score_raw × (1 − 0.40·clamp(spam,0,1)) × duplicate_penalty
//! ```
//!
//! The ranker is engine-free: it consumes [`Candidate`]s from the index reader and the
//! caller's [`CollectionStats`]. It never touches Tantivy.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::float_cmp
    )
)]
#![forbid(unsafe_code)]

pub mod bm25f;
pub mod config;
pub mod signals;

use std::collections::{BTreeMap, BTreeSet};

use lynx_indexer::{Candidate, CollectionStats, DocId};
use lynx_query::QueryPlan;
use time::OffsetDateTime;

pub use config::{Bm25Params, FreshnessConfig, RankingConfig, RankingWeights, CONFIG_VERSION};

/// The service name, used in logs and metrics.
pub const SERVICE_NAME: &str = "ranker";

/// Every signal computed for one result, always all of them.
///
/// Completeness is a test obligation, not an intention: a missing signal is a failure
/// (`docs/ranking/explainability.md` § Testing).
#[derive(Debug, Clone, PartialEq)]
pub struct SignalVector {
    /// Lexical relevance (weighted BM25F).
    pub lexical: f64,
    /// The raw BM25F score, before the lexical weight.
    pub bm25f: f64,
    /// Lexical contribution attributed to each field.
    pub lexical_by_field: BTreeMap<String, f64>,
    /// Freshness in `[0, 1]`.
    pub freshness: f64,
    /// Quality in `[-1, 1]`.
    pub quality: f64,
    /// Authority in `[0, 1]`.
    pub authority: f64,
    /// Context in `[0, 1]`.
    pub context: f64,
    /// Spam score in `[0, 1]`.
    pub spam_score: f64,
    /// Duplicate-cluster penalty in `(0, 1]`.
    pub duplicate_penalty: f64,
    /// The duplicate cluster, if any.
    pub cluster_id: Option<u64>,
    /// Fraction of the query's unique terms present in this document.
    pub query_term_coverage: f64,
}

/// One ranked result, borrowing its candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct RankedResult<'a> {
    /// The candidate this result ranks.
    pub candidate: &'a Candidate,
    /// Final score after penalties.
    pub score: f64,
    /// Score before multiplicative penalties.
    pub score_raw: f64,
    /// The multiplicative spam penalty applied.
    pub spam_penalty: f64,
    /// The full signal vector.
    pub signals: SignalVector,
    /// 1-based rank within the returned set.
    pub rank: usize,
}

impl RankedResult<'_> {
    /// The document identity.
    #[must_use]
    pub fn doc_id(&self) -> DocId {
        self.candidate.doc_id
    }
}

/// The ranker: a configuration plus the pure functions that apply it.
#[derive(Debug, Clone, Copy)]
pub struct Ranker {
    config: RankingConfig,
}

impl Ranker {
    /// A ranker with an explicit configuration.
    #[must_use]
    pub fn new(config: RankingConfig) -> Self {
        Self { config }
    }

    /// The ranker's configuration.
    #[must_use]
    pub fn config(&self) -> &RankingConfig {
        &self.config
    }

    /// Rank `candidates`, best-first, against `plan` and `stats`.
    ///
    /// The order is deterministic: ties break on `doc_id` ascending, so the same
    /// generation, config, and candidate set always produce the same order.
    #[must_use]
    pub fn rank<'a>(
        &self,
        plan: &QueryPlan,
        candidates: &'a [Candidate],
        stats: &CollectionStats,
        now: OffsetDateTime,
    ) -> Vec<RankedResult<'a>> {
        let query_terms = unique_query_terms(plan);
        let mut results: Vec<RankedResult<'a>> = candidates
            .iter()
            .map(|c| self.score(c, stats, now, &query_terms))
            .collect();

        results.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.candidate.doc_id.cmp(&b.candidate.doc_id))
        });
        for (i, r) in results.iter_mut().enumerate() {
            r.rank = i + 1;
        }
        results
    }

    /// Score one candidate without ordering.
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // term counts are far below f64's exact range
    pub fn score<'a>(
        &self,
        candidate: &'a Candidate,
        stats: &CollectionStats,
        now: OffsetDateTime,
        query_terms: &BTreeSet<String>,
    ) -> RankedResult<'a> {
        let lexical = bm25f::score(
            &candidate.terms,
            candidate.field_lengths,
            stats,
            &self.config,
        );
        let freshness = signals::freshness(candidate, now, &self.config.freshness);
        let quality = signals::quality(candidate);
        let authority = signals::authority(candidate);
        let context = signals::context(candidate);
        let spam_score = signals::clamp01(f64::from(candidate.spam_score));

        let w = &self.config.weights;
        let score_raw = w.lexical * f64::from(lexical.total)
            + w.freshness * freshness
            + w.quality * quality
            + w.authority * authority
            + w.context * context;
        let spam_penalty = 1.0 - w.spam_penalty * spam_score;
        let duplicate_penalty = 1.0;
        let score = score_raw * spam_penalty * duplicate_penalty;

        let coverage = if query_terms.is_empty() {
            1.0
        } else {
            let present = candidate
                .terms
                .iter()
                .filter(|t| query_terms.contains(&t.term))
                .count();
            present as f64 / query_terms.len() as f64
        };

        let signals = SignalVector {
            lexical: f64::from(lexical.total),
            bm25f: f64::from(lexical.total),
            lexical_by_field: lexical
                .by_field
                .into_iter()
                .map(|(k, v)| (k, f64::from(v)))
                .collect(),
            freshness,
            quality,
            authority,
            context,
            spam_score,
            duplicate_penalty,
            cluster_id: if candidate.cluster_id == 0 {
                None
            } else {
                Some(candidate.cluster_id)
            },
            query_term_coverage: coverage,
        };

        RankedResult {
            candidate,
            score,
            score_raw,
            spam_penalty,
            signals,
            rank: 0,
        }
    }
}

impl Default for Ranker {
    fn default() -> Self {
        Self::new(RankingConfig::default())
    }
}

/// The set of unique query terms a retrieval would have matched on.
#[must_use]
pub fn unique_query_terms(plan: &QueryPlan) -> BTreeSet<String> {
    plan.must
        .iter()
        .chain(&plan.should)
        .map(|t| t.text().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn service_name_is_stable() {
        assert_eq!(super::SERVICE_NAME, "ranker");
    }
}
