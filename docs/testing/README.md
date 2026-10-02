# Testing

LYNX test strategy: fast unit tests, realistic integration tests, contract tests for API,
and continuous quality evaluation for search relevance.

| Document | Contents |
| -------- | -------- |
| [README.md](./README.md) | This overview |
| [unit.md](./unit.md) | Cargo `#[test]`, property-based, snapshot |
| [integration.md](./integration.md) | Testcontainers, API contracts, crawler flows |
| [e2e.md](./e2e.md) | Full-stack scenarios, chaos, performance |
| [fuzzing.md](./fuzzing.md) | cargo-fuzz, AFL++, OSS-Fuzz integration |
| [quality-evaluation.md](./quality-evaluation.md) | NDCG, judge sets, A/B, regression detection |

## Test pyramid

```
           ┌─────────────┐
           │   E2E (5%)  │  ← 10 scenarios, nightly
           ├─────────────┤
           │ Integration │  ← 50 suites, PR gate
           ├─────────────┤
           │  Unit (90%) │  ← ~3,000 tests, < 3 min
           └─────────────┘
```

## Running tests

```bash
# Unit + integration (PR pipeline)
cargo nextest run --workspace --all-targets

# E2E (nightly, needs Docker)
cargo nextest run --workspace --test e2e -- --ignored

# Fuzzing (CI weekly)
cargo fuzz run parse_query -- -max_total_time=300

# Quality evaluation (weekly + on index change)
python -m lynx.eval.ndcg --judge-set=judge-sets/v2026-q3.jsonl
```

## Coverage targets

| Layer | Target | Tool |
| ----- | ------ | ---- |
| Unit | ≥ 90 % line, ≥ 80 % branch | `cargo llvm-cov --lcov` |
| Integration | All public API endpoints | `cargo nextest run --test integration` |
| Fuzzing | All public parsers/decoders | `cargo fuzz` corpus growth |

## Flaky test policy

- Quarantine with `#[ignore = "flaky: #1234"]` + GitHub issue.
- Auto-fail if flaky test passes 3× in a row (nextest `--retries=3`).
- Root-cause within 2 weeks or delete.

## Test data

- **Unit:** Inline fixtures, `proptest` strategies.
- **Integration:** `testcontainers` (Postgres, Redis, MinIO, Tantivy index fixture).
- **E2E:** Curated 10 k-doc corpus (`test-data/corpus-10k/`), judge sets in `eval/judge-sets/`.
- **Fuzzing:** Corpus in `fuzz/corpus/<target>/`, seeded from real traffic (anonymized).

## CI gates (GitHub Actions)

| Job | Trigger | Timeout | Required |
| --- | ------- | ------- | -------- |
| `unit` | PR, push | 10 min | ✅ |
| `integration` | PR, push | 20 min | ✅ |
| `e2e` | nightly, main | 60 min | ❌ (informational) |
| `fuzzing` | weekly, main | 4 h | ❌ (corpus growth) |
| `quality-eval` | weekly, index change | 30 min | ⚠️ (NDCG regression > 1 %) |

## Local development

```bash
# Watch mode
cargo nextest run --workspace --watch

# Single test with debug
cargo test --lib search::ranking::bm25 -- --nocapture

# Integration with logs
cargo nextest run --test integration -- --test-threads=1 2>&1 | less
```

## Tools

| Tool | Purpose |
| ---- | ------- |
| `cargo-nextest` | Fast test runner, retries, JUnit XML |
| `proptest` | Property-based testing |
| `insta` | Snapshot testing (serialized responses) |
| `testcontainers` | Real Postgres/Redis/MinIO in integration tests |
| `cargo-fuzz` | Coverage-guided fuzzing (libFuzzer) |
| `criterion` | Microbenchmarks (CI tracks regressions) |
| `lynx-eval` | Custom NDCG/MRR evaluation CLI |