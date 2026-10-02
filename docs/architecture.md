# LYNX Architecture

> The short version. The long version is
> [docs/architecture/specification.md](./architecture/specification.md).

## 1. System context

```mermaid
flowchart LR
  U([User]) -->|HTTPS| CDN[Edge / CDN\nstatic assets only]
  CDN --> WEB[Web UI\napps/web]
  WEB -->|HTTPS, no cookies| API[Search API\napps/api]
  API --> PL[Privacy Layer\nrate limit · minimisation · redaction]
  PL --> QP[Query Processor\nparse · normalise · spell · expand]
  QP --> SE[Search Engine\nindex reader]
  SE --> RE[Ranking Engine\nBM25F · freshness · quality · spam]
  RE --> IN[(Search Index\nTantivy shards)]
  SE --> IN
  API --> RESP[Response shaping · dedup · pagination]

  subgraph Crawl["Crawl plane (offline, outbound-only)"]
    SEEDS[Seed sets] --> FR[URL Frontier\nPostgres SKIP LOCKED]
    FR --> CR[Crawler\nrustls · bounded fetch]
    CR --> RP{Robots /\nPolicy check}
    RP -->|allow| DL[Downloader\nsize · time · redirect caps]
    RP -->|deny| FR
    DL --> PR[Parser\nHTML → text + links + meta]
    PR --> IX[Indexer\ntokenise · normalise · dedup]
    IX --> IN
    IX --> DB[(Postgres\noperational state)]
    FR --> DB
  end

  O11Y[Prometheus · JSON logs · OTel] -.-> API
  O11Y -.-> CR
```

Two planes, deliberately separated:

- **Query plane** — internet-facing, read-only over the index, no outbound network access.
- **Crawl plane** — outbound-only, never internet-facing, holds the only code that touches
  untrusted web content. It cannot read user requests.

Compromise of the crawl plane does not become a query-log compromise, because the crawl
plane never has access to query data.

## 2. Component responsibilities

| Component | Repo path | Responsibility | Scaling axis |
| --------- | --------- | -------------- | ------------ |
| Web UI | `apps/web` | React SPA, SSG for static pages, privacy-preserving SERP | CDN, static |
| Admin console | `apps/admin` | Private ops dashboard, never public | Behind VPN/SSO |
| Search API | `apps/api` | axum HTTP edge, auth, rate limits, response assembly | Horizontal, stateless |
| Privacy layer | `packages/security` + `services/api` middleware | IP-prefix bucket hashing, daily salt rotation, header policy, redaction | In-process |
| Query processor | `services/query` | Lexer, parser, AST, operators, spell correction, expansion | CPU-bound, shardable |
| Ranking engine | `services/ranker` | Feature extraction, weighted scoring, dedup, penalties | CPU-bound, shardable |
| Search index | `services/indexer` (writer) / `apps/api` (reader) | Tantivy shards, commit policy, snapshots | Shards × replicas |
| URL frontier | `services/frontier` | Priority queue, dedup, scheduling, budget accounting | Postgres contention → shards |
| Crawler | `services/crawler` | Fetch execution, SSRF policy, politeness, budgets | Horizontal, per-host caps |
| Parser | `services/parser` | HTML→text, links, canonical, metadata, language, dates | CPU-bound, batchable |
| Indexer | `services/indexer` | Normalise, tokenise, dedup, write documents, commit | Horizontal by doc id |
| Document store | `database/` (Postgres) | Extracted text + metadata per URL version | Partitioned |
| Metadata store | `database/` (Postgres) | Domains, robots, frontier state, link graph, ops | Partitioned |
| Suggestions | `services/suggestions` | Prefix→completion from term/document frequency | Read-only replica |
| Astra (AI) | `services/ai` | Retrieval-grounded answers with citations, optional | Isolated, rate limited |
| Monitoring | `infrastructure/monitoring` | Prometheus, Grafana, alert rules | Infra |

## 3. Data flow: a search

