//! Backend parity: the in-memory reference and the Tantivy reader must agree.
//!
//! The same fixture corpus is loaded into both [`InMemoryIndex`] and
//! [`TantivyIndexReader::from_documents`], and every plan in the matrix must produce the
//! same matching documents, the same `count()`, and deterministic ordering. This is the
//! fence between the two readers: any semantic drift — boolean semantics, tokenisation,
//! phrase matching, filter pushdown — fails here, not in production.
//!
//! `Candidate::bm25` intentionally differs between the engines (weighted term frequency
//! vs. Tantivy BM25): retrieval order is not contract, so parity is asserted over
//! ordered-by-URL sets, plus internal determinism per backend.

#![cfg(feature = "tantivy")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation
)]

use std::collections::BTreeSet;

use lynx_common::ids::DocumentId;
use lynx_indexer::{
    InMemoryIndex, IndexGeneration, IndexedDocument, SearchIndexReader, SearchOptions,
    TantivyIndexReader,
};
use lynx_query::parse;
use time::macros::datetime;

/// The instant every parity plan is evaluated at.
const NOW: time::OffsetDateTime = datetime!(2026-02-18 00:00 UTC);

fn id(n: u128) -> DocumentId {
    DocumentId::from_canonical_url(&format!("lynx://parity/{n}"))
}

/// The fixture corpus. Identical documents go into both backends; `doc(n)` builds the
/// n-th one so both sides get the same `doc_id` values.
fn fixture() -> Vec<IndexedDocument> {
    let mut docs = Vec::new();

    // 1: the canonical rust doc, dated, phrase source.
    let mut rust = IndexedDocument::new(
        id(1),
        "https://docs.rs/tokio",
        "Tokio async runtime",
        "An asynchronous runtime for Rust.",
    );
    rust.lang = Some("en".to_string());
    rust.content_type = Some("text/html".to_string());
    rust.published_at = Some(datetime!(2025-06-01 00:00 UTC));
    docs.push(rust);

    // 2: a SUBDOMAIN of example.com — `site:example.com` must match it (suffix logic),
    //    not only the bare host.
    let mut www = IndexedDocument::new(
        id(2),
        "https://www.example.com/guide",
        "Rust guide",
        "A guide about rust.",
    );
    www.lang = Some("en".to_string());
    docs.push(www);

    // 3: a PDF, dated recently.
    let mut pdf = IndexedDocument::new(
        id(3),
        "https://example.com/report.pdf",
        "Annual report",
        "Rust adoption in industry.",
    );
    pdf.lang = Some("en".to_string());
    pdf.content_type = Some("application/pdf".to_string());
    pdf.published_at = Some(datetime!(2026-01-15 00:00 UTC));
    docs.push(pdf);

    // 4: undated — date filters must exclude it in both backends.
    let mut undated = IndexedDocument::new(
        id(4),
        "https://example.com/undated",
        "Undated page",
        "Rust and Zig together.",
    );
    undated.lang = Some("en".to_string());
    docs.push(undated);

    // 5: the zig alternative.
    let mut zig = IndexedDocument::new(
        id(5),
        "https://ziglang.org/learn",
        "Learning Zig",
        "Zig is a general-purpose language.",
    );
    zig.lang = Some("en".to_string());
    zig.published_at = Some(datetime!(2023-01-01 00:00 UTC));
    docs.push(zig);

    // 6: anchor text — the only document with anchors.
    let mut anchored = IndexedDocument::new(
        id(6),
        "https://anchor.test/page",
        "Linked page",
        "Plenty of unrelated words here.",
    );
    anchored.lang = Some("en".to_string());
    anchored.anchors = "rust guide and more".to_string();
    docs.push(anchored);

    // 7: phrase spans a field boundary — title ends with `async`, body starts with
    //    `runtime`; `"async runtime"` must NOT match.
    let mut spanning =
        IndexedDocument::new(id(7), "https://span.test/", "async one", "runtime two");
    spanning.lang = Some("en".to_string());
    docs.push(spanning);

    // 8: punctuation, casing, and NFKC folding: `Ｃ++`, `async-runtime`, `Ｒｕｓｔ`.
    let mut messy = IndexedDocument::new(
        id(8),
        "https://cpp.test/",
        "Ｃ++ guide",
        "The async-runtime for Ｒｕｓｔ fans.",
    );
    messy.lang = Some("en".to_string());
    docs.push(messy);

    // 9: Go, in German — the only non-English document, and the negation-carrier.
    let mut go = IndexedDocument::new(
        id(9),
        "https://go.dev/",
        "Go programming",
        "Golang for Ｒｕｓｔ refugees.",
    );
    go.lang = Some("de".to_string());
    docs.push(go);

    docs
}

