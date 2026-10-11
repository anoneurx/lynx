//! Integration tests for the index abstraction.
//!
//! These test against `&dyn SearchIndexReader` only — never a concrete engine — so they
//! hold for the in-memory fake and the Tantivy reader alike. See
//! `docs/indexing/index-schema.md`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation
)]

use lynx_common::ids::DocumentId;
use lynx_indexer::{InMemoryIndex, IndexedDocument, SearchIndexReader, SearchOptions};
use lynx_query::{parse, QueryPlan};
use time::macros::datetime;

fn id(n: u128) -> DocumentId {
    DocumentId::from_canonical_url(&format!("lynx://test/{n}"))
}

fn doc(idn: u128, url: &str, title: &str, body: &str) -> IndexedDocument {
    let mut d = IndexedDocument::new(id(idn), url, title, body);
    d.lang = Some("en".to_string());
    d.content_type = Some("text/html".to_string());
    d
}

fn sample() -> InMemoryIndex {
    let mut index = InMemoryIndex::new();

    let mut rust = doc(
        1,
        "https://docs.rs/tokio",
        "Tokio async runtime",
        "An asynchronous runtime for Rust.",
    );
    rust.published_at = Some(datetime!(2025-06-01 00:00 UTC));
    rust.quality_score = 0.9;
    index.add(rust);

    let mut zig = doc(
        2,
        "https://ziglang.org/learn",
        "Learning Zig",
        "Zig is a general-purpose language.",
    );
    zig.published_at = Some(datetime!(2023-01-01 00:00 UTC));
    index.add(zig);

    let mut pdf = doc(
        3,
        "https://example.com/report.pdf",
        "Annual report",
        "Rust adoption in industry.",
    );
    pdf.content_type = Some("application/pdf".to_string());
    pdf.published_at = Some(datetime!(2026-01-15 00:00 UTC));
    index.add(pdf);

    // No date: must be excluded by any date filter.
    let undated = doc(
        4,
        "https://example.com/undated",
        "Undated page",
        "Rust and Zig together.",
    );
    index.add(undated);

    index
}

fn opts() -> SearchOptions {
    SearchOptions {
        now: datetime!(2026-02-18 00:00 UTC),
        ..SearchOptions::default()
    }
}

fn search(index: &dyn SearchIndexReader, q: &str) -> Vec<lynx_indexer::Candidate> {
    index.search(&parse(q), &opts()).unwrap()
}

#[test]
fn all_terms_must_be_present() {
    let index = sample();
    let hits = search(&index, "rust async");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].url, "https://docs.rs/tokio");
}

#[test]
fn disjunction_requires_one() {
    let index = sample();
    // "rust" matches docs 1 and 3 and 4; "tokio" matches doc 1.
    let hits = search(&index, "tokio OR zig");
    let urls: Vec<_> = hits.iter().map(|c| c.url.as_str()).collect();
    assert!(urls.contains(&"https://docs.rs/tokio"));
    assert!(urls.contains(&"https://ziglang.org/learn"));
}

#[test]
fn phrase_requires_adjacency() {
    let index = sample();
    assert_eq!(search(&index, r#""async runtime""#).len(), 1);
    assert!(search(&index, r#""runtime async""#).is_empty());
}

#[test]
fn site_filter() {
    let index = sample();
    let hits = search(&index, "rust site:docs.rs");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].url, "https://docs.rs/tokio");
}

#[test]
fn ext_filter() {
    let index = sample();
    let hits = search(&index, "rust ext:pdf");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].url, "https://example.com/report.pdf");
}

#[test]
fn date_filter_excludes_undated_documents() {
    let index = sample();
    let hits = search(&index, "rust after:2024");
    let urls: Vec<_> = hits.iter().map(|c| c.url.as_str()).collect();
    // doc 1 (2025) and doc 3 (2026) pass; doc 4 (undated) is excluded.
    assert!(urls.contains(&"https://docs.rs/tokio"));
    assert!(urls.contains(&"https://example.com/report.pdf"));
    assert!(!urls.contains(&"https://example.com/undated"));
}

#[test]
fn freshness_window_excludes_old_documents() {
    let index = sample();
    let hits = search(&index, "rust freshness:week");
    // `now` is 2026-02-18; doc 3 is 2026-01-15 (older than a week), doc 1 older still.
    assert!(hits.is_empty());
}

