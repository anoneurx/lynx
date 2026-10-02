# Architecture Overview

High-level view of LYNX components and data flows. Detailed spec: [specification.md](./specification.md).

## Component Map

```
                    ┌─────────────────────────────────────┐
                    │           User / Client             │
                    └──────────────┬──────────────────────┘
                                   │ HTTPS
                                   ▼
                    ┌─────────────────────────────────────┐
                    │        Load Balancer (TLS)          │
                    └──────────────┬──────────────────────┘
                                   │
               ┌───────────────────┼───────────────────┐
               ▼                   ▼                   ▼
      ┌──────────────┐    ┌──────────────┐    ┌──────────────┐
      │  lynx-api    │    │  lynx-api    │    │  lynx-api    │  ← Search Plane
      │  (Pod 1)     │    │  (Pod 2)     │    │  (Pod N)     │
      └──────┬───────┘    └──────┬───────┘    └──────┬───────┘
             │                   │                   │
             └───────────────────┼───────────────────┘
                                 │
                    ┌────────────▼────────────┐
                    │   Tantivy Segments      │
                    │   (ReadOnlyMany Vol)    │
                    └────────────┬────────────┘
                                 │
                    ┌────────────▼────────────┐
                    │      Redis Cache        │
                    │   (Query Results,       │
                    │    Suggestions)         │
                    └─────────────────────────┘
```

## Three Planes (Failure Domains)

| Plane | Services | Scaling | Failure Impact |
| ----- | -------- | ------- | -------------- |
| **Search** | `lynx-api` | HPA (QPS, latency) | Queries fail (mitigated: multi-AZ) |
| **Crawl** | `lynx-crawler` | HPA (queue depth) | New pages delayed |
| **Index** | `lynx-indexer` | Singleton (leader) | Freshness delayed |

## Data Flow

```
Crawl Plane                    Index Plane                    Search Plane
┌─────────────┐               ┌─────────────┐               ┌─────────────┐
│  Frontier   │               │  Indexer    │               │   API       │
│  (Postgres) │───pages ─────▶│  (Tantivy)  │───segments──▶│  (Readers)  │
└─────────────┘               └─────────────┘               └─────────────┘
      │                            │                            ▲
      │                            │                            │
      ▼                            ▼                            │
┌─────────────┐               ┌─────────────┐               ┌────┴────┐
│  S3/MinIO   │               │  Redis      │               │ Cache   │
│  (Raw HTML) │               │  (Pub/Sub)  │────INDEX_RELOAD▶│ (Redis) │
└─────────────┘               └─────────────┘               └─────────┘
```

## Key Design Principles

1. **Read-only search plane** — No writes, horizontally scalable, zero-downtime reloads
2. **Async boundaries** — Planes communicate via Postgres, Redis, S3 (no direct RPC)
3. **Immutable indexes** — Generation-based, atomic symlink swap, instant rollback
4. **No query logging** — Privacy by design; metrics only
5. **Rust everywhere** — Memory safety, performance, single binary deployment

## Technology Choices

| Layer | Choice | ADR |
| ----- | ------ | --- |
| Language | Rust | [0001](./adr/0001-rust-language.md) |
| Async Runtime | Tokio | [0002](./adr/0002-tokio-runtime.md) |
| Search Index | Tantivy | [0003](./adr/0003-tantivy-index.md) |
| Operational DB | PostgreSQL | [0005](./adr/0005-postgres-operational.md) |
| Cache/Coordination | Redis Cluster | [0006](./adr/0006-redis-cache-coordination.md) |
| Deployment | Kubernetes (EKS/GKE) | [0009](./adr/0009-kubernetes-deployment.md) |
| GitOps | ArgoCD + Kustomize | [0015](./adr/0015-gitops-argocd.md) |
| Observability | OpenTelemetry → Prometheus/Loki/Jaeger | [0019](./adr/0019-telemetry-opentelemetry.md) |

## Further Reading

- [Operations Guide](../operations/README.md) — Deployment, scaling, monitoring
- [ADR Index](../adr/README.md) — All architecture decisions
- [Diagrams](../diagrams/README.md) — Data flow, threat boundary, deployment
- [Security Model](../threat-model.md) — Trust zones, attack surface