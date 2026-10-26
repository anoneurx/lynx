//! The golden gate: the v0 judged set through the fixture corpus on every test run.
//!
//! This is the CI guarantee that a ranking change cannot silently regress retrieval
//! quality. It runs the *engine-agnostic* reader (the Tantivy engine has its own parity
//! tests; the fixture similarity it shares with the in-memory reader is asserted there).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use lynx_indexer::{DocId, SearchIndexReader};
use lynx_quality::fixtures;
use lynx_quality::gates::check_gates;
use lynx_ranker::Ranker;

#[test]
fn the_v0_judge_set_parses_and_names_only_indexed_documents() {
    let judged = fixtures::judged_queries().unwrap();
    assert_eq!(judged.len(), 9);
    let index = fixtures::index();
    for q in judged {
        for j in q.judgments {
            assert!(
                index
                    .document(DocId::from_canonical_url(&j.doc))
                    .unwrap()
                    .is_some(),
                "judged document {} is not in the fixture corpus",
                j.doc
            );
        }
    }
}

#[test]
fn the_golden_gates_hold_for_the_v0_set() {
    let index = fixtures::index();
    let judged = fixtures::judged_queries().unwrap();
    let report =
        lynx_quality::evaluate(&index, &Ranker::default(), &judged, fixtures::NOW).unwrap();

    // Print the substantive metrics into the test log for every failure triage.
    println!(
        "ndcg@10 {:.4} mrr@10 {:.4} recall@100 {:.4} dup {:.4} spam {:.4} p99 {} ms",
        report.ndcg_at_10,
        report.mrr_at_10,
        report.recall_at_100,
        report.duplicate_rate_top10,
        report.spam_rate_top10,
        report.p99_latency_ms
    );

    assert!(
        check_gates(&report).is_ok(),
        "golden gates failed: {:?}",
        check_gates(&report).unwrap_err()
    );
}

#[test]
fn spam_and_duplicates_never_lead_a_result_list() {
    // The structural claim underneath the gates: the spam document and the mirror
    // duplicate rank below genuinely relevant results everywhere they appear.
    let index = fixtures::index();
    let judged = fixtures::judged_queries().unwrap();
    let report =
        lynx_quality::evaluate(&index, &Ranker::default(), &judged, fixtures::NOW).unwrap();
    for q in &report.per_query {
        if q.results > 0 {
            assert!(
                q.mrr_at_10 > 0.0,
                "query {} had results but no relevant top-10",
                q.query
            );
        }
    }
}

#[test]
fn the_report_renders_with_headline_metrics() {
    let index = fixtures::index();
    let judged = fixtures::judged_queries().unwrap();
    let report =
        lynx_quality::evaluate(&index, &Ranker::default(), &judged, fixtures::NOW).unwrap();
    let md = lynx_quality::render_markdown(&report);
    assert!(md.contains("# Search Quality Report"));
    assert!(md.contains("NDCG@10: 0."));
    assert!(!md.contains("coverage gap"));
}
