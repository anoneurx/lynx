# Deployment — LYNX

Staged deployment, from one host to a distributed platform. Architecture:
[docs/architecture.md](./docs/architecture.md). Operations:
[docs/operations/](./docs/operations/README.md).

## Stages at a glance

| Stage | Shape | Index | DB | Crawl | Target corpus | Indexing p95 | Target RTO |
| ----- | ----- | ----- | -- | ----- | ------------- | ------------- | ---------- |
| Local | Compose | 1 shard, local dir | container | container | 100 docs | — | — |
| Development | Compose / k3d | 1 shard | container | 1 worker | 100 k | < 500 ms | — |
| Small production | 1 host + managed services | 1 shard | managed 2 vCPU / 8 GB | 2 workers | 2 M | < 400 ms | 4 h |
| Medium | Split planes | 2 shards | managed 4 vCPU / 16 GB + replica | 8 workers | 50 M | < 500 ms | 2 h |
| Large | Distributed | 16 shards | HA 8 vCPU / 64 GB | 64 workers | 5 B | < 800 ms | 30 min |

Latency figures are **targets**, measured at the stated corpus size, single region, warm
cache. They are not guarantees and they are revisited as the architecture changes.

## Stage 1 — Small production

One host runs the query plane and the crawl plane; Postgres and Redis are managed.

```text
                    ┌──────────────── single host ────────────────┐
  CDN ───▶ web ───▶ lynx-api ──▶ index (local SSD)                 │
                    │      └──▶ Redis (managed)                     │
                    lynx-crawler ──▶ Postgres (managed, 2 vCPU/8GB) │
                    lynx-indexer ──▶ index                          │
                    └──────────────────────────────────────────────┘
```

What changes from development: secrets move to a secret manager; TLS terminates at the
edge; monitoring and alerting go live; backups and a restore rehearsal become real; the
production config validator enforces the security posture.

What this stage is for: validating that the search path is fast and correct before
investing in distribution. It is a deliberate single point of failure, with backups and a
documented recovery time — not an oversight.

## Stage 2 — Medium

Planes are separated. The crawl plane gets its own nodes and its own network policy.

```text
  LB ──▶ web ──▶ api-1, api-2 ──▶ index readers (1–2 shards)
                    │    └──▶ Redis (managed)
  crawl nodes ──▶ crawler ×N ──▶ Postgres primary
  index nodes ──▶ indexer ×N ──▶ Postgres replica (read-only reads)
                     └────────▶ index writers (shard PVCs)
```

What changes: network policies (crawl plane: no ingress, egress 80/443 + DNS only);
autoscaling on CPU and queue depth; index sharding by 2; a Postgres read replica; index
snapshots; separate deployment and scaling for the API and the crawler.

## Stage 3 — Large

```text
  LB / CDN
    └──▶ API cluster (autoscaled) ──▶ search cluster (sharded, replicated readers)
                                     ├──▶ distributed index (16 shards, N replicas)
                                     └──▶ shard coordinator
  crawl cluster (autoscaled, spot-tolerant) ──▶ Postgres (HA) + read replicas
  indexer cluster ──▶ index writers
```

What changes: sharding with per-shard top-K plus global rerank; replication for reader
availability; coordinator for shard health and routing; crawl workers on preemptible
capacity with resumable frontier state; region awareness for latency; cost controls and
budget alerts; multi-region disaster recovery.

The migration path is additive at each step, and the index abstraction is what makes it
possible — the API never learns which Tantivy version it is talking to.

## Deployment pipeline

```text
PR → CI (lint, typecheck, privacy guard, tests, security suites, quality gates)
   → merge to main → staging deploy (automatic) → smoke tests
   → signed tag → production preflight (quality report, perf budget, migration check)
   → manual approval → helm --atomic deploy → index generation switch
   → smoke + privacy spot check → confirm new generation serving
   → (on any failure) automatic rollback of release and index generation
```

Staging uses synthetic data only. Production deploys require manual approval and never
run untrusted pull-request code with secrets.

