# Entity relationships

```mermaid
erDiagram
    CRAWL_HOST ||--o{ URL : hosts
    CRAWL_HOST ||--|| ROBOTS_CACHE : "has robots"
    CRAWL_HOST ||--o{ FRONTIER : "schedules"
    URL ||--o{ FRONTIER : "queued as"
    URL ||--o| DOCUMENT : "becomes"
    DOCUMENT ||--o{ LINK : "source of"
    URL ||--o{ LINK : "target of"
    DOCUMENT ||--o| DEDUP_SIGNATURE : "signed by"
    DEDUP_SIGNATURE }o--|| CLUSTER : "groups into"
    CLUSTER ||--o{ DOCUMENT : "contains"
    DOCUMENT ||--o{ TOMBSTONE : "may be"
    DOCUMENT ||--o{ QUALITY_LABEL : "labelled by"
    DOCUMENT ||--o| TANTIVY_DOC : "mirrored in"

    CRAWL_HOST {
        uuid id PK
        text hostname
        text registrable_domain
        inet resolved_ip
        timestamptz next_request_at
        int rate_limit
    }
    URL {
        uuid id PK
        bytea url_hash UK
        text canonical_url
        uuid host_id FK
        text path
    }
    FRONTIER {
        uuid id PK
        uuid url_id FK
        uuid host_id FK
        smallint priority
        frontier_state state
        timestamptz available_at
        smallint attempts
    }
    DOCUMENT {
        text doc_id PK
        uuid url_id FK
        document_state state
        bytea content_hash
        uuid cluster_id FK
        real quality_score
        real spam_score
        text lang
    }
    LINK {
        bigint id PK
        text source_doc_id FK
        uuid target_url_id FK
        text target_doc_id
        text anchor_text
        boolean is_nofollow
        text context
    }
    DEDUP_SIGNATURE {
        uuid id PK
        bytea structural_hash
        bytea text_hash
        uuid cluster_id FK
        int member_count
    }
    CLUSTER {
        uuid id PK
        text kind
        real confidence
        int member_count
        int host_diversity
    }
    TOMBSTONE {
        text doc_id PK
        text reason
        text requested_by
        boolean verified
    }
```

## The crawl pipeline in relations

```mermaid
sequenceDiagram
    participant W as Worker
    participant DB as Postgres
    participant H as crawl_host
    participant U as url
    participant D as document
    participant T as Tantivy

    W->>DB: claim frontier (SKIP LOCKED, honouring host rate)
    DB->>H: check rate + backoff
    DB-->>W: url_id, host_id, resolved_ip
    W->>W: fetch via SSRF chain, pin to validated IP
    W->>W: parse in sandboxed process
    W->>DB: upsert document, insert links, compute signature
    W->>T: write document with fast fields
    W->>DB: insert frontier rows for new links
    W->>DB: mark frontier fetched
```

Every step is idempotent by `doc_id` and `url_hash`. A worker that crashes after writing to
Tantivy but before updating Postgres re-fetches the URL, finds the content hash unchanged, and
skips the index write. That ordering — index first, then Postgres — means a crash leaves a
stale frontier row rather than an orphaned document, and the stale row is the harmless failure.

## Deduplication relationships

```mermaid
erDiagram
    DEDUP_SIGNATURE }o--|| CLUSTER : "belongs to"
    DEDUP_SIGNATURE ||--o{ DOCUMENT : "matches"
    CLUSTER ||--o{ DOCUMENT : "suppresses"

    DEDUP_SIGNATURE {
        bytea structural_hash "DOM shingles, no text"
        bytea text_hash "full text"
        int shingles
        real minhash_jaccard
    }
    CLUSTER {
        text kind "near_duplicate | doorway | template"
        real confidence
        int member_count
        int host_diversity "syndication vs doorway"
    }
```

Two hashes, and the reason is the main subtlety in the whole model: a structural hash ignoring
text catches template farms; a text hash catches verbatim copies. One hash for both produces
false positives on legitimate templated sites and false negatives on paraphrased content.

`host_diversity` is what separates syndication from a doorway network. Fifty identical press
releases on fifty independent hosts is one document. Fifty near-identical landing pages with
interchanged local terms is a network, and the same structural signature describes both.

## Link graph

```mermaid
graph LR
  D1[doc A] -->|body| U1[url 1]
  D1 -->|footer| U2[url 2]
  D2[doc B] -->|nav| U1
  D3[doc C] -->|main| U3[url 3]
  U1 --> D4[doc D]
  U3 --> D5[doc E]

  style D1 fill:#e8f4ea
  style U1 fill:#fff4e6
```

Link `context` decides whether an edge enters the authority graph:

| Context | In the graph? | Reason |
| ------- | ------------- | ------ |
| `main` | Yes | An editorial reference |
| `body` | Yes | An editorial reference |
| `nav` | No | Site navigation, present on every page |
| `footer` | No | Template, present on every page |
| Any, `is_nofollow` | No | The author asked us not to treat it as endorsement |
| Any, `is_sponsored` | No | Paid placement is not an editorial judgement |

Excluding nav and footer is not a refinement. If they were included, every site would link to
every other site through shared navigation, and PageRank would measure template overlap rather
than reputation.

## Deletion relationships

```mermaid
flowchart TD
  R[Request: robots / meta / operator / legal] --> T["Write tombstone<br/>(durable, first)"]
  T --> D1[Remove from every generation]
  D1 --> V[Verify absence by querying the index]
  V -->|absent| M["Mark verified=true"]
  V -->|present| D1
  M --> A[Emit deletion event, no user data]

  style T fill:#ffe6e6
  style V fill:#fff4e6
```

The ordering is the design. Tombstone before removal means a replayed crawl cannot resurrect the
document; verification after removal means a deletion is confirmed rather than requested.

## Cardinality rationale

| Relationship | Cardinality | Note |
| ------------- | ----------- | ---- |
| `crawl_host` → `url` | 1 : many | One host, many URLs |
| `url` → `document` | 1 : 0..1 | Not every URL is indexable |
| `document` → `link` | 1 : many | A document has many outlinks |
| `dedup_signature` → `document` | 1 : many | One signature, many members |
| `cluster` → `document` | 1 : many | A cluster suppresses its members |
| `document` → `tombstone` | 1 : 0..1 | A tombstone is unique per document |

`url` → `document` being 0..1 is worth noting: the majority of discovered URLs never become
documents, which is why `frontier` is the largest table and `document` is not.

## Cross-system consistency

```text
the same doc_id exists in Postgres and in Tantivy
Postgres is the label; Tantivy is the content
```

| Failure | Detection | Response |
| ------- | --------- | -------- |
| Document in Tantivy, absent in Postgres | Startup reconciliation job | Re-derive the row from the index |
| Document in Postgres, absent in Tantivy | Same job | Re-index |
| `doc_id` collision | Impossible with ULIDs; a UUIDv7 `url` id is not a doc id | n/a |
| Cluster id differs between systems | Cluster ids are stored as fast fields and compared | [RB-03](../../security/runbooks/index-corruption.md) |

Reconciliation runs on every new index generation rather than on a schedule, because a
generation swap is precisely when the two systems could have diverged.

## What is not modelled

| Not modelled | Why |
| ------------ | --- |
| Users | There are none |
| Queries | There are none |
| Sessions | There are none |
| Impressions | Not tracked |
| Clicks | Not tracked |

An ERD that omitted these would look incomplete against a conventional search engine's schema.
That absence is the schema, and it is the reason the privacy claims are verifiable rather than
aspirational.