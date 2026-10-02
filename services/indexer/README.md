# `services/indexer` — document pipeline writer

**Binary:** `lynx-indexer`. Consumes parse output, writes the search index and the
operational stores. Full design: [docs/indexing/](../../docs/indexing/README.md).

## Pipeline

```text
ParsedDocument
  → normalisation        unicode NFC, control-char strip, width folding, case fold
  → language confirm     (parser estimate + stopword ratio agreement)
  → tokenisation         unicode word segmentation, per-field tokenizers
  → stopword handling    per-language stoplists, kept for phrase queries only
  → stemming              per-language (Porter2 / Snowball), index-time only
  → term statistics       tf, field lengths, doc length, position stream
  → dedup                SimHash band lookup → MinHash verification → cluster assignment
  → field assembly        title / headings / body / anchors / url with weights
  → tantivy document      with a monotonic doc version and crawl timestamp
  → commit policy         every N docs or T seconds, WAL-backed
  → operational writes    document + version rows + link edges
```

## Responsibilities

| Concern | Detail |
| ------- | ------ |
| Indexing | Write the Tantivy document with the field weights from `index-schema.md` |
| Dedup | Assign a `cluster_id`; near-duplicates get a demotion weight rather than deletion |
| Freshness metadata | `last_crawl_at`, `last_modified_guess`, `published_at`, content hash |
| Versioning | Each URL has a `document_version` row; only the newest is indexed |
| Idempotency | Re-indexing the same content hash is a no-op (avoids commit churn) |
| Backpressure | Pauses consumption when the index writer stalls or the frontier is deep |
| Compaction | Merge policy and force-merge triggered by segment count/size thresholds |
| Snapshotting | Tagged snapshots to object storage for disaster recovery |

## Rules

- The indexer is the **only** writer to the search index. No admin route may write a
  document directly; admin may trigger a rebuild or a purge job, which runs this binary.
- The indexer never reads user data and never accepts a query. It has no API surface.
- Field weights and tokenizer versions are recorded in the index manifest so a ranking
  result can always be attributed to an exact analysis configuration.