#[test]
fn negation_is_not_an_exclusion() {
    let index = sample();
    // "-zig" must not remove a document that matches the positive term.
    let hits = search(&index, "rust -zig");
    assert!(hits.iter().any(|c| c.url == "https://docs.rs/tokio"));
}

#[test]
fn count_matches_search() {
    let index = sample();
    let plan = parse("rust");
    assert_eq!(index.count(&plan).unwrap(), 3);
}

#[test]
fn filter_only_query_matches_all_passing() {
    let index = sample();
    let plan = parse("site:example.com");
    assert_eq!(index.count(&plan).unwrap(), 2);
}

#[test]
fn document_metadata_round_trips() {
    let index = sample();
    let meta = index.document(id(1)).unwrap().unwrap();
    assert_eq!(meta.title, "Tokio async runtime");
    assert_eq!(meta.ranking_config_version, "v0.1.0");
    assert!(index.document(id(999)).unwrap().is_none());
}

#[test]
fn explain_reports_term_contributions() {
    let index = sample();
    let plan = parse("tokio");
    let explanation = index.explain(&plan, id(1)).unwrap();
    assert!(explanation.bm25 > 0.0);
    assert!(explanation
        .terms
        .iter()
        .any(|t| t.term == "tokio" && t.field == "title"));
}

#[test]
fn generation_reports_doc_count() {
    let index = sample();
    assert_eq!(index.generation().doc_count, 4);
    assert_eq!(index.generation().shards, 1);
}

#[test]
fn title_match_outranks_body_match() {
    let mut index = InMemoryIndex::new();
    index.add(doc(1, "https://a.test/", "tokio guide", "body"));
    index.add(doc(2, "https://b.test/", "guide", "tokio in the body"));
    let hits = search(&index, "tokio");
    assert_eq!(hits[0].url, "https://a.test/");
    assert!(hits[0].field_hits.title > 0);
}

#[test]
fn pagination_is_deterministic() {
    let index = sample();
    let first = search(&index, "rust OR zig");
    let mut paged = SearchOptions {
        limit: 1,
        offset: 0,
        ..opts()
    };
    let front = index.search(&parse("rust OR zig"), &paged).unwrap();
    paged.offset = 1;
    let back = index.search(&parse("rust OR zig"), &paged).unwrap();
    assert_eq!(front[0], first[0]);
    assert_ne!(front[0].doc_id, back[0].doc_id);
}

#[test]
fn empty_query_matches_everything() {
    let index = sample();
    let plan: QueryPlan = parse("");
    assert_eq!(index.count(&plan).unwrap(), 4);
}

#[test]
fn default_clock_still_excludes_undated_documents() {
    // With the default (epoch) clock, dated documents fall inside any recency window but
    // undated ones are still excluded.
    let index = sample();
    let plan = parse("rust freshness:week");
    let hits = index.search(&plan, &SearchOptions::default()).unwrap();
    assert!(hits.iter().any(|c| c.url == "https://docs.rs/tokio"));
    assert!(!hits.iter().any(|c| c.url == "https://example.com/undated"));
}

#[test]
fn collection_stats_report_corpus_size_and_field_lengths() {
    let index = sample();
    let stats = index.collection_stats().unwrap();
    assert_eq!(stats.doc_count, 4);
    assert!(stats.avg.body > 0.0);
    assert!(stats.avg.title > 0.0);
}

#[test]
fn candidates_carry_per_field_frequencies_and_lengths() {
    // `runtime` appears in doc 1's title and body only, so df is 1 and both field hits survive.
    let index = sample();
    let hits = search(&index, "runtime");
    assert_eq!(hits.len(), 1);
    let c = &hits[0];
    let runtime = c
        .terms
        .iter()
        .find(|t| t.term == "runtime")
        .expect("runtime term present");
    assert_eq!(runtime.df, 1);
    assert_eq!(runtime.tf.title, 1);
    assert_eq!(runtime.tf.body, 1);
    // Field lengths are the token counts of each stored field.
    assert_eq!(
        c.field_lengths.title,
        c.title.split_whitespace().count() as u32
    );
}