```mermaid
sequenceDiagram
  autonumber
  participant U as User
  participant W as Web UI
  participant A as Search API
  participant P as Privacy Layer
  participant Q as Query Processor
  participant S as Search Engine
  participant R as Ranker
  participant I as Index
  U->>W: submit query
  W->>A: GET /api/v1/search?q=...
  A->>P: client IP, headers, request_id
  Note over P: derive rate-limit bucket =<br/>HMAC(daily salt, /24 prefix).<br/>No query, no cookie, no ID.
  P->>Q: normalised request
  Q->>Q: lex → parse → AST → validate operators
  Q->>Q: normalise, tokenise, fold, spell-correct
  Q->>S: compiled plan
  S->>I: BM25F over up to 8 fields
  I-->>S: top 2000 candidates + doc features
  S->>R: candidate set
  R->>R: freshness × quality × authority − spam
  R->>R: cluster dedup suppression
  R-->>A: ranked top N with signal breakdown
  A->>A: cache lookup(key = HMAC(query) + index_version)
  A-->>W: JSON result page (no PII, no cookies)
  W-->>U: render
  Note over A: metrics record shapes only:<br/>length bucket, token count, operator class. Never the text.
```

## 4. Data flow: a crawl

```mermaid
flowchart TD
  S([Seed URL / outlink]) --> DEDUP{Canonicalise<br/>+ dedup key}
  DEDUP -->|seen| DROP([Drop])
  DEDUP -->|new| POL{Crawl policy:<br/>scope, budget,<br/>depth, params}
  POL -->|reject| DROP
  POL -->|accept| RB{Robots check<br/>UA-specific}
  RB -->|disallow| DROP
  RB -->|allow| PRIO[Compute priority<br/>depth · freshness · priority hint]
  PRIO --> Q[(frontier_queue)]
  Q --> W[Worker claims row<br/>SKIP LOCKED]
  W --> SAFE[Fetch safety gate<br/>scheme · host · IP · port · redirects]
  SAFE -->|deny| BAN[Record denial reason<br/>+ repeat-offender escalation]
  SAFE -->|allow| HOST[Per-host token bucket<br/>crawl-delay · concurrency]
  HOST --> FETCH[HTTP/1.1+2 fetch<br/>rustls · no cookies · no JS]
  FETCH --> CAP{Response caps:<br/>status · type · bytes ·<br/>ratio · deadline}
  CAP -->|violate| ERR[Classify error → retry/backoff/ban]
  CAP -->|ok| PARSE[Parse: text, links,<br/>canonical, meta, lang, dates]
  PARSE --> CONT[Content safety:<br/>normalise · strip scripts<br/>truncate · flag injection patterns]
  CONT --> DEDUP2[Content signature<br/>SimHash + MinHash cluster]
  DEDUP2 --> IDX[Index: tokenise, field<br/>weights, doc stats, commit]
  IDX --> OUT[New outlinks → back to S]
  IDX --> DB[(Postgres: attempts,<br/>document version, links)]
```

## 5. Search index shape

```mermaid
erDiagram
  SHARD ||--o{ SEGMENT : contains
  SHARD ||--o{ SEARCH_INDEX : "read by"
  SEARCH_INDEX ||--o{ INDEXED_DOCUMENT : contains

  SHARD {
    int shard_id PK
    string role "primary|replica"
    string state "building|serving|sealed"
    bigint doc_count
    string base_version
    datetime created_at
  }
  SEGMENT {
    string segment_id PK
    int shard_id FK
    bigint live_docs
    bigint bytes
    datetime sealed_at
  }
  INDEXED_DOCUMENT {
    string doc_id PK "url-hash based, stable"
    string url_id FK
    string domain_id FK
    string language
    datetime last_crawl_at
    datetime last_modified_guess
    string cluster_id
    string ranking_config_version
  }
```

Index fields and weights are defined in
[docs/indexing/index-schema.md](./indexing/index-schema.md). Index technology choice
and its escape hatch are in [ADR-0002](./adr/0002-search-index.md).

