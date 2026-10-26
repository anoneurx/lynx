//! The evaluation driver: judged queries in, [`QualityReport`] out.
//!
//! Each judged query runs the *production* pipeline — `lynx_query::parse`, the
//! [`SearchIndexReader`] retrieval, and the [`Ranker`] — so the harness measures what
//! ships, not a simulacrum of it.

use std::time::Instant;

use lynx_indexer::{DocId, SearchIndexReader, SearchOptions};
use lynx_ranker::Ranker;
use time::OffsetDateTime;

use crate::judged::JudgedQuery;
use crate::metrics;

/// Per-query headline metrics, in judged-set order.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryScore {
    /// The query text, verbatim from the judged set.
    pub query: String,
    /// NDCG@10 of the ranked top-10 against the judged labels.
    pub ndcg_at_10: f64,
    /// MRR@10 of the ranked top-10.
    pub mrr_at_10: f64,
    /// Recall@100 over the retrieval window.
    pub recall_at_100: f64,
    /// How many results the query produced at all (0 means a coverage gap).
    pub results: usize,
}

/// Aggregate quality of one judged set against one index + ranker.
#[derive(Debug, Clone, PartialEq)]
pub struct QualityReport {
    /// Judged queries evaluated.
    pub query_count: usize,
    /// Mean NDCG@10.
    pub ndcg_at_10: f64,
    /// Mean MRR@10.
    pub mrr_at_10: f64,
    /// Mean Recall@100.
    pub recall_at_100: f64,
    /// Share of top-10 slots occupied by a result from a duplicate cluster already
    /// seen among that query's top slots. The lower the better.
    pub duplicate_rate_top10: f64,
    /// Share of top-10 slots occupied by a result the ranker penalised for spam
    /// (`spam_penalty < 1`). The lower the better.
    pub spam_rate_top10: f64,
    /// Mean retrieval+ranking latency per query, in milliseconds.
    pub mean_latency_ms: f64,
    /// p99 latency across the evaluated queries (fixture sets are tiny, so this is the
    /// max — the honest worst case for a one-off run).
    pub p99_latency_ms: u64,
    /// Per-query metrics, in judged-set order.
    pub per_query: Vec<QueryScore>,
}

/// Why the evaluation cannot produce a report.
#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    /// The index could not answer a query.
    #[error("index unavailable during evaluation: {0}")]
    Index(String),
    /// A judgment names a document that is not in the index: the judge set and the
    /// corpus have drifted apart, which is a data bug, not a quality signal.
    #[error("judged document {doc:?} is missing from the index (judge set and corpus diverged)")]
    UnknownDoc {
        /// The offending judged document.
        doc: String,
    },
}

