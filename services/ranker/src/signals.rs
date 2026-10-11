//! The non-lexical signals and the bounded arithmetic around them.
//!
//! Every input is clamped before it is weighted, so a feature bug cannot produce an
//! unbounded score (`docs/ranking/overview.md`). Quality, authority, and freshness add;
//! spam subtracts multiplicatively.

use lynx_indexer::Candidate;
use time::OffsetDateTime;

use crate::config::FreshnessConfig;

/// Clamp to `[0, 1]`.
#[must_use]
pub fn clamp01(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}

/// Clamp to `[-1, 1]`.
#[must_use]
pub fn clamp_signed(x: f64) -> f64 {
    x.clamp(-1.0, 1.0)
}

/// On-page quality, already a stored `[-1, 1]` fast field.
#[must_use]
pub fn quality(candidate: &Candidate) -> f64 {
    clamp_signed(f64::from(candidate.quality_score))
}

/// Authority: the greater of host and domain rank, capped to `[0, 1]`.
///
/// Both ranks are log-compressed `PageRank` aggregates; domain rank is itself capped, so
/// authority can never dominate lexical relevance (`docs/ranking/authority.md`).
#[must_use]
pub fn authority(candidate: &Candidate) -> f64 {
    clamp01(f64::from(candidate.host_rank.max(candidate.domain_rank)))
}

/// Context in `[0, 1]`.
///
/// v0.1 computes only the language component. Intent match, safe-to-render, and readability
/// are deferred; they default to their neutral contributions and are documented as gaps in
/// `docs/ranking/overview.md` § Context signal.
#[must_use]
pub fn context(candidate: &Candidate) -> f64 {
    // 0.30 weight on language_confidence. No intent (0.40), always safe to render (0.20),
    // no readability proxy (0.10).
    let language_confidence = if candidate.lang.as_deref().is_some_and(|l| !l.is_empty()) {
        1.0
    } else {
        0.0
    };
    0.30 * language_confidence + 0.20
}

/// Freshness in `[0, 1]`, combining publication, modification, and crawl recency.
///
/// Precedence is `modified_at` over `published_at`, but all present dates contribute, with
/// crawl recency weighted least. A document with no dates scores zero.
#[must_use]
#[allow(clippy::cast_precision_loss)] // ages in days are small; f64 is ample
pub fn freshness(candidate: &Candidate, now: OffsetDateTime, cfg: &FreshnessConfig) -> f64 {
    let mut value = 0.0;
    let mut any = false;

    if let Some(t) = candidate.published_at {
        value += cfg.w_published * decay(age_days(t, now), f64::from(cfg.half_life_days));
        any = true;
    }
    if let Some(t) = candidate.modified_at {
        value += cfg.w_modified * decay(age_days(t, now), f64::from(cfg.half_life_days));
        any = true;
    }
    if let Some(t) = candidate.crawled_at {
        value += cfg.w_crawled * decay(age_days(t, now), f64::from(cfg.crawled_half_life_days));
        any = true;
    }

    if any {
        clamp01(value)
    } else {
        0.0
    }
}

/// Age in days, clamped to zero so clock skew never yields a value above one.
#[allow(clippy::cast_precision_loss)] // ages in days are small; f64 is ample
fn age_days(t: OffsetDateTime, now: OffsetDateTime) -> f64 {
    let seconds = (now - t).whole_seconds().max(0) as f64;
    seconds / 86_400.0
}

/// Exponential decay on a half-life: `0.5 ^ (age / half_life)`.
fn decay(age_days: f64, half_life_days: f64) -> f64 {
    if half_life_days <= 0.0 {
        return 0.0;
    }
    0.5f64.powf(age_days / half_life_days)
}

