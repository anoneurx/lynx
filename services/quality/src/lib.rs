//! `lynx-quality` — the search quality harness (implementation Step 15).
//!
//! LYNX has no query logs, so quality is measured against a **curated judged set**:
//! queries with human relevance labels (0–3) over documents that exist in the index.
//! The harness runs each judged query through the production pipeline — parse, retrieve,
//! rank — and computes the metrics from `docs/testing/quality-evaluation.md`:
//!
//! | Metric | Target |
//! | ------ | ------ |
//! | NDCG@10  | ≥ 0.65 |
//! | MRR@10   | ≥ 0.55 |
//! | Recall@100 | ≥ 0.80 |
//! | latency p99 | < 200 ms |
//!
//! plus the index-hygiene rates a top-10 can exhibit: duplicate share (results from the
//! same duplicate cluster) and spam share (results the ranker penalised).
//!
//! [`GATES`] pins these as **golden gates**: [`check_gates`] fails on any miss, and the
//! golden test in this crate runs the fixture judge set through the engine-agnostic
//! reader on every `cargo test`, so a ranking regression cannot merge silently.
//!
//! The harness never mutates anything and decides nothing about ordering changes: it
//! measures and reports.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

pub mod eval;
pub mod fixtures;
pub mod gates;
pub mod judged;
pub mod metrics;
pub mod report;

pub use eval::{evaluate, QualityReport, QueryScore};
pub use gates::{check_gates, GateFailure, GateThresholds, GATES};
pub use judged::{JudgedQuery, JudgedSetError, Judgment};
pub use report::render_markdown;

/// The service name, used in logs and metrics.
pub const SERVICE_NAME: &str = "quality";

#[cfg(test)]
mod tests {
    #[test]
    fn service_name_is_stable() {
        assert_eq!(super::SERVICE_NAME, "quality");
    }
}
