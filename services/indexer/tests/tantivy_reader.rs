//! Trait-level tests for the Tantivy-backed reader.
//!
//! These mirror `tests/reader.rs` but run only with `--features tantivy`. They assert the
//! same contract, proving the in-memory fake and the real engine are interchangeable at
//! the `SearchIndexReader` boundary.

#![cfg(feature = "tantivy")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation
)]

use lynx_common::ids::DocumentId;
use lynx_indexer::{
    AddOutcome, GenerationBuilder, IndexGeneration, IndexedDocument, SearchIndexReader,
    SearchOptions, TantivyIndexReader,
};
use lynx_models::domain::ParsedDocument;
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

fn generation() -> IndexGeneration {
    IndexGeneration {
        id: 1,
        tokenizer_version: "lynx-test".to_string(),
        ranking_config_version: "v0.1.0".to_string(),
        created_at: datetime!(2026-01-01 00:00 UTC),
        doc_count: 0,
        size_bytes: 0,
        checksum: String::new(),
        shards: 1,
    }
}

fn sample() -> TantivyIndexReader {
    let mut docs = Vec::new();

    let mut rust = doc(
        1,
        "https://docs.rs/tokio",
        "Tokio async runtime",
        "An asynchronous runtime for Rust.",
    );
    rust.published_at = Some(datetime!(2025-06-01 00:00 UTC));
    rust.quality_score = 0.9;
    docs.push(rust);

    let mut zig = doc(
        2,
        "https://ziglang.org/learn",
        "Learning Zig",
        "Zig is a general-purpose language.",
    );
    zig.published_at = Some(datetime!(2023-01-01 00:00 UTC));
    docs.push(zig);

    let mut pdf = doc(
        3,
        "https://example.com/report.pdf",
        "Annual report",
        "Rust adoption in industry.",
    );
    pdf.content_type = Some("application/pdf".to_string());
    pdf.published_at = Some(datetime!(2026-01-15 00:00 UTC));
    docs.push(pdf);

    let undated = doc(
        4,
        "https://example.com/undated",
        "Undated page",
        "Rust and Zig together.",
    );
    docs.push(undated);

    TantivyIndexReader::from_documents(docs, generation()).unwrap()
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
fn generation_reports_doc_count() {
    let index = sample();
    assert_eq!(index.generation().doc_count, 4);
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
    let urls: Vec<_> = search(&index, "tokio OR zig")
        .iter()
        .map(|c| c.url.clone())
        .collect();
    assert!(urls.contains(&"https://docs.rs/tokio".to_string()));
    assert!(urls.contains(&"https://ziglang.org/learn".to_string()));
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
    let urls: Vec<_> = search(&index, "rust after:2024")
        .iter()
        .map(|c| c.url.clone())
        .collect();
    assert!(urls.contains(&"https://docs.rs/tokio".to_string()));
    assert!(urls.contains(&"https://example.com/report.pdf".to_string()));
    assert!(!urls.contains(&"https://example.com/undated".to_string()));
}

#[test]
fn negation_is_not_an_exclusion() {
    let index = sample();
    let hits = search(&index, "rust -zig");
    assert!(hits.iter().any(|c| c.url == "https://docs.rs/tokio"));
}

#[test]
fn negation_carries_the_term_for_the_ranker() {
    let docs = vec![
        doc(1, "https://a.test/", "rust", "a systems language"),
        doc(
            2,
            "https://b.test/",
            "rust",
            "a systems language that also mentions golang",
        ),
    ];
    let index = TantivyIndexReader::from_documents(docs, generation()).unwrap();
    let hits = search(&index, "rust -golang");
    let b = hits
        .iter()
        .find(|c| c.url == "https://b.test/")
        .expect("negated document still returned");
    assert!(b.terms.iter().any(|t| t.term == "golang"));
}

#[test]
fn intitle_requires_the_term_in_the_title() {
    let docs = vec![
        doc(1, "https://a.test/", "tokio guide", "many things"),
        doc(2, "https://b.test/", "unrelated", "tokio in the body"),
    ];
    let index = TantivyIndexReader::from_documents(docs, generation()).unwrap();
    let hits = search(&index, "tokio intitle:guide");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].url, "https://a.test/");
}

#[test]
fn count_matches_search() {
    let index = sample();
    let plan: QueryPlan = parse("rust");
    assert_eq!(index.count(&plan, opts().now).unwrap(), 3);
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
fn empty_query_matches_everything() {
    let index = sample();
    let plan: QueryPlan = parse("");
    assert_eq!(index.count(&plan, opts().now).unwrap(), 4);
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
    assert_eq!(
        c.field_lengths.title,
        c.title.split_whitespace().count() as u32
    );
}

fn parsed(url: &str, title: &str, body: &str) -> ParsedDocument {
    ParsedDocument {
        canonical: url.to_string(),
        title: title.to_string(),
        headings: Vec::new(),
        body: body.to_string(),
        anchors: Vec::new(),
        links: Vec::new(),
        lang: Some("en".to_string()),
        content_hash: format!("h-{url}"),
        ..ParsedDocument::default()
    }
}

#[test]
fn generation_round_trips_through_disk() {
    let dir = tempfile::tempdir().unwrap();
    let mut builder = GenerationBuilder::new(datetime!(2026-03-01 00:00 UTC));
    assert_eq!(
        builder.add(parsed(
            "https://docs.rs/tokio",
            "Tokio async runtime",
            "An asynchronous runtime for Rust.",
        )),
        AddOutcome::Indexed
    );
    builder.add(parsed(
        "https://ziglang.org/learn",
        "Learning Zig",
        "Zig is a general-purpose language.",
    ));

    let written = builder.write_to_dir(dir.path(), 7).unwrap();
    assert_eq!(written.id, 7);
    assert_eq!(written.doc_count, 2);
    assert!(!written.checksum.is_empty());

    let index = TantivyIndexReader::open(dir.path()).unwrap();
    assert_eq!(index.generation().id, 7);
    assert_eq!(index.generation().doc_count, 2);
    assert_eq!(search(&index, "rust async").len(), 1);
    assert_eq!(
        index
            .count(&parse("zig"), datetime!(2026-02-18 00:00 UTC))
            .unwrap(),
        1
    );
}

#[test]
fn reopening_preserves_generation_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let mut builder =
        GenerationBuilder::new(datetime!(2026-03-01 00:00 UTC)).with_ranking_config_version("v9");
    builder.add(parsed("https://a.example/x", "Title", "Body text"));

    let written = builder.write_to_dir(dir.path(), 11).unwrap();
    let reopened = TantivyIndexReader::open(dir.path()).unwrap();
    assert_eq!(reopened.generation(), written);
}

#[test]
fn open_missing_generation_is_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    assert!(TantivyIndexReader::open(dir.path()).is_err());
}
