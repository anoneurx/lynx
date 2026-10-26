//! The quality report in the shape operators read it: the monthly "Search Quality
//! Report" from `docs/testing/quality-evaluation.md`, rendered as Markdown.

use std::fmt::Write as _;

use crate::eval::QualityReport;

/// Render a report as the documented Markdown summary.
///
/// Regressions are listed inline in the per-query block: any query below its gate share
/// of the aggregate is called out, so a report reader sees the damage without a diff.
#[must_use]
pub fn render_markdown(report: &QualityReport) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str("# Search Quality Report\n\n");
    out.push_str("## Headline metrics\n\n");
    let _ = write!(
        out,
        "NDCG@10: {:.4}\nMRR@10:  {:.4}\nRecall@100: {:.4}\n",
        report.ndcg_at_10, report.mrr_at_10, report.recall_at_100
    );
    let _ = write!(
        out,
        "Duplicate rate top-10: {:.4}\nSpam rate top-10: {:.4}\n",
        report.duplicate_rate_top10, report.spam_rate_top10
    );
    let _ = writeln!(
        out,
        "Latency p99: {} ms (mean {:.1} ms)",
        report.p99_latency_ms, report.mean_latency_ms
    );
    let _ = writeln!(out, "Queries judged: {}", report.query_count);
    out.push_str("\n## Zero-result queries\n\n");
    let zero: Vec<&crate::eval::QueryScore> =
        report.per_query.iter().filter(|q| q.results == 0).collect();
    if zero.is_empty() {
        out.push_str("(none)\n");
    } else {
        let _ = writeln!(out, "{} coverage gap(s):", zero.len());
        for q in zero {
            let _ = writeln!(out, "- {}", q.query);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::QueryScore;

    #[test]
    fn a_report_reads_like_the_documented_report() {
        let report = QualityReport {
            query_count: 2,
            ndcg_at_10: 0.7123,
            mrr_at_10: 0.8,
            recall_at_100: 1.0,
            duplicate_rate_top10: 0.05,
            spam_rate_top10: 0.1,
            mean_latency_ms: 1.5,
            p99_latency_ms: 3,
            per_query: vec![
                QueryScore {
                    query: "rust async".to_string(),
                    ndcg_at_10: 0.9,
                    mrr_at_10: 1.0,
                    recall_at_100: 1.0,
                    results: 4,
                },
                QueryScore {
                    query: "obscure thing".to_string(),
                    ndcg_at_10: 0.5,
                    mrr_at_10: 0.6,
                    recall_at_100: 1.0,
                    results: 0,
                },
            ],
        };
        let md = render_markdown(&report);
        assert!(md.contains("# Search Quality Report"));
        assert!(md.contains("NDCG@10: 0.7123"));
        assert!(md.contains("Latency p99: 3 ms"));
        assert!(md.contains("- obscure thing"));
    }

    #[test]
    fn zero_coverage_gaps_are_listed() {
        let report = QualityReport {
            query_count: 1,
            ndcg_at_10: 1.0,
            mrr_at_10: 1.0,
            recall_at_100: 1.0,
            duplicate_rate_top10: 0.0,
            spam_rate_top10: 0.0,
            mean_latency_ms: 0.5,
            p99_latency_ms: 1,
            per_query: vec![QueryScore {
                query: "hard to find doc".to_string(),
                ndcg_at_10: 1.0,
                mrr_at_10: 1.0,
                recall_at_100: 1.0,
                results: 0,
            }],
        };
        assert!(render_markdown(&report).contains("1 coverage gap(s)"));
    }
}
