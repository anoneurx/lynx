# Architecture Decision Records

Architecture Decision Records. One file per decision, immutable once accepted. A decision
is changed by writing a **new** ADR that supersedes the old one and updating the index
below — never by editing the original record.

## Format

```markdown
# ADR-NNNN: Title

Status:      Proposed | Accepted | Superseded by ADR-NNNN | Deprecated
Date:        YYYY-MM-DD
Deciders:    who
Supersedes:  ADR-NNNN
Affects:     crates, tables, configs, docs

## Context
The forces at play. Constraints, requirements, what was already decided upstream.

## Decision
What we are doing, stated in the present tense.

## Rationale
Why this option and not the alternatives. Be specific about the trade-off accepted.

## Alternatives considered
Each with the reason it lost. An ADR with no rejected alternatives is not a real decision.

## Consequences
Positive, negative, and what becomes harder. Follow-up work. What this forecloses.

## Revisit when
The measurable trigger that would reopen this decision.
```

## Index

| ADR | Title | Status | Date |
| --- | ----- | ------ | ---- |
| [0001](./0001-rust-language.md) | Rust as Primary Implementation Language | Accepted | 2024-01-15 |
| [0002](./0002-tokio-runtime.md) | Tokio as Async Runtime | Accepted | 2024-01-15 |
| [0003](./0003-tantivy-index.md) | Tantivy as Search Index Library | Accepted | 2024-01-20 |
| [0004](./0004-index-generation-reload.md) | Index Generation & Atomic Reload | Accepted | 2024-02-01 |
| [0005](./0005-postgres-operational.md) | Postgres for Operational Data | Accepted | 2024-01-25 |
| [0006](./0006-redis-cache-coordination.md) | Redis for Cache & Coordination | Accepted | 2024-01-25 |
| [0007](./0007-separate-planes.md) | Separate Crawl, Index, Search Planes | Accepted | 2024-02-10 |
| [0008](./0008-no-query-logging.md) | No Query Logging | Accepted | 2024-02-15 |
| [0009](./0009-kubernetes-deployment.md) | Kubernetes for Production Deployment | Accepted | 2024-03-01 |
| [0010](./0010-crawler-architecture.md) | Crawler Architecture | Accepted | 2024-03-15 |
| [0011](./0011-indexer-architecture.md) | Indexer Architecture | Accepted | 2024-03-20 |
| [0012](./0012-sharding-strategy.md) | Sharding Strategy | Accepted (Deferred) | 2024-04-01 |
| [0013](./0013-ranking-pipeline.md) | Ranking Pipeline | Accepted | 2024-04-15 |
| [0014](./0014-ab-testing.md) | A/B Testing Framework | Accepted | 2024-05-01 |
| [0015](./0015-gitops-argocd.md) | GitOps with ArgoCD | Accepted | 2024-05-15 |
| [0016](./0016-multi-az-ha.md) | Multi-AZ High Availability | Accepted | 2024-06-01 |
| [0017](./0017-sandboxed-html-parsing.md) | Sandboxed HTML Parsing | Accepted | 2024-06-15 |
| [0018](./0018-fuzzing-strategy.md) | Fuzzing Strategy | Accepted | 2024-07-01 |
| [0019](./0019-telemetry-opentelemetry.md) | Telemetry (OpenTelemetry) | Accepted | 2024-07-15 |
| [0020](./0020-alerting-strategy.md) | Alerting Strategy | Accepted | 2024-08-01 |
| [0021](./0021-configuration-management.md) | Configuration Management | Accepted | 2024-08-15 |

## Process

1. Open a PR containing only the ADR, status `Proposed`.
2. Discussion happens on the ADR itself, not in private chat. Comments are resolved in the file.
3. On consensus, change status to `Accepted` and merge. Implementation follows in later PRs.
4. An ADR that contradicts shipped code is a bug: either the code changes or a superseding
   ADR is written.

Superseded ADRs stay in the tree with a `Superseded by` line. The record of having changed
our minds is more valuable than the current state.