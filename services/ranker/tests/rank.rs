//! End-to-end ranking tests over candidates produced by the in-memory reader.
//!
//! These exercise the whole path a real query takes — retrieve, then rank — so the
//! properties the design promises (title beats body, stuffing fails, spam sinks a result)
//! are asserted on the assembled system, not on a hand-built Candidate.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use lynx_indexer::{
    Candidate, CollectionStats, DocId, InMemoryIndex, IndexedDocument, SearchIndexReader,
    SearchOptions,
};
use lynx_query::{parse, QueryPlan};
use lynx_ranker::{Ranker, SignalVector};
use time::macros::datetime;
use time::OffsetDateTime;

/// An owned view of a ranked result, so tests can hold it after the candidate set drops.
struct Hit {
    url: String,
    score: f64,
    spam_penalty: f64,
    negation_penalty: f64,
    signals: SignalVector,
    rank: usize,
}

fn id(n: u128) -> DocId {
    DocId::from_canonical_url(&format!("https://e.com/{n}"))
}

fn doc(n: u128, title: &str, body: &str) -> IndexedDocument {
    let mut d = IndexedDocument::new(id(n), format!("https://e.com/{n}"), title, body);
    d.lang = Some("en".to_string());
    d
}

/// Retrieve candidates for `query`. The returned `Vec<Candidate>` is owned so a caller can
/// rank it without the temporary being dropped.
fn retrieve(
    index: &InMemoryIndex,
    query: &str,
    now: OffsetDateTime,
) -> (QueryPlan, Vec<Candidate>, CollectionStats) {
    let plan = parse(query);
    let opts = SearchOptions {
        now,
        ..SearchOptions::default()
    };
    let candidates = index.search(&plan, &opts).unwrap();
    let stats = index.collection_stats().unwrap();
    (plan, candidates, stats)
}

fn rank_all(index: &InMemoryIndex, query: &str, now: OffsetDateTime) -> Vec<Hit> {
    let (plan, candidates, stats) = retrieve(index, query, now);
    Ranker::default()
        .rank(&plan, &candidates, &stats, now)
        .into_iter()
        .map(|r| Hit {
            url: r.candidate.url.clone(),
            score: r.score,
            spam_penalty: r.spam_penalty,
            negation_penalty: r.negation_penalty,
            signals: r.signals,
            rank: r.rank,
        })
        .collect()
}

fn url_of(r: &Hit) -> String {
    r.url.clone()
}

#[test]
fn title_match_beats_body_match() {
    let mut index = InMemoryIndex::new();
    index.add(doc(1, "Rust programming", "A language."));
    index.add(doc(
        2,
        "A language",
        "Rust programming is discussed here at length.",
    ));

    let ranked = rank_all(&index, "rust", datetime!(2026-01-01 00:00 UTC));
    assert_eq!(url_of(&ranked[0]), "https://e.com/1");
    assert_eq!(ranked[0].rank, 1);
}

#[test]
fn saturation_bounds_and_spam_sinks_a_stuffed_document() {
    // `docs/ranking/bm25f.md`: term-frequency stuffing is handled by *saturation, plus spam
    // penalties*. This asserts both halves.
    let mut index = InMemoryIndex::new();
    index.add(doc(
        1,
        "Rust programming",
        "Rust is a programming language for systems.",
    ));
    let stuffed_body = "rust ".repeat(200);
    let mut stuffed = doc(2, "Unrelated page", &stuffed_body);
    // Spam detection fires on the stuffing; the ranker multiplies the score down.
    stuffed.spam_score = 0.7;
    index.add(stuffed);

    let ranked = rank_all(&index, "rust", datetime!(2026-01-01 00:00 UTC));
    let natural = ranked.iter().find(|r| r.url == "https://e.com/1").unwrap();
    let stuffed = ranked.iter().find(|r| r.url == "https://e.com/2").unwrap();

    // Saturation: 200 repeats give well under 200x the natural document's lexical score.
    assert!(
        stuffed.signals.lexical < 2.0 * natural.signals.lexical,
        "stuffed={} natural={}",
        stuffed.signals.lexical,
        natural.signals.lexical
    );
    // Spam penalty: with stuffing flagged, the stuffed page loses outright.
    assert!(stuffed.spam_penalty < 1.0);
    assert_eq!(url_of(&ranked[0]), "https://e.com/1");
}

