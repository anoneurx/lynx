# Unit Testing

## Conventions

- Tests live in `#[cfg(test)]` modules next to code.
- Name: `mod tests { ... }` or `#[test] fn test_<function>_<scenario>()`.
- Use `pretty_assertions` for diffs.
- No I/O, no network, no time dependence.

## Property-based testing (proptest)

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn bm25_score_never_nan(tf in 0u32..1_000_000, df in 1u32..1_000_000, n in 1u32..10_000_000) {
        let score = bm25(tf, df, n, 100.0, 1.2, 0.75);
        prop_assert!(score.is_finite());
        prop_assert!(score >= 0.0);
    }
}
```

Run: `cargo test --proptest-threads=4`.

## Snapshot testing (insta)

```rust
#[test]
fn search_response_shape() {
    let resp = search(Query { q: "rust".into(), ..Default::default() }).await;
    insta::assert_json_snapshot!(resp, @r###
    {
      "results": [...],
      "total": 42,
      "took_ms": 12
    }
    ###);
}
```

Update: `cargo insta test --review`.

## Mocking

- **No mockall** – use real implementations with test doubles via traits.
- Database: `sqlx::test` macros spin up temp Postgres (transaction rollback).
- Time: `time::OffsetDateTime::now_utc()` wrapped in `Clock` trait.

## Test organization

```
src/
  search/
    ranking/
      bm25.rs
      bm25_tests.rs      ← unit tests for bm25.rs
    query/
      parser.rs
      parser_tests.rs
    mod.rs
    tests/               ← integration-style but fast
      ranking_integration.rs
```

## Benchmarks as tests

```rust
#[bench]
fn bench_bm25_scoring(b: &mut Bencher) {
    let docs = generate_corpus(1_000_000);
    b.iter(|| bm25_batch(&docs, &query));
}
```

Run: `cargo bench --bench ranking`. CI fails if median > 1.1× baseline.

## Common patterns

### Async tests

```rust
#[tokio::test]
async fn search_returns_results() {
    let idx = test_index().await;
    let results = idx.search("test").await.unwrap();
    assert!(!results.is_empty());
}
```

### Temp directories

```rust
#[test]
fn index_persists_segments() {
    let dir = tempfile::tempdir().unwrap();
    let idx = Index::create_in(dir.path()).unwrap();
    idx.add_document(doc!("title" => "test")).unwrap();
    idx.commit().unwrap();
    drop(idx);
    // Reopen
    let idx = Index::open(dir.path()).unwrap();
    assert_eq!(idx.num_docs(), 1);
}
```

### Feature-gated tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "simd")]
    fn simd_bm25_matches_scalar() { ... }
}
```

## Linting in tests

```toml
# Cargo.toml
[profile.test]
debug = true
opt-level = 0
overflow-checks = true

[profile.release]
debug = false
opt-level = 3
```

Clippy runs on test code too: `cargo clippy --tests`.

## Coverage

```bash
cargo llvm-cov --workspace --all-targets --lcov --output-path lcov.info
genhtml lcov.info -o coverage-report
```

Enforce in CI: `cargo llvm-cov --fail-under-lines 90 --fail-under-branches 80`.