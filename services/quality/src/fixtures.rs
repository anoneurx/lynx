//! The v0 fixture: a small deterministic corpus plus its judged set.
//!
//! This is the golden gate's substrate: twelve topical documents, one keyword-stuffed
//! spam document, and one mirror duplicate, with nine judged queries covering terms,
//! operators, and a phrase. Judgement labels were assigned before running the gate and
//! are updated only through the judge-set release process (`docs/testing/
//! quality-evaluation.md`) — never to make a failing gate pass.

use crate::judged::{parse_jsonl, JudgedQuery};
use lynx_indexer::{InMemoryIndex, IndexedDocument};
use time::macros::datetime;

/// The embedded judged set (v0), under source control with the corpus it targets.
pub const JUDGE_SET_JSONL: &str = include_str!("../fixtures/judge-set-v0.jsonl");

/// The instant every fixture query is evaluated at.
pub const NOW: time::OffsetDateTime = datetime!(2026-04-01 00:00 UTC);

/// Build the fixture corpus. Documents are grouped by topic with distinct vocabulary,
/// plus a spam document that matches the "rust async" vocabulary and a mirrored pair
/// sharing a duplicate cluster, so the hygiene rates have something to catch.
#[must_use]
#[allow(clippy::too_many_lines)] // declarative data: each document block is one record
pub fn corpus() -> Vec<IndexedDocument> {
    let mut docs = Vec::new();

    // ---- Rust / async -------------------------------------------------------------
    let mut d = IndexedDocument::new(
        lynx_indexer::DocId::from_canonical_url("https://tokio.rs/"),
        "https://tokio.rs/",
        "Tokio — the async runtime for Rust",
        "Tokio is an async runtime for Rust: timers, task scheduling, and IO drivers, \
         built to be fast and reliable.",
    );
    d.quality_score = 0.95;
    d.host_rank = 0.9;
    d.published_at = Some(datetime!(2026-01-15 00:00 UTC));
    docs.push(d);

    let mut d = IndexedDocument::new(
        lynx_indexer::DocId::from_canonical_url("https://docs.rs/async-std/"),
        "https://docs.rs/async-std/",
        "async-std documentation",
        "async-std brings async primitives to the Rust standard library style: tasks, \
         streams, and timers without a macro runtime.",
    );
    d.quality_score = 0.85;
    d.host_rank = 0.85;
    d.published_at = Some(datetime!(2025-06-01 00:00 UTC));
    docs.push(d);

    let mut d = IndexedDocument::new(
        lynx_indexer::DocId::from_canonical_url("https://blog.rust-lang.org/async/"),
        "https://blog.rust-lang.org/async/",
        "Why async in Rust is hard",
        "The Rust async story explained: futures, executors, and where the borrow \
         checker makes async Rust different from other languages.",
    );
    d.quality_score = 0.8;
    d.published_at = Some(datetime!(2024-03-01 00:00 UTC));
    docs.push(d);

    // Keyword-stuffed spam: matches the "rust async" vocabulary, must be penalised.
    let mut d = IndexedDocument::new(
        lynx_indexer::DocId::from_canonical_url("https://junk.example/rust-boost/"),
        "https://junk.example/rust-boost/",
        "rust async rust async rust async cheap",
        "rust async rust async cheap backlinks guaranteed growth miracle rust async \
         followers rust async traffic MiracleGrow now",
    );
    d.quality_score = 0.1;
    d.spam_score = 1.0;
    d.published_at = Some(datetime!(2026-02-01 00:00 UTC));
    docs.push(d);

    // ---- Zig ----------------------------------------------------------------------
    let mut d = IndexedDocument::new(
        lynx_indexer::DocId::from_canonical_url("https://ziglang.org/"),
        "https://ziglang.org/",
        "Zig programming language",
        "Zig is a general-purpose programming language with explicit allocator control, \
         no hidden control flow, and a built-in test runner.",
    );
    d.quality_score = 0.9;
    d.host_rank = 0.85;
    docs.push(d);

    let mut d = IndexedDocument::new(
        lynx_indexer::DocId::from_canonical_url("https://zig.news/getting-started/"),
        "https://zig.news/getting-started/",
        "Getting started with Zig",
        "Learn the Zig programming language step by step: the build system, the std \
         library, and how to run your first tests.",
    );
    d.quality_score = 0.75;
    docs.push(d);

    // Mirror duplicate of the getting-started guide: same cluster, junkier host.
    let mut d = IndexedDocument::new(
        lynx_indexer::DocId::from_canonical_url("https://mirror.example/zig-getting-started/"),
        "https://mirror.example/zig-getting-started/",
        "Getting started with Zig",
        "Learn the Zig programming language step by step: the build system, the std \
         library, and how to run your first tests.",
    );
    d.quality_score = 0.3;
    d.cluster_id = 7;
    docs.push(d);

    // ---- CSS ----------------------------------------------------------------------
    let mut d = IndexedDocument::new(
        lynx_indexer::DocId::from_canonical_url("https://developer.mozilla.org/flexbox/"),
        "https://developer.mozilla.org/flexbox/",
        "CSS flexbox: the Flexible Box Layout module",
        "CSS flexbox lays out items in one direction: flex containers, flex wrap, and \
         alignment you can reason about.",
    );
    d.quality_score = 0.95;
    d.host_rank = 0.9;
    docs.push(d);

    let mut d = IndexedDocument::new(
        lynx_indexer::DocId::from_canonical_url("https://css-tricks.com/guide-to-flexbox/"),
        "https://css-tricks.com/guide-to-flexbox/",
        "A complete guide to flexbox",
        "An illustrated guide to CSS flexbox: sizing, alignment, and the gotchas that \
         bite in production layouts.",
    );
    d.quality_score = 0.8;
    docs.push(d);

    // ---- Postgres -----------------------------------------------------------------
    let mut d = IndexedDocument::new(
        lynx_indexer::DocId::from_canonical_url("https://www.postgresql.org/docs/indexes/"),
        "https://www.postgresql.org/docs/indexes/",
        "Postgres index access methods",
        "Postgres supports B-tree, hash, GiST, SP-GiST, GIN, and BRIN index access \
         methods, each suited to different queries.",
    );
    d.quality_score = 0.9;
    d.host_rank = 0.85;
    docs.push(d);

    let mut d = IndexedDocument::new(
        lynx_indexer::DocId::from_canonical_url("https://use-the-index-luke.com/"),
        "https://use-the-index-luke.com/",
        "Use the index, Luke! Postgres tuning",
        "How to make Postgres queries fast with the right index: covering index usage, \
         partial index filters, and the clustering trade-off.",
    );
    d.quality_score = 0.75;
    docs.push(d);

    // ---- Python -------------------------------------------------------------------
    let mut d = IndexedDocument::new(
        lynx_indexer::DocId::from_canonical_url("https://docs.python.org/library/asyncio/"),
        "https://docs.python.org/library/asyncio/",
        "asyncio — asynchronous IO in Python",
        "The asyncio module in Python provides async event loops, coroutines, tasks, \
         and IO primitives in the standard library.",
    );
    d.quality_score = 0.9;
    d.host_rank = 0.85;
    docs.push(d);

    let mut d = IndexedDocument::new(
        lynx_indexer::DocId::from_canonical_url("https://www.python.org/"),
        "https://www.python.org/",
        "Welcome to Python.org",
        "The official home of the Python language: downloads, documentation, and the \
         community, including async tutorials.",
    );
    d.quality_score = 0.85;
    d.host_rank = 0.9;
    docs.push(d);

    // ---- Docker -------------------------------------------------------------------
    let mut d = IndexedDocument::new(
        lynx_indexer::DocId::from_canonical_url("https://docs.docker.com/"),
        "https://docs.docker.com/",
        "Docker documentation: containers and images",
        "Docker builds, ships, and runs containers: write a Dockerfile, layer images, \
         and ship the same container everywhere.",
    );
    d.quality_score = 0.9;
    d.host_rank = 0.85;
    docs.push(d);

    // ---- Git ----------------------------------------------------------------------
    let mut d = IndexedDocument::new(
        lynx_indexer::DocId::from_canonical_url("https://git-scm.com/book/"),
        "https://git-scm.com/book/",
        "Pro Git: everything about Git basics",
        "The Pro Git book covers Git basics: commits, branches, merges, rebasing, and \
         working with remotes.",
    );
    d.quality_score = 0.9;
    d.host_rank = 0.85;
    docs.push(d);

    docs
}

/// An index seeded with the fixture corpus.
#[must_use]
pub fn index() -> InMemoryIndex {
    let mut idx = InMemoryIndex::new();
    for doc in corpus() {
        idx.add(doc);
    }
    idx
}

/// The parsed v0 judged set.
///
/// # Errors
/// Propagates `judged::parse_jsonl` failures; the fixture embedded in the binary must
/// always parse.
pub fn judged_queries() -> Result<Vec<JudgedQuery>, crate::judged::JudgedSetError> {
    parse_jsonl(JUDGE_SET_JSONL)
}