#[test]
fn spam_penalty_sinks_an_otherwise_equal_document() {
    let mut index = InMemoryIndex::new();
    index.add(doc(1, "Rust programming", "A language for systems."));
    let mut spammy = doc(2, "Rust programming", "A language for systems.");
    spammy.spam_score = 1.0;
    index.add(spammy);

    let ranked = rank_all(&index, "rust", datetime!(2026-01-01 00:00 UTC));
    let clean = ranked
        .iter()
        .find(|r| url_of(r) == "https://e.com/1")
        .unwrap();
    let spam = ranked
        .iter()
        .find(|r| url_of(r) == "https://e.com/2")
        .unwrap();
    assert!(spam.score < clean.score);
    assert!((spam.spam_penalty - 0.60).abs() < 1e-9);
}

#[test]
fn freshness_prefers_the_newer_document() {
    let mut index = InMemoryIndex::new();
    let mut new = doc(1, "Rust release notes", "Rust release notes and changes.");
    new.published_at = Some(datetime!(2025-12-01 00:00 UTC));
    let mut old = doc(2, "Rust release notes", "Rust release notes and changes.");
    old.published_at = Some(datetime!(2015-01-01 00:00 UTC));
    index.add(new);
    index.add(old);

    let now = datetime!(2026-01-01 00:00 UTC);
    let ranked = rank_all(&index, "rust release notes", now);
    let fresh = ranked
        .iter()
        .find(|r| url_of(r) == "https://e.com/1")
        .unwrap();
    let stale = ranked
        .iter()
        .find(|r| url_of(r) == "https://e.com/2")
        .unwrap();
    assert!(fresh.signals.freshness > stale.signals.freshness);
    assert!(fresh.score > stale.score);
}

#[test]
fn all_signals_are_present_and_bounded() {
    let mut index = InMemoryIndex::new();
    index.add(doc(1, "Rust programming", "A language for systems."));

    let ranked = rank_all(&index, "rust", datetime!(2026-01-01 00:00 UTC));
    let s = &ranked[0].signals;
    assert!(s.lexical >= 0.0);
    assert!((0.0..=1.0).contains(&s.freshness));
    assert!((-1.0..=1.0).contains(&s.quality));
    assert!((0.0..=1.0).contains(&s.authority));
    assert!((0.0..=1.0).contains(&s.context));
    assert!((0.0..=1.0).contains(&s.spam_score));
    assert!((0.0..=1.0).contains(&s.negation));
    assert!((0.0..=1.0).contains(&s.duplicate_penalty));
    assert!(s.lexical_by_field.contains_key("title"));
}

#[test]
fn negation_downranks_but_does_not_exclude() {
    let mut index = InMemoryIndex::new();
    index.add(doc(1, "Rust programming", "A systems language."));
    index.add(doc(
        2,
        "Rust programming",
        "A systems language. It also mentions golang.",
    ));

    let ranked = rank_all(&index, "rust -golang", datetime!(2026-01-01 00:00 UTC));
    // Both documents appear: negation penalises, it never excludes.
    assert_eq!(ranked.len(), 2);
    let clean = ranked
        .iter()
        .find(|r| url_of(r) == "https://e.com/1")
        .unwrap();
    let negated = ranked
        .iter()
        .find(|r| url_of(r) == "https://e.com/2")
        .unwrap();

    assert_eq!(clean.signals.negation, 0.0);
    assert!(negated.signals.negation > 0.0);
    assert!(negated.negation_penalty < 1.0);
    assert!(negated.score < clean.score);
    assert_eq!(url_of(&ranked[0]), "https://e.com/1");
}

#[test]
fn query_term_coverage_reflects_present_terms() {
    let mut index = InMemoryIndex::new();
    index.add(doc(1, "Rust", "A language for systems."));

    let ranked = rank_all(&index, "rust systems", datetime!(2026-01-01 00:00 UTC));
    assert!((ranked[0].signals.query_term_coverage - 1.0).abs() < 1e-9);
}

#[test]
fn ranking_is_deterministic() {
    let mut index = InMemoryIndex::new();
    index.add(doc(1, "Rust programming", "A language."));
    index.add(doc(2, "Rust systems", "Another Rust page."));
    index.add(doc(3, "A page", "Rust appears here."));

    let now = datetime!(2026-01-01 00:00 UTC);
    let first = rank_all(&index, "rust", now);
    let second = rank_all(&index, "rust", now);
    let order_first: Vec<_> = first.iter().map(url_of).collect();
    let order_second: Vec<_> = second.iter().map(url_of).collect();
    assert_eq!(order_first, order_second);
    for (a, b) in first.iter().zip(&second) {
        assert!((a.score - b.score).abs() < 1e-12);
    }
}