/// Run the complete judged set through retrieval + ranking and score it.
///
/// `now` is passed to the pipeline for freshness purposes; pass a fixed instant in
/// tests and real time in periodic evaluation runs.
///
/// # Errors
/// [`EvalError::Index`] when retrieval fails, or [`EvalError::UnknownDoc`] when a judged
/// document is missing from the index (corpus/judge-set divergence).
pub fn evaluate(
    index: &dyn SearchIndexReader,
    ranker: &Ranker,
    judged: &[JudgedQuery],
    now: OffsetDateTime,
) -> Result<QualityReport, EvalError> {
    // Judge/corpus integrity first: a judgment for a missing document means the data
    // drifted, and every metric computed after that would be fiction.
    for q in judged {
        for j in &q.judgments {
            let known = index
                .document(DocId::from_canonical_url(&j.doc))
                .map_err(|e| EvalError::Index(e.to_string()))?;
            if known.is_none() {
                return Err(EvalError::UnknownDoc { doc: j.doc.clone() });
            }
        }
    }

    let opts = SearchOptions {
        limit: 100,
        offset: 0,
        now,
        ..SearchOptions::default()
    };

    let mut per_query = Vec::with_capacity(judged.len());
    let mut latency_ms: Vec<u64> = Vec::with_capacity(judged.len());
    let mut top10_slots = 0_usize;
    let mut duplicate_slots = 0_usize;
    let mut spam_slots = 0_usize;

    for q in judged {
        let started = Instant::now();
        let plan = lynx_query::parse(&q.query);
        let candidates = index
            .search(&plan, &opts)
            .map_err(|e| EvalError::Index(e.to_string()))?;
        let stats = index
            .collection_stats()
            .map_err(|e| EvalError::Index(e.to_string()))?;
        let ranked_results = ranker.rank(&plan, &candidates, &stats, now);
        latency_ms.push(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));

        let ranked_relevance: Vec<u8> = ranked_results
            .iter()
            .map(|r| {
                q.judgments
                    .iter()
                    .find(|j| j.doc == r.candidate.url)
                    .map_or(0_u8, |j| j.relevance)
            })
            .collect();
        let judged_relevance: Vec<u8> = q.judgments.iter().map(|j| j.relevance).collect();

        per_query.push(QueryScore {
            query: q.query.clone(),
            ndcg_at_10: metrics::ndcg_at_k(&ranked_relevance, &judged_relevance, 10),
            mrr_at_10: metrics::mrr_at_k(&ranked_relevance, 10),
            recall_at_100: metrics::recall_at_k(
                ranked_relevance.iter().filter(|&&r| r >= 1).count(),
                100,
                judged_relevance.iter().filter(|&&r| r >= 1).count(),
            ),
            results: ranked_results.len(),
        });

        // Top-10 hygiene: duplicates after the first cluster member and spam-penalised
        // results are both advantages the list should not squander slots on.
        for (rank, r) in ranked_results.iter().take(10).enumerate() {
            top10_slots += 1;
            let seen_earlier = ranked_results[..rank].iter().any(|earlier| {
                earlier.candidate.cluster_id != 0
                    && earlier.candidate.cluster_id == r.candidate.cluster_id
            });
            if seen_earlier {
                duplicate_slots += 1;
            }
            if r.spam_penalty < 1.0 {
                spam_slots += 1;
            }
        }
    }

    let count = judged.len();
    Ok(QualityReport {
        query_count: count,
        ndcg_at_10: mean_of(&per_query.iter().map(|q| q.ndcg_at_10).collect::<Vec<f64>>()),
        // MRR is undefined for queries with no relevant judgment: such a query asks
        // nothing, so it is excluded from the mean rather than counted as a success.
        mrr_at_10: mean_of(
            &per_query
                .iter()
                .zip(judged)
                .filter(|(_, q)| q.judgments.iter().any(crate::judged::Judgment::is_relevant))
                .map(|(score, _)| score.mrr_at_10)
                .collect::<Vec<f64>>(),
        ),
        recall_at_100: mean_of(
            &per_query
                .iter()
                .map(|q| q.recall_at_100)
                .collect::<Vec<f64>>(),
        ),
        duplicate_rate_top10: ratio(duplicate_slots, top10_slots),
        spam_rate_top10: ratio(spam_slots, top10_slots),
        mean_latency_ms: mean_of_u64(&latency_ms),
        p99_latency_ms: latency_ms.iter().copied().max().unwrap_or(0),
        per_query,
    })
}

/// The mean of a non-empty metric stream; an empty stream means nothing was evaluated,
/// which is defined as `0.0` so reports over thin judge sets stay total.
#[allow(clippy::cast_precision_loss)] // millisecond means are small; u64→f64 is the report arithmetic
fn mean_of(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / f64::from(u32::try_from(values.len()).unwrap_or(u32::MAX))
    }
}

fn mean_of_u64(values: &[u64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        // The sum of tiny per-query latencies cannot overflow u64 in practice; the
        // saturating sum keeps even an adversarial judge set total.
        #[allow(clippy::cast_precision_loss)] // u64→f64 is the report arithmetic
        {
            values.iter().copied().fold(0_u64, u64::saturating_add) as f64 / values.len() as f64
        }
    }
}

/// Slots / slots with the empty-list guard.
fn ratio(small: usize, big: usize) -> f64 {
    if big == 0 {
        return 0.0;
    }
    let small = u32::try_from(small).unwrap_or(u32::MAX);
    let big = u32::try_from(big).unwrap_or(u32::MAX);
    f64::from(small) / f64::from(big)
}