fn both() -> (InMemoryIndex, TantivyIndexReader) {
    let docs = fixture();
    let mut memory = InMemoryIndex::new();
    for doc in &docs {
        memory.add(doc.clone());
    }
    let tantivy = TantivyIndexReader::from_documents(
        docs,
        IndexGeneration {
            id: 1,
            tokenizer_version: "lynx-test".to_string(),
            ranking_config_version: "v0.1.0".to_string(),
            created_at: datetime!(2026-01-01 00:00 UTC),
            doc_count: 0,
            size_bytes: 0,
            checksum: String::new(),
            shards: 1,
        },
    )
    .unwrap();
    (memory, tantivy)
}

fn opts() -> SearchOptions {
    SearchOptions {
        limit: 100,
        now: NOW,
        ..SearchOptions::default()
    }
}

/// Search both backends and compare: the same matching URLs (in sorted order), and the
/// same `count()`.
fn assert_match(q: &str, memory: &dyn SearchIndexReader, tantivy: &dyn SearchIndexReader) {
    let plan = parse(q);
    let title = format!("plan {q:?}");

    let a = memory.search(&plan, &opts()).unwrap();
    let b = tantivy.search(&plan, &opts()).unwrap();
    let a_set: BTreeSet<String> = a.iter().map(|c| c.url.clone()).collect();
    let b_set: BTreeSet<String> = b.iter().map(|c| c.url.clone()).collect();
    assert_eq!(a_set, b_set, "{title}: result sets diverge");

    let a_count = memory.count(&plan, NOW).unwrap();
    let b_count = tantivy.count(&plan, NOW).unwrap();
    assert_eq!(a_count, b_count, "{title}: counts diverge");
}

#[test]
fn conjunction_matches_identically() {
    let (memory, tantivy) = both();
    assert_match("rust", &memory, &tantivy);
    assert_match("rust async", &memory, &tantivy);
}

#[test]
fn disjunction_matches_identically() {
    let (memory, tantivy) = both();
    assert_match("tokio OR zig", &memory, &tantivy);
    assert_match("rust zig OR go", &memory, &tantivy);
    assert_match("nothing OR language", &memory, &tantivy);
}

#[test]
fn phrases_match_identically() {
    let (memory, tantivy) = both();
    // Present in two documents, in different fields.
    assert_match("\"async runtime\"", &memory, &tantivy);
    // Punctuation and case fold away to the same token run.
    assert_match("\"Async-Runtime!\"", &memory, &tantivy);
    // Absent.
    assert_match("\"runtime for async\"", &memory, &tantivy);
    // `async` ends the title and `runtime` opens the body: no field-local run.
    let plan = parse("\"async runtime\"");
    for index in [
        &memory as &dyn SearchIndexReader,
        &tantivy as &dyn SearchIndexReader,
    ] {
        let urls: Vec<String> = index
            .search(&plan, &opts())
            .unwrap()
            .into_iter()
            .map(|c| c.url)
            .collect();
        assert!(
            !urls.iter().any(|u| u.starts_with("https://span.test/")),
            "{urls:?}"
        );
    }
}

#[test]
fn intitle_matches_identically() {
    let (memory, tantivy) = both();
    assert_match("tokio intitle:runtime", &memory, &tantivy);
    assert_match("intitle:guide", &memory, &tantivy);
}

#[test]
fn inanchor_matches_identically() {
    let (memory, tantivy) = both();
    // `guide` appears in doc 2's title/body too, but not in any anchors text; only
    // document 6 carries anchors at all.
    assert_match("rust inanchor:guide", &memory, &tantivy);
    assert_match("inanchor:linked", &memory, &tantivy);
}

#[test]
fn site_matches_identically_including_subdomains() {
    let (memory, tantivy) = both();
    // Bare host.
    assert_match("rust site:docs.rs", &memory, &tantivy);
    // Suffix semantics: `site:example.com` matches bare AND `www.` subdomain hits.
    assert_match("rust site:example.com", &memory, &tantivy);
}

#[test]
fn inurl_matches_identically() {
    let (memory, tantivy) = both();
    assert_match("runtime inurl:tokio", &memory, &tantivy);
    assert_match("rust inurl:report", &memory, &tantivy);
}

