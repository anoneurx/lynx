//! Tests for the engine-agnostic generation builder.
//!
//! These run without the `tantivy` feature: they exercise mapping, normalisation,
//! idempotency, rejection, and generation sealing purely on the value types.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lynx_common::hash::ContentHash;
use lynx_common::ids::DocumentId;
use lynx_common::time::FixedClock;
use lynx_indexer::{AddOutcome, GenerationBuilder};
use lynx_models::domain::ParsedDocument;
use time::macros::datetime;

fn parsed(url: &str, title: &str, body: &str) -> ParsedDocument {
    ParsedDocument {
        canonical: url.to_string(),
        title: title.to_string(),
        headings: Vec::new(),
        body: body.to_string(),
        anchors: Vec::new(),
        links: Vec::new(),
        lang: Some("EN".to_string()),
        content_hash: format!("h-{url}"),
    }
}

#[test]
fn maps_parsed_document_to_indexed_document() {
    let mut b = GenerationBuilder::new(datetime!(2026-01-01 00:00 UTC));
    let mut p = parsed("https://docs.rs/tokio", "Tokio", "Async runtime");
    p.headings = vec!["Intro".into(), "Usage".into()];
    p.anchors = vec!["tokio docs".into()];
    p.links = vec!["https://a".into(), "https://b".into()];

    assert_eq!(b.add(p), AddOutcome::Indexed);
    let (docs, gen) = b.build(1);

    assert_eq!(docs.len(), 1);
    let d = &docs[0];
    assert_eq!(
        d.doc_id,
        DocumentId::from_canonical_url("https://docs.rs/tokio")
    );
    assert_eq!(d.host, "docs.rs");
    assert_eq!(d.title, "Tokio");
    assert_eq!(d.headings, "Intro Usage");
    assert_eq!(d.anchors, "tokio docs");
    assert_eq!(d.lang.as_deref(), Some("en"));
    assert_eq!(d.link_count, 2);
    assert_eq!(gen.doc_count, 1);
}

#[test]
fn rejects_documents_with_no_indexable_text() {
    let mut b = GenerationBuilder::new(datetime!(2026-01-01 00:00 UTC));
    assert_eq!(
        b.add(parsed("https://e.com/x", "", "")),
        AddOutcome::Rejected("empty_text")
    );
    assert!(b.is_empty());
    let (docs, gen) = b.build(1);
    assert!(docs.is_empty());
    assert_eq!(gen.doc_count, 0);
}

#[test]
fn unchanged_content_is_a_no_op() {
    let mut b = GenerationBuilder::new(datetime!(2026-01-01 00:00 UTC));
    let p = parsed("https://e.com/x", "T", "B");
    assert_eq!(b.add(p.clone()), AddOutcome::Indexed);
    assert_eq!(b.add(p), AddOutcome::Unchanged);
    assert_eq!(b.len(), 1);
}

#[test]
fn changed_content_replaces_in_place() {
    let mut b = GenerationBuilder::new(datetime!(2026-01-01 00:00 UTC));
    b.add(parsed("https://e.com/x", "Old", "old body"));
    let mut updated = parsed("https://e.com/x", "New", "new body");
    updated.content_hash = "h-2".to_string();
    assert_eq!(b.add(updated), AddOutcome::Indexed);

    assert_eq!(b.len(), 1);
    let (docs, _) = b.build(1);
    assert_eq!(docs[0].title, "New");
}

#[test]
fn rejecting_a_new_version_removes_the_stale_one() {
    let mut b = GenerationBuilder::new(datetime!(2026-01-01 00:00 UTC));
    b.add(parsed("https://e.com/x", "T", "B"));
    let mut emptied = parsed("https://e.com/x", "", "");
    emptied.content_hash = "h-2".to_string();
    assert_eq!(b.add(emptied), AddOutcome::Rejected("empty_text"));
    assert!(b.is_empty());
}

#[test]
fn checksum_is_order_independent_and_content_sensitive() {
    let a = parsed("https://e.com/a", "A", "alpha");
    let b = parsed("https://e.com/b", "B", "beta");

    let mut b1 = GenerationBuilder::new(datetime!(2026-01-01 00:00 UTC));
    b1.add(a.clone());
    b1.add(b.clone());
    let (_, gen1) = b1.build(1);

    let mut b2 = GenerationBuilder::new(datetime!(2026-01-01 00:00 UTC));
    b2.add(b);
    b2.add(a);
    let (_, gen2) = b2.build(2);

    assert_eq!(gen1.checksum, gen2.checksum);

    let mut b3 = GenerationBuilder::new(datetime!(2026-01-01 00:00 UTC));
    let mut changed = parsed("https://e.com/a", "A", "changed content");
    changed.content_hash = "h-changed".to_string();
    b3.add(changed);
    b3.add(parsed("https://e.com/b", "B", "beta"));
    let (_, gen3) = b3.build(3);
    assert_ne!(gen1.checksum, gen3.checksum);
}

#[test]
fn checksum_matches_blake3_of_sorted_pairs() {
    let mut b = GenerationBuilder::new(datetime!(2026-01-01 00:00 UTC));
    b.add(parsed("https://e.com/a", "A", "alpha"));
    let (_, gen) = b.build(1);

    let id = DocumentId::from_canonical_url("https://e.com/a").to_string();
    let expected = ContentHash::of(format!("{id}\u{1f}h-https://e.com/a\n").as_bytes());
    assert_eq!(gen.checksum, expected.as_str());
}

#[test]
fn text_is_normalised_for_display() {
    let mut b = GenerationBuilder::new(datetime!(2026-01-01 00:00 UTC));
    let mut p = parsed(
        "https://e.com/x",
        "ＦＵＬＬ\u{200B}width",
        "  spaced   out  ",
    );
    p.content_hash = "h".to_string();
    b.add(p);
    let (docs, _) = b.build(1);
    assert_eq!(docs[0].title, "FULLwidth");
    assert_eq!(docs[0].body, "spaced out");
}

#[test]
fn generation_records_versions_and_shard_floor() {
    let b = GenerationBuilder::new(datetime!(2026-01-01 00:00 UTC)).with_shards(0);
    let (_, gen) = b.build(42);
    assert_eq!(gen.id, 42);
    assert_eq!(gen.shards, 1);
    assert_eq!(gen.tokenizer_version, lynx_indexer::TOKENIZER_VERSION);
    assert_eq!(
        gen.ranking_config_version,
        lynx_indexer::RANKING_CONFIG_VERSION
    );
}

#[test]
fn now_uses_the_injected_clock() {
    use time::Duration;
    let clock = FixedClock::at(datetime!(2026-02-18 09:00 UTC));
    clock.advance(Duration::hours(1));
    let b = GenerationBuilder::now(&clock);
    let (_, gen) = b.build(1);
    assert_eq!(gen.created_at, datetime!(2026-02-18 10:00 UTC));
}

#[test]
fn empty_text_with_only_invisible_characters_is_rejected() {
    let mut b = GenerationBuilder::new(datetime!(2026-01-01 00:00 UTC));
    assert_eq!(
        b.add(parsed("https://e.com/x", "\u{200B}\u{FEFF}", "\n\t  ")),
        AddOutcome::Rejected("empty_text")
    );
}
