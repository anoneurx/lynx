//! Ranked-list metrics: graded-relevance NDCG, MRR, and Recall.
//!
//! All functions are total: empty inputs score `0.0`, and truncation at `k` is part of
//! the metric, mirroring `docs/testing/quality-evaluation.md`.
//!
//! The `as u32` casts below are annotated away deliberately: ranks and judged-set sizes
//! are bounded by the retrieval window and the judge set, both of which fit u32 by
//! construction; `u64` precision loss is the arithmetic the metric intends.
#![allow(clippy::cast_possible_truncation)]

/// The graded gain of a single judged document: `2^relevance − 1` (the standarder
/// convention for 0–3 labels; a "perfect" document contributes disproportionately more
/// than a "maybe", which is the point of graded labels).
#[must_use]
pub fn gain(relevance: u8) -> f64 {
    (f64::from(1_u32 << relevance.min(3))) - 1.0
}

/// Discounted cumulative gain of `ranked_relevance`, in rank order, truncated at `k`.
#[must_use]
pub fn dcg_at_k(ranked_relevance: &[u8], k: usize) -> f64 {
    ranked_relevance
        .iter()
        .take(k)
        .enumerate()
        .map(|(rank, &rel)| gain(rel) / (f64::from(rank as u32 + 2)).log2())
        .sum()
}

/// Normalised DCG@k: the DCG@k of the retrieval order divided by the DCG@k of the
/// ideal (descending) order of the same judged documents. `1.0` when the ideal order
/// contains no gain (nothing judged relevant), `0.0` when nothing was retrieved.
#[must_use]
pub fn ndcg_at_k(ranked_relevance: &[u8], judged_relevance: &[u8], k: usize) -> f64 {
    let mut ideal: Vec<u8> = judged_relevance.to_vec();
    ideal.sort_unstable_by(|a, b| b.cmp(a));
    let dcg = dcg_at_k(ranked_relevance, k);
    let idcg = dcg_at_k(&ideal, k);
    if idcg == 0.0 {
        return 1.0; // nothing judged relevant: an empty retrieval cannot be wrong
    }
    dcg / idcg
}

/// Recall@k: retrieved-relevant (relevance ≥ 1) within `k`, over all judged-relevant
/// documents. `1.0` when there is nothing judged relevant.
#[must_use]
pub fn recall_at_k(retrieved_relevant: usize, k: usize, judged_relevant: usize) -> f64 {
    if judged_relevant == 0 {
        return 1.0;
    }
    let taken = retrieved_relevant.min(k);
    f64::from(taken as u32) / f64::from(judged_relevant as u32)
}

/// MRR@k: the reciprocal of the rank of the *first* relevant (relevance ≥ 1) result
/// within `k`, or `0.0` when none appears. Purely over the retrieval order: queries
/// whose judged set has no relevant document are excluded upstream, at the mean.
#[must_use]
pub fn mrr_at_k(ranked_relevance: &[u8], k: usize) -> f64 {
    for (rank, &rel) in ranked_relevance.iter().take(k).enumerate() {
        if rel >= 1 {
            return 1.0 / f64::from(rank as u32 + 1);
        }
    }
    0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_is_the_pow2_minus_one_convention() {
        assert_eq!(gain(0), 0.0);
        assert_eq!(gain(1), 1.0);
        assert_eq!(gain(2), 3.0);
        assert_eq!(gain(3), 7.0);
        // Labels above the 0–3 vocabulary clamp, never explode.
        assert_eq!(gain(9), gain(3));
    }

    #[test]
    fn dcg_discounts_by_position() {
        // 7 at rank 1 (gain/log2(2)) and 3 at rank 2 (gain/log2(3)).
        let dcg = dcg_at_k(&[3, 2], 2);
        assert!((dcg - (7.0 + 3.0 / 3_f64.log2())).abs() < 1e-9);
        // Truncation at k is part of the metric.
        assert_eq!(dcg_at_k(&[3, 2], 1), 7.0);
    }

    #[test]
    fn ndcg_is_the_ratio_to_the_ideal_order() {
        // Perfect order scores 1.
        assert!((ndcg_at_k(&[3, 2], &[3, 2], 10) - 1.0).abs() < 1e-9);
        // Swapped order: DCG(2,3) over DCG(3,2).
        let swapped = ndcg_at_k(&[2, 3], &[3, 2], 10);
        let expected = (3.0 + 7.0 / 3_f64.log2()) / (7.0 + 3.0 / 3_f64.log2());
        assert!((swapped - expected).abs() < 1e-9, "{swapped}");
        // An irrelevant-only retrieval is the floor, not a degenerate NaN.
        assert_eq!(ndcg_at_k(&[0], &[3], 10), 0.0);
        // Nothing judged relevant vacuously scores 1.
        assert_eq!(ndcg_at_k(&[], &[], 10), 1.0);
    }

    #[test]
    fn recall_counts_relevant_within_k() {
        assert_eq!(recall_at_k(3, 10, 3), 1.0);
        assert_eq!(recall_at_k(2, 10, 3), 2.0 / 3.0);
        // k truncates the reachable relevant count.
        assert_eq!(recall_at_k(5, 2, 5), 2.0 / 5.0);
        assert_eq!(recall_at_k(0, 10, 0), 1.0);
    }

    #[test]
    fn mrr_is_the_reciprocal_of_the_first_relevant() {
        assert_eq!(mrr_at_k(&[0, 2, 3], 10), 0.5);
        assert_eq!(mrr_at_k(&[3], 10), 1.0);
        assert_eq!(mrr_at_k(&[0, 0], 10), 0.0);
        assert_eq!(mrr_at_k(&[], 10), 0.0);
        assert_eq!(mrr_at_k(&[0, 0, 3], 2), 0.0); // relevant result past k
    }
}
