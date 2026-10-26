//! The golden quality gates.
//!
//! The thresholds are the documented targets from `docs/testing/quality-evaluation.md`:
//! they are release discipline, not aspirations. Baselines move only through the judge
//! set release process; a gate is never relaxed to make a regression merge.

/// NDCG@10 minimum.
pub const GOLDEN_NDCG_AT_10: f64 = 0.65;
/// MRR@10 minimum.
pub const GOLDEN_MRR_AT_10: f64 = 0.55;
/// Recall@100 minimum.
pub const GOLDEN_RECALL_AT_100: f64 = 0.80;
/// Duplicate share in the top-10, maximum.
pub const GOLDEN_DUPLICATE_RATE_TOP10: f64 = 0.20;
/// Spam-penalised share in the top-10, maximum.
pub const GOLDEN_SPAM_RATE_TOP10: f64 = 0.10;
/// p99 evaluation latency, maximum, in milliseconds.
pub const GOLDEN_P99_MS: u64 = 200;

/// A gate threshold set, so experiments can score against alternatives in code while
/// CI keeps the canonical [`GATES`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GateThresholds {
    pub ndcg_at_10: f64,
    pub mrr_at_10: f64,
    pub recall_at_100: f64,
    pub duplicate_rate_top10: f64,
    pub spam_rate_top10: f64,
    pub p99_latency_ms: u64,
}

/// The canonical golden gates.
pub const GATES: GateThresholds = GateThresholds {
    ndcg_at_10: GOLDEN_NDCG_AT_10,
    mrr_at_10: GOLDEN_MRR_AT_10,
    recall_at_100: GOLDEN_RECALL_AT_100,
    duplicate_rate_top10: GOLDEN_DUPLICATE_RATE_TOP10,
    spam_rate_top10: GOLDEN_SPAM_RATE_TOP10,
    p99_latency_ms: GOLDEN_P99_MS,
};

/// One gate miss.
#[derive(Debug, Clone, PartialEq)]
pub struct GateFailure {
    /// Gate the metric missed.
    pub metric: &'static str,
    /// Observed value.
    pub observed: String,
    /// The golden threshold.
    pub limit: String,
}

impl std::fmt::Display for GateFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "gate {}: {} ({})",
            self.metric, self.observed, self.limit
        )
    }
}

impl GateFailure {
    fn miss(metric: &'static str, observed: f64, limit: f64, high: bool) -> Self {
        Self {
            metric,
            observed: format!("{observed:.4}"),
            limit: if high {
                format!("must be ≥ {limit}")
            } else {
                format!("must be ≤ {limit}")
            },
        }
    }
}

/// Check a report against the golden gates.
///
/// # Errors
/// Returns one [`GateFailure`] per missed gate, in a fixed order (the order above), so
/// a failing CI run lists everything the release would regress.
pub fn check_gates(report: &crate::eval::QualityReport) -> Result<(), Vec<GateFailure>> {
    check_against(report, &GATES)
}

/// Check a report against an explicit threshold set.
///
/// # Errors
/// As [`check_gates`].
pub fn check_against(
    report: &crate::eval::QualityReport,
    gates: &GateThresholds,
) -> Result<(), Vec<GateFailure>> {
    let mut failures = Vec::new();
    if report.ndcg_at_10 < gates.ndcg_at_10 {
        failures.push(GateFailure::miss(
            "ndcg@10",
            report.ndcg_at_10,
            gates.ndcg_at_10,
            true,
        ));
    }
    if report.mrr_at_10 < gates.mrr_at_10 {
        failures.push(GateFailure::miss(
            "mrr@10",
            report.mrr_at_10,
            gates.mrr_at_10,
            true,
        ));
    }
    if report.recall_at_100 < gates.recall_at_100 {
        failures.push(GateFailure::miss(
            "recall@100",
            report.recall_at_100,
            gates.recall_at_100,
            true,
        ));
    }
    if report.duplicate_rate_top10 > gates.duplicate_rate_top10 {
        failures.push(GateFailure::miss(
            "duplicate_rate@top10",
            report.duplicate_rate_top10,
            gates.duplicate_rate_top10,
            false,
        ));
    }
    if report.spam_rate_top10 > gates.spam_rate_top10 {
        failures.push(GateFailure::miss(
            "spam_rate@top10",
            report.spam_rate_top10,
            gates.spam_rate_top10,
            false,
        ));
    }
    if report.p99_latency_ms > gates.p99_latency_ms {
        failures.push(GateFailure {
            metric: "p99_latency_ms",
            observed: report.p99_latency_ms.to_string(),
            limit: format!("must be ≤ {}", gates.p99_latency_ms),
        });
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::QualityReport;

    fn report(ndcg: f64, mrr: f64, recall: f64, dup: f64, spam: f64, p99: u64) -> QualityReport {
        QualityReport {
            query_count: 1,
            ndcg_at_10: ndcg,
            mrr_at_10: mrr,
            recall_at_100: recall,
            duplicate_rate_top10: dup,
            spam_rate_top10: spam,
            mean_latency_ms: p99 as f64,
            p99_latency_ms: p99,
            per_query: Vec::new(),
        }
    }

    #[test]
    fn a_healthy_report_passes() {
        let r = report(0.75, 0.8, 1.0, 0.0, 0.05, 1);
        assert!(check_gates(&r).is_ok());
    }

    #[test]
    fn every_miss_is_reported_in_a_fixed_order() {
        let r = report(0.1, 0.1, 0.1, 0.9, 0.5, 1_000);
        let failures = check_gates(&r).unwrap_err();
        let metrics: Vec<&'static str> = failures.iter().map(|f| f.metric).collect();
        assert_eq!(
            metrics,
            vec![
                "ndcg@10",
                "mrr@10",
                "recall@100",
                "duplicate_rate@top10",
                "spam_rate@top10",
                "p99_latency_ms"
            ]
        );
    }
}
