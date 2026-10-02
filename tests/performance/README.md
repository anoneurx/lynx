# `tests/performance` — performance budgets

Load tests and benchmarks. These do not run on every push — they run nightly and before a
release, on a dedicated runner, because they are timing-sensitive and produce noise on
shared CI.

## Budgets (targets, not guarantees)

| Scenario | Budget | Conditions |
| --------- | ------ | ---------- |
| Search p50 | < 100 ms | warm cache, single node, 10 M documents |
| Search p95 | < 400 ms | warm cache |
| Search p99 | < 1000 ms | cold index page cache |
| Search throughput | > 200 req/s per replica | cached, 10 M documents |
| Cache hit ratio | > 70 % on a realistic query mix | measured, not assumed |
| Indexer throughput | > 20 documents/s per worker | small documents, local index |
| Crawler throughput | > 10 pages/s per worker | fixture server, 1 KiB responses |
| Index size | < 400 bytes/document compressed | English text corpus |
| API memory | < 512 MiB RSS per replica at steady state | — |
| Index reader RSS | < 2 GiB for a 10 M document index | — |
| Cold start (API with index loaded) | < 20 s | — |

Conditions matter more than the numbers. A budget without its conditions is not a budget.

## Tools

- **k6** — search API load, rate-limit validation, latency percentiles.
- **criterion** — Rust micro-benchmarks for tokenisation, query parsing, ranking.
- **custom harness** — indexer and crawler throughput with a local fixture origin server.

## Rules

1. Never load-test against any third-party host. The crawl-side harness uses the fixture
   server only.
2. Every result is recorded with the index generation, hardware profile, and config
   fingerprint, so a regression can be compared like for like.
3. A budget regression > 20 % fails the release lane and requires either a fix or an
   explicitly documented, time-boxed waiver in [ROADMAP.md](../ROADMAP.md).
4. Performance numbers quoted in documentation must come from a recorded run — never from
   an estimate. See [docs/operations/capacity.md](../docs/operations/capacity.md).