## 6. Ranking pipeline

```mermaid
flowchart LR
  C[Candidates from BM25F] --> N[Normalise feature vectors]
  N --> Q[Quality signals]
  N --> F[Freshness decay]
  N --> A[Authority / link graph]
  N --> S[Spam signals]
  Q --> SC[Weighted sum<br/>versioned config]
  F --> SC
  A --> SC
  S --> PEN[Multiplicative penalties]
  SC --> PEN
  PEN --> DD[Near-duplicate cluster suppression]
  DD --> OUT[Ranked results +<br/>per-signal breakdown]
```

Every score is reproducible from `(doc_id, index_version, ranking_config_version)`.
Breakdowns are returned in debug mode and always internally, so ranking complaints are
answerable. See [docs/ranking/](./ranking/overview.md).

## 7. Deployment topology

```mermaid
flowchart TB
  subgraph Edge
    LB[CDN / L7 LB]
  end
  subgraph QP["Query plane"]
    WEB[web]
    API1[api-1]
    API2[api-2]
  end
  subgraph Data
    RD[(index readers)]
    PG[(Postgres primary)]
    PGR[(Postgres replica)]
    RDISS[(Redis cache + RL)]
  end
  subgraph CP["Crawl plane (no inbound)"]
    CR1[crawler-1]
    CR2[crawler-2]
    IX1[indexer-1]
    FR[frontier workers]
  end
  OBS[[Prometheus · Grafana · Loki · OTel]]
  LB --> WEB
  LB --> API1 & API2
  API1 & API2 --> RD & RDISS
  CR1 & CR2 --> PG
  FR --> PG
  IX1 --> RD & PG
  PGR --> IX1
  API1 -.-> OBS
  CR1 -.-> OBS
  PG -.-> S3[(Backups / snapshots)]
  RD -.-> S3
```

The crawl plane runs on a separate network policy: **no ingress from the internet, egress
restricted to port 80/443 and DNS.** A compromised crawler host cannot be used as a pivot
into the query plane, and the query plane cannot be used to make the crawler fetch internal
addresses.

## 8. Key architectural decisions

| Decision | Choice | Why | ADR |
| -------- | ------ | --- | --- |
| Primary language | Rust | Untrusted-input safety, no GC latency, single binary | [0003](./adr/0003-primary-language.md) |
| Service shape | Modular monolith + workers | No premature microservices; independent scaling via process boundaries | [0001](./adr/0001-project-architecture.md) |
| Search index | Tantivy behind a trait | Rust-native, embeddable, BM25F; replaceable | [0002](./adr/0002-search-index.md) |
| Primary DB | PostgreSQL | Operational state needs relational integrity | [0004](./adr/0004-database.md) |
| Frontier queue | Postgres `SKIP LOCKED` | One less dependency; migrate to broker when measured | [0008](./adr/0008-frontier-queue.md) |
| Frontend | Vite + React + Tailwind | One component model, self-hosted assets | [0009](./adr/0009-frontend-rendering.md) |
| Ranking | Feature-based, versioned config | Explainability, reproducibility, safe A/B | [0010](./adr/0010-ranking-framework.md) |
| AI | Optional, isolated, retrieval-grounded | Never let generation replace retrieval | [0015](./adr/0015-ai-integration-boundary.md) |
| IDs | UUIDv7 | Time-ordered, index-friendly, non-enumerable | [0017](./adr/0017-identifier-strategy.md) |

## 9. What deliberately does not exist in v0.1

- No user accounts, no profiles, no history, no cookies, no fingerprinting.
- No advertising or bidding systems.
- No distributed index or cluster orchestration.
- No JavaScript execution in the crawler.
- No full-page HTML archive (extract and store text; see
  [ADR-0016](./adr/0016-content-storage-policy.md)).
- No agentic AI loop over untrusted content.

See [ROADMAP.md](../ROADMAP.md) for when each of these is reconsidered and why.