/// Duplicate-cluster penalty: `1 / (1 + 0.6·(members − 1))`.
///
/// Ten copies of an article produce one strong result, not ten (`docs/ranking/overview.md`
/// § Cluster suppression).
#[must_use]
#[allow(clippy::cast_precision_loss)] // cluster sizes are small
pub fn duplicate_penalty(members: u32) -> f64 {
    if members <= 1 {
        1.0
    } else {
        1.0 / (1.0 + 0.6 * (f64::from(members) - 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lynx_indexer::{DocId, FieldHits, FieldLengths};
    use time::macros::datetime;

    fn candidate(lang: Option<&str>) -> Candidate {
        Candidate {
            doc_id: DocId::from_canonical_url("https://e.com/x"),
            url: "https://e.com/x".to_string(),
            title: "T".to_string(),
            body: "B".to_string(),
            lang: lang.map(ToString::to_string),
            content_type: None,
            published_at: None,
            modified_at: None,
            crawled_at: None,
            quality_score: 0.0,
            spam_score: 0.0,
            host_rank: 0.0,
            domain_rank: 0.0,
            content_length: 0,
            link_count: 0,
            cluster_id: 0,
            bm25: 0.0,
            field_hits: FieldHits::default(),
            field_lengths: FieldLengths::default(),
            terms: Vec::new(),
        }
    }

    #[test]
    fn quality_and_authority_are_clamped() {
        let mut c = candidate(Some("en"));
        c.quality_score = 5.0;
        c.host_rank = 0.4;
        c.domain_rank = 0.9;
        assert_eq!(quality(&c), 1.0);
        assert!((authority(&c) - 0.9).abs() < 1e-6);
    }

    #[test]
    fn context_rewards_declared_language() {
        assert!(context(&candidate(Some("en"))) > context(&candidate(None)));
    }

    #[test]
    fn freshness_sums_all_three_dates() {
        let now = datetime!(2026-01-01 00:00 UTC);
        let mut c = candidate(Some("en"));
        c.published_at = Some(now);
        c.modified_at = Some(now);
        c.crawled_at = Some(now);
        assert!((freshness(&c, now, &FreshnessConfig::default()) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn freshness_is_zero_without_dates() {
        let now = datetime!(2026-01-01 00:00 UTC);
        assert_eq!(
            freshness(&candidate(None), now, &FreshnessConfig::default()),
            0.0
        );
    }

    #[test]
    fn older_documents_are_less_fresh() {
        let now = datetime!(2026-01-01 00:00 UTC);
        let cfg = FreshnessConfig::default();
        let mut recent = candidate(None);
        recent.published_at = Some(now - time::Duration::days(10));
        let mut old = candidate(None);
        old.published_at = Some(now - time::Duration::days(3000));
        assert!(freshness(&recent, now, &cfg) > freshness(&old, now, &cfg));
    }

    #[test]
    fn clamps_are_bounded() {
        assert_eq!(clamp01(-1.0), 0.0);
        assert_eq!(clamp01(2.0), 1.0);
        assert_eq!(clamp_signed(-3.0), -1.0);
        assert_eq!(clamp_signed(3.0), 1.0);
    }

    #[test]
    fn decay_boundaries() {
        assert_eq!(decay(0.0, 90.0), 1.0);
        assert!((decay(90.0, 90.0) - 0.5).abs() < 1e-12);
        assert!((decay(180.0, 90.0) - 0.25).abs() < 1e-12);
        // Negative age (clock skew) is clamped by age_days, not here; decay itself is
        // monotone.
        assert!(decay(-5.0, 90.0) >= 1.0);
    }

    #[test]
    fn duplicate_penalty_shrinks_with_members() {
        assert_eq!(duplicate_penalty(0), 1.0);
        assert_eq!(duplicate_penalty(1), 1.0);
        assert!((duplicate_penalty(2) - 1.0 / 1.6).abs() < 1e-12);
        assert!(duplicate_penalty(10) < duplicate_penalty(2));
    }

    #[test]
    fn age_never_negative_under_clock_skew() {
        let now = datetime!(2026-01-01 00:00 UTC);
        let future = now + time::Duration::days(3);
        assert_eq!(age_days(future, now), 0.0);
    }
}
