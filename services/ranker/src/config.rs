//! Ranker configuration: every weight and parameter, versioned.
//!
//! Every number in the ranking function lives here rather than in code, so a result can be
//! attributed to an exact configuration and a change is a reviewable diff
//! (`docs/ranking/overview.md` § Weight philosophy).

use lynx_indexer::FieldWeights;
use serde::{Deserialize, Serialize};

/// The configuration version recorded on every ranked result. Bump on any change.
pub const CONFIG_VERSION: &str = "v0.1.0";

/// Additive and multiplicative weights for the signal set.
///
/// `lexical` has weight 1.00 by definition; its scale is normalised to roughly `[0, 12]`
/// for a strong multi-term match, so no other signal can buy past relevance
/// (`docs/ranking/overview.md`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RankingWeights {
    /// Weight on the lexical (BM25F) signal.
    pub lexical: f64,
    /// Weight on freshness.
    pub freshness: f64,
    /// Weight on quality.
    pub quality: f64,
    /// Weight on authority.
    pub authority: f64,
    /// Weight on context.
    pub context: f64,
    /// Strength of the multiplicative spam penalty. `0.40` means a fully-spam document keeps
    /// `60%` of its raw score.
    pub spam_penalty: f64,
    /// Strength of the multiplicative negation penalty. `0.50` means a document containing
    /// every negated term at full intensity keeps `50%` of its raw score. Negation downranks
    /// but never excludes (`docs/search/query-parsing.md` § Filters vs scoring).
    pub negation_penalty: f64,
}

impl Default for RankingWeights {
    fn default() -> Self {
        Self {
            lexical: 1.00,
            freshness: 0.18,
            quality: 0.12,
            authority: 0.15,
            context: 0.10,
            spam_penalty: 0.40,
            negation_penalty: 0.50,
        }
    }
}

/// BM25F parameters (`docs/ranking/bm25f.md` § Tuning).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bm25Params {
    /// Term-frequency saturation for a single field.
    pub k1: f32,
    /// Saturation applied to the fused term frequency (the `k₁'` of BM25F).
    pub k1_fused: f32,
    /// Length-normalisation strength for the title.
    pub b_title: f32,
    /// Length-normalisation strength for headings.
    pub b_headings: f32,
    /// Length-normalisation strength for the body.
    pub b_body: f32,
    /// Length-normalisation strength for anchors, URL, and alt.
    pub b_other: f32,
    /// Lower bound on IDF, so a very common term still contributes a little rather than
    /// exactly nothing.
    pub idf_floor: f32,
}

impl Default for Bm25Params {
    fn default() -> Self {
        Self {
            k1: 1.4,
            k1_fused: 2.0,
            b_title: 1.0,
            b_headings: 1.0,
            b_body: 0.5,
            b_other: 0.75,
            idf_floor: 0.0,
        }
    }
}

/// Freshness decay parameters (`docs/ranking/freshness.md` § Decay).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FreshnessConfig {
    /// Half-life for publication and modification recency, in days.
    pub half_life_days: f32,
    /// Half-life for crawl recency, in days. Longer because crawl recency reflects our
    /// attention, not the page's importance.
    pub crawled_half_life_days: f32,
    /// Weight on publication recency.
    pub w_published: f64,
    /// Weight on modification recency.
    pub w_modified: f64,
    /// Weight on crawl recency.
    pub w_crawled: f64,
}

impl Default for FreshnessConfig {
    fn default() -> Self {
        Self {
            half_life_days: 90.0,
            crawled_half_life_days: 365.0,
            w_published: 0.5,
            w_modified: 0.3,
            w_crawled: 0.2,
        }
    }
}

/// The full ranking configuration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct RankingConfig {
    /// Additive and multiplicative weights.
    pub weights: RankingWeights,
    /// BM25F parameters.
    pub bm25: Bm25Params,
    /// Reader-side field weights.
    pub field_weights: FieldWeights,
    /// Freshness decay parameters.
    pub freshness: FreshnessConfig,
}
