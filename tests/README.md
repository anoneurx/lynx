# `tests/` — cross-cutting test suites

Unit tests live next to the code they test (Rust convention, so they can use private
internals). This directory is for suites that **span** crates or processes, or that need
their own harness.

| Directory | Kind | What runs | Needs |
| --------- | ---- | --------- | ----- |
| [`unit/`](./unit/) | cross-crate unit | golden-file tests for URL normalisation, tokenisation, query parsing, ranking maths | nothing |
| [`integration/`](./integration/) | in-process multi-crate | parse→index→search, API→index, frontier→crawler, config validation | Postgres (testcontainer) |
| [`e2e/`](./e2e/) | black box | search request → rendered result page in a real browser | full local stack |
| [`crawler/`](./crawler/) | protocol | robots, redirects, content negotiation, traps, slow responses, malformed HTML — against a local fixture origin server | local HTTP fixture server |
| [`ranking/`](./ranking/) | quality & regression | golden ranking snapshots, quality metric regression gates | fixtures |
| [`security/`](./security/) | adversarial | SSRF matrix, header/CSP assertions, injection, oversized responses, secret scanning | local stack |
| [`performance/`](./performance/) | load & bench | k6 search load, indexer throughput, crawler throughput, index size per doc | full stack, dedicated CI runner |

## Rules

1. **No test touches the public internet.** Everything runs against a local fixture origin
   server with scripted responses, including "malicious site" scenarios. This keeps CI
   hermetic, fast, and non-abusive to third parties.
2. **Security tests are not optional.** A change to `packages/security`,
   `services/crawler/safety.rs`, or the log/redaction path must not merge with a failing
   security suite.
3. **Regression gates, not just pass/fail.** Ranking and quality suites fail on metric
   *regression* (NDCG@10 drop > 5%, duplicate rate increase, p95 latency increase > 20%)
   even when every test passes. See
   [docs/testing/quality-evaluation.md](../docs/testing/quality-evaluation.md).
4. **Determinism.** Clock and randomness are injected everywhere they matter. A ranking
   test that depends on wall-clock time is a flaky test.
5. **Golden files are reviewed.** A ranking golden diff is a product decision, not a
   mechanical update. `make ranking-accept` is a deliberate act with a diff review.

## CI lanes

```text
fast        fmt + clippy + unit            every push          ~3 min
test        integration + e2e + security   every push         ~12 min
quality     ranking + quality metrics      every PR to main    ~15 min
perf        k6 + indexer bench             nightly + release   ~40 min
fuzz        cargo-fuzz targets (short)     nightly            ~30 min
supply      cargo-audit/deny + trivy       every push          ~5 min
```

Full test strategy: [docs/testing/strategy.md](../docs/testing/strategy.md).