#[test]
fn filetype_and_ext_match_identically() {
    let (memory, tantivy) = both();
    assert_match("report filetype:pdf", &memory, &tantivy);
    assert_match("report ext:pdf", &memory, &tantivy);
    assert_match("guide ext:pdf", &memory, &tantivy);
}

#[test]
fn lang_matches_identically() {
    let (memory, tantivy) = both();
    // Includes the German document when the filter asks for it, case-insensitively.
    assert_match("rust lang:EN", &memory, &tantivy);
    assert_match("rust lang:de", &memory, &tantivy);
}

#[test]
fn date_filters_match_identically() {
    let (memory, tantivy) = both();
    assert_match("rust before:2026-01-01", &memory, &tantivy);
    assert_match("rust after:2025-01-01", &memory, &tantivy);
    assert_match("rust after:2025-01-01 before:2026-01-01", &memory, &tantivy);
    // Recency window: document 3 (2026-01-15) is older than a week; the undated
    // document never satisfies a window.
    assert_match("rust freshness:week", &memory, &tantivy);
}

#[test]
fn negation_penalises_but_does_not_exclude_identically() {
    let (memory, tantivy) = both();
    // Document 9 matches `rust` (via Ｒｕｓｔ) and contains `golang`: it stays in the
    // result set in both backends, so the ranker can downrank it.
    assert_match("rust -golang", &memory, &tantivy);
    // The negated token is exposed on the candidate so the ranker can see it.
    let plan = parse("rust -golang");
    for index in [
        &memory as &dyn SearchIndexReader,
        &tantivy as &dyn SearchIndexReader,
    ] {
        let mut has_negated = false;
        for candidate in index.search(&plan, &opts()).unwrap() {
            if candidate.terms.iter().any(|t| t.term == "golang") {
                has_negated = true;
            }
        }
        assert!(has_negated, "a backend dropped the negated term token");
    }
}

#[test]
fn empty_query_matches_everything_identically() {
    let (memory, tantivy) = both();
    assert_match("", &memory, &tantivy);
}

#[test]
fn tokenization_parity_holds_for_nfkc_folded_text() {
    let (memory, tantivy) = both();
    // `Ｒｕｓｔ` folds to `rust` only under the LYNX tokenizer; if the Tantivy side
    // indexed with its default tokenizer the sets would diverge here.
    assert_match("rust", &memory, &tantivy);
    assert_match("c", &memory, &tantivy);
    assert_match("async-runtime", &memory, &tantivy);
    // Candidate term keys are the folded tokens in both backends.
    let plan = parse("c");
    for index in [
        &memory as &dyn SearchIndexReader,
        &tantivy as &dyn SearchIndexReader,
    ] {
        for candidate in index.search(&plan, &opts()).unwrap() {
            assert!(
                candidate.terms.iter().any(|t| t.term == "c"),
                "{} did not expose folded token `c`",
                index.generation().id
            );
        }
    }
}

#[test]
fn ranking_counts_agree_with_search_window_sizes() {
    let (memory, tantivy) = both();
    let plan = parse("rust");
    let a = memory.search(&plan, &opts()).unwrap();
    let b = tantivy.search(&plan, &opts()).unwrap();
    assert_eq!(a.len(), b.len());
    assert_eq!(
        memory.count(&plan, NOW).unwrap(),
        tantivy.count(&plan, NOW).unwrap()
    );
}

#[test]
fn ordering_is_deterministic_within_each_backend() {
    let (memory, tantivy) = both();
    let plan = parse("rust");
    for index in [
        &memory as &dyn SearchIndexReader,
        &tantivy as &dyn SearchIndexReader,
    ] {
        let first: Vec<String> = index
            .search(&plan, &opts())
            .unwrap()
            .into_iter()
            .map(|c| c.url)
            .collect();
        let second: Vec<String> = index
            .search(&plan, &opts())
            .unwrap()
            .into_iter()
            .map(|c| c.url)
            .collect();
        assert_eq!(first, second);
    }
    // And pagination is stable: skip(1)+take(1) continues where the first page ended.
    let mut pages = SearchOptions {
        limit: 1,
        now: NOW,
        ..SearchOptions::default()
    };
    pages.offset = 0;
    let front = memory.search(&plan, &pages).unwrap();
    pages.offset = 1;
    let back = memory.search(&plan, &pages).unwrap();
    assert_ne!(front[0].url, back[0].url);
}
