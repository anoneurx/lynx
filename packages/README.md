# `packages/` — shared internal libraries

Workspace crates that are libraries only. They contain no binaries, no deployment, and no
knowledge of which application consumes them.

| Package | Purpose | May depend on |
| ------- | ------- | ------------- |
| [`common/`](./common/) | IDs (UUIDv7), time (`Instant`-based durations), errors, hashing, canonical URL helpers, bounded-string types | — |
| [`models/`](./models/) | Domain types shared across planes: `Document`, `DocumentVersion`, `Domain`, `CrawlAttempt`, `ApiKey`, plus the **public** API DTOs with `serde` derives | `common` |
| [`config/`](./config/) | Layered configuration loader, validation, secret references, redaction for logs | `common` |
| [`logging/`](./logging/) | `tracing` setup, JSON formatter, query-redaction helpers, sampling policy, request-id context | `common`, `config` |
| [`security/`](./security/) | Rate-limit bucket derivation, API-key hashing, secret handling, header policy constants, blocklist matching | `common`, `config` |

## Dependency rules

- The graph is a DAG. No cycles, enforced by CI (`cargo metadata` + a rule check).
- `packages/*` may not depend on `services/*` or `apps/*`.
- `packages/*` may not depend on third-party crates that phone home or execute code at
  build time (`build = "…"` with network access is prohibited by
  [ADR-0021](../adr/0021-supply-chain-policy.md)).
- Every `packages/*` crate must be free of `unsafe` — the workspace lints set
  `unsafe_code = "forbid"` globally.
- Every public type that crosses a process boundary is versioned (`#[serde]`, schema-tagged)
  so a rolling upgrade cannot deserialise the wrong shape.

## What does *not* belong here

- Business logic that only one service needs (keep it in that service).
- Anything that could become a dumping ground for "common" code that several callers want
  but that has no coherent single responsibility. If you cannot write one sentence naming
  the responsibility, it does not go in `packages/`.