## Index generation switching

An index is versioned data, deployed like code:

```text
build new generation → snapshot → deploy as a new shard set
  → health check → atomic switch of the active generation
  → keep the previous generation for 24 h → delete
```

Rollback is a switch back to the previous generation, not a rebuild. See
[docs/operations/runbooks.md](./docs/operations/runbooks.md).

## Configuration per environment

| | Development | Staging | Production |
| - | ----------- | ------- | ---------- |
| Config file | `config/development.toml` | `config/production.toml` + overlay | `config/production.toml` + overlay |
| Secrets | `.env` | secret manager | secret manager |
| Log format | pretty | JSON | JSON |
| Tracing | full | 10 % | 1 % |
| `admin_require_vpn` | false | true | true |
| `ai.enabled` | false | false | false by default |
| Data | seeds | synthetic | real crawl + real queries (queries not logged) |

## Security posture in production

Enforced by the production validator, which refuses to start otherwise:

- no literal secret in any config file — references only
- `privacy.query_logging = false`
- TLS ≥ `security.tls_min_version`, no `cors_origins = ["*"]`, no non-first-party origin
- `admin.require_vpn = true`, `admin.enable_basic_auth = false`
- `ai.enabled = false` unless a pinned model id is set
- images pinned by digest, scanned, signed; no `latest`
- non-root, read-only rootfs, dropped capabilities on every workload
- crawl plane network policy: no ingress, egress 80/443 + DNS

## Sizing

Per-worker resources (targets, measured — see [docs/operations/capacity.md](./docs/operations/capacity.md)):

| Service | vCPU | Memory | Notes |
| ------- | ---- | ------ | ----- |
| `lynx-api` | 1–2 | 512 MiB–1 GiB | scales on latency and in-flight requests |
| `lynx-crawler` | 1 | 256 MiB | concurrency is the scaling axis, not CPU |
| `lynx-indexer` | 1–2 | 512 MiB | CPU-bound; scales with CPU |
| `lynx-ai` | 0.5 | 512 MiB | bounded concurrency; token budget is the real limit |

## Cost drivers

Crawling is cheap: bandwidth and time, no egress fee to the origin. Serving is expensive:
query-plane CPU, index memory, and object storage for backups and snapshots. The
cost controls that matter most are index memory, result caching, retention of crawl
history, and the AI token budget. Budget alerts are configured per stage.

## Backups and disaster recovery

Summary; full detail in [docs/operations/disaster-recovery.md](./docs/operations/disaster-recovery.md).

| Artifact | Reproducible | Backup | RPO | RTO |
| -------- | ------------- | ------ | --- | --- |
| Postgres (crawl state, documents, governance) | no | continuous WAL archiving + nightly full | 5 min | 60 min |
| Tantivy index | yes, by re-crawling — but expensive | snapshot per generation | 24 h | 30–60 min |
| Configuration | no | git + SOPS-encrypted secrets | — | minutes |
| Ranking config versions | no | git (versioned) | — | minutes |
| Secrets | no | secret manager, versioned | — | minutes |

Secrets and ranking-config history are the two things we cannot regenerate. Everything else
is either backed up or reconstructible, and the distinction is explicit rather than
assumed.

## Scaling triggers — measure, do not guess

Move to the next stage when a specific threshold is hit, not when it feels necessary:

| Trigger | Threshold | Action |
| ------- | --------- | ------ |
| API | p95 > 400 ms with cache hit rate > 70 % | more replicas, then more index memory |
| Index | single shard cannot hold the corpus in budget | shard |
| Retrieval | shard fan-out > 8 shards adds > 100 ms to p95 | coordinator, or hierarchical top-K |
| Crawler | frontier depth grows faster than throughput drains it | more crawl nodes |
| Postgres | read load > 60 % of replica capacity | more read replicas, then partition |
| Search | index RSS > 70 % of node memory | more memory, then shard |

Every trigger is a measurement with a recorded number. That is the difference between a
scaling plan and a hope.