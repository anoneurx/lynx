//! BM25F — the lexical core (`docs/ranking/bm25f.md`).
//!
//! Each field's raw term frequency is normalised by its own length before weighting, then
//! saturation is applied **once** to the fused value. Summing separate per-field BM25
//! scores instead would let the longest field dominate.
//!
//! ```text
//! TF̃(t, d) = Σ_f w_f · tf(t,d,f) / (1 − b_f + b_f · len_f(d)/avglen_f)
//! score(d)  = Σ_t IDF(t) · TF̃(t,d) · (k₁'+1) / (k₁' + TF̃(t,d))
//! IDF(t)    = ln( 1 + (N − df(t) + 0.5) / (df(t) + 0.5) )
//! ```

use std::collections::BTreeMap;

use lynx_indexer::{CollectionStats, FieldLengths, TermFrequency};

use crate::config::RankingConfig;

/// The lexical score of one document, with per-field and per-term provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct LexicalScore {
    /// Total BM25F score.
    pub total: f32,
    /// Contribution attributed to each field, summing to [`Self::total`].
    pub by_field: BTreeMap<String, f32>,
    /// Per-term contributions.
    pub terms: Vec<TermContribution>,
}

/// One query term's contribution to a document's lexical score.
#[derive(Debug, Clone, PartialEq)]
pub struct TermContribution {
    /// The term.
    pub term: String,
    /// The fused, length-normalised term frequency.
    pub fused_tf: f32,
    /// The term's inverse document frequency.
    pub idf: f32,
    /// The term's contribution to the total.
    pub contribution: f32,
}

/// The six fields, in the weight order used by `field_weights`.
struct FieldView {
    name: &'static str,
    tf: u32,
    len: f32,
    avg: f32,
    weight: f32,
    b: f32,
}

/// Score a document's query terms.
#[must_use]
pub fn score(
    terms: &[TermFrequency],
    lengths: FieldLengths,
    stats: &CollectionStats,
    cfg: &RankingConfig,
) -> LexicalScore {
    let n = stats.doc_count;
    if n == 0 {
        return LexicalScore {
            total: 0.0,
            by_field: BTreeMap::new(),
            terms: Vec::new(),
        };
    }

    let mut total = 0.0f32;
    let mut by_field: BTreeMap<String, f32> = BTreeMap::new();
    let mut contributions = Vec::with_capacity(terms.len());

    for term in terms {
        let idf = idf(term.df, n, cfg.bm25.idf_floor);

        let fields = [
            field(
                "title",
                term.tf.title,
                lengths.title,
                stats.avg.title,
                cfg.field_weights.title,
                cfg.bm25.b_title,
            ),
            field(
                "headings",
                term.tf.headings,
                lengths.headings,
                stats.avg.headings,
                cfg.field_weights.headings,
                cfg.bm25.b_headings,
            ),
            field(
                "anchors",
                term.tf.anchors,
                lengths.anchors,
                stats.avg.anchors,
                cfg.field_weights.anchors,
                cfg.bm25.b_other,
            ),
            field(
                "url",
                term.tf.url,
                lengths.url,
                stats.avg.url,
                cfg.field_weights.url,
                cfg.bm25.b_other,
            ),
            field(
                "body",
                term.tf.body,
                lengths.body,
                stats.avg.body,
                cfg.field_weights.body,
                cfg.bm25.b_body,
            ),
            field(
                "alt",
                term.tf.alt,
                lengths.alt,
                stats.avg.alt,
                cfg.field_weights.alt,
                cfg.bm25.b_other,
            ),
        ];

        let mut fused = 0.0f32;
        // Per-field weighted normalised frequencies, kept for attribution.
        let mut components = [0.0f32; 6];
        for (i, f) in fields.iter().enumerate() {
            let c = normalised(f);
            components[i] = c;
            fused += c;
        }

        let (contribution, saturation) = if fused > 0.0 {
            let sat = (cfg.bm25.k1_fused + 1.0) / (cfg.bm25.k1_fused + fused);
            (idf * fused * sat, sat)
        } else {
            (0.0, 0.0)
        };

        total += contribution;
        for (i, f) in fields.iter().enumerate() {
            *by_field.entry(f.name.to_string()).or_insert(0.0) += idf * saturation * components[i];
        }

        contributions.push(TermContribution {
            term: term.term.clone(),
            fused_tf: fused,
            idf,
            contribution,
        });
    }

    LexicalScore {
        total,
        by_field,
        terms: contributions,
    }
}

fn field(name: &'static str, tf: u32, len: u32, avg: f32, weight: f32, b: f32) -> FieldView {
    #[allow(clippy::cast_precision_loss)] // token counts fit f32 exactly
    FieldView {
        name,
        tf,
        len: len as f32,
        avg,
        weight,
        b,
    }
}

/// One field's weighted, length-normalised term frequency.
fn normalised(f: &FieldView) -> f32 {
    if f.tf == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)] // term frequencies are tiny; f32 is ample
    let tf = f.tf as f32;
    // An empty corpus or empty field is length-neutral rather than a division by zero.
    let ratio = if f.avg > 0.0 && f.len > 0.0 {
        f.len / f.avg
    } else {
        1.0
    };
    let denom = 1.0 - f.b + f.b * ratio;
    let denom = if denom > 0.0 { denom } else { 1.0 };
    f.weight * tf / denom
}

/// Robertson–Walker IDF with a floor.
#[allow(clippy::cast_precision_loss)] // document counts fit f32 exactly
fn idf(df: u64, n: u64, floor: f32) -> f32 {
    let df = df.min(n) as f32;
    let n = n as f32;
    let raw = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
    raw.max(floor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Bm25Params;
    use lynx_indexer::{AvgFieldLengths, FieldHits};
    use proptest::prelude::*;

    fn lengths(body: u32) -> FieldLengths {
        FieldLengths {
            body,
            ..FieldLengths::default()
        }
    }

    fn stats(avg_body: f32, n: u64) -> CollectionStats {
        CollectionStats {
            doc_count: n,
            avg: AvgFieldLengths {
                body: avg_body,
                ..AvgFieldLengths::default()
            },
        }
    }

    fn term_in_body(df: u64, tf: u32) -> TermFrequency {
        TermFrequency {
            term: "rust".to_string(),
            df,
            tf: FieldHits {
                body: tf,
                ..FieldHits::default()
            },
        }
    }

    #[test]
    fn single_field_matches_hand_computation() {
        // N=1, df=1 => idf = ln(1 + 0.5/1.5) = 0.287682
        // len == avg => fused = w_body * tf = 2.0
        // sat = (2.0+1)/(2.0+2.0) = 0.75
        // score = 0.287682 * 2.0 * 0.75 = 0.431523
        let s = score(
            &[term_in_body(1, 1)],
            lengths(10),
            &stats(10.0, 1),
            &RankingConfig::default(),
        );
        assert!((s.total - 0.431_523).abs() < 1e-5, "got {}", s.total);
        assert_eq!(s.terms.len(), 1);
        assert!((s.by_field["body"] - s.total).abs() < 1e-6);
    }

    #[test]
    fn absent_term_scores_zero() {
        let s = score(&[], lengths(10), &stats(10.0, 1), &RankingConfig::default());
        assert_eq!(s.total, 0.0);
    }

    #[test]
    fn empty_corpus_scores_zero() {
        let s = score(
            &[term_in_body(0, 1)],
            lengths(10),
            &stats(10.0, 0),
            &RankingConfig::default(),
        );
        assert_eq!(s.total, 0.0);
    }

    #[test]
    fn idf_floor_is_applied() {
        // A term in every document has raw idf ~ 0; the floor lifts it.
        let cfg = RankingConfig {
            bm25: Bm25Params {
                idf_floor: 1.0,
                ..Bm25Params::default()
            },
            ..RankingConfig::default()
        };
        let s = score(&[term_in_body(10, 1)], lengths(10), &stats(10.0, 10), &cfg);
        assert!(s.total > 0.0);
    }

    proptest! {
        #[test]
        fn score_is_non_decreasing_in_tf(tf in 0u32..20) {
            let cfg = RankingConfig::default();
            let low = score(&[term_in_body(1, tf)], lengths(10), &stats(10.0, 1), &cfg).total;
            let high = score(&[term_in_body(1, tf + 1)], lengths(10), &stats(10.0, 1), &cfg).total;
            prop_assert!(high >= low);
        }

        #[test]
        fn score_is_non_increasing_in_length(len in 1u32..1000) {
            let cfg = RankingConfig::default();
            let short = score(&[term_in_body(1, 1)], lengths(len), &stats(200.0, 1), &cfg).total;
            let long = score(&[term_in_body(1, 1)], lengths(len + 1), &stats(200.0, 1), &cfg).total;
            prop_assert!(long <= short + 1e-6);
        }

        #[test]
        fn more_query_terms_score_at_least_as_high(extra in 0u32..5) {
            let cfg = RankingConfig::default();
            let base = score(&[term_in_body(1, 1)], lengths(10), &stats(10.0, 1), &cfg).total;
            let mut terms = vec![term_in_body(1, 1)];
            for _ in 0..extra {
                terms.push(term_in_body(1, 1));
            }
            let more = score(&terms, lengths(10), &stats(10.0, 1), &cfg).total;
            prop_assert!(more >= base);
        }
    }
}
