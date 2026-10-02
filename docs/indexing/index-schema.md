# Index schema

The Tantivy schema, field weights, stored data, and the trait that keeps the index
replaceable.

## The abstraction boundary

```rust
// Defined by the CONSUMER (apps/api), implemented by services/indexer.
trait SearchIndexReader: Send + Sync {
    fn generation(&self) -> IndexGeneration;
    fn search(&self, plan: &QueryPlan, opts: &SearchOptions) -> Result<Vec<Candidate>>;
    fn count(&self, plan: &QueryPlan) -> Result<u64>;
    fn document(&self, doc_id: DocId) -> Result<Option<DocumentMetadata>>;
    fn explain(&self, plan: &QueryPlan, doc_id: DocId) -> Result<Explanation>;
}
```

Nothing in `apps/api` or `services/ranker` imports a Tantivy type. This is what makes the
index swappable — to Lucene, OpenSearch, or a custom implementation — without rewriting the
API or the ranker. See [ADR-0002](../adr/0002-search-index.md).

## Fields

| Field | Type | Weight | Stored | Used for |
| ----- | ---- | ------ | ------ | -------- |
| `title` | text, word+CJK | **12.0** | yes | highest-precision match, snippet title |
| `headings` | text, word+CJK | 6.0 | yes (as text) | section relevance |
| `body` | text, word+CJK | 2.0 | yes (main + full) | the bulk of the signal, snippet source |
| `anchors` | text, word | 4.0 | yes | anchor-text matching, `inanchor:` |
| `url` | text, path-tokenised | 3.0 | yes | URL matching, `inurl:` |
| `alt` | text, word | 1.5 | yes | image alt text, captions |
| `domain` | keyword (registrable domain) | n/a | yes | `site:` filter |
| `host` | keyword | n/a | yes | exact-host filter |
| `lang` | keyword | n/a | yes | `lang:` filter |
| `content_type` | keyword | n/a | yes | `filetype:` filter |
| `published_at` | i64 timestamp | n/a | yes | `before:` / `after:`, freshness |
| `modified_at` | i64 timestamp | n/a | yes | freshness |
| `crawled_at` | i64 timestamp | n/a | yes | crawl-recency freshness |
| `spelling` | text (lowercase surface forms) | n/a | no | spell correction dictionary |
| `spelling_df` | u64 | n/a | no | dictionary frequency threshold |
| `quality_score` | f32 fast field | n/a | yes | quality signal |
| `spam_score` | f32 fast field | n/a | yes | spam penalty |
| `host_rank` | f32 fast field | n/a | yes | authority signal |
| `domain_rank` | f32 fast field | n/a | yes | authority signal (domain-capped) |
| `content_length` | u32 fast field | n/a | yes | thin-content detection |
| `link_count` | u32 fast field | n/a | yes | quality/spam signal |
| `cluster_id` | u64 fast field | n/a | yes | duplicate suppression |
| `quality_flags` | bitflags | n/a | yes | truncated, js_required, thin, soft_404, … |
| `ranking_config_version` | keyword | n/a | yes | reproducibility attribution |

Fast fields (`f32`/`u32`/`i64` numeric) are stored uncompressed and un-tokenised so they can
be read and filtered on without touching the inverted index. Signals that ranking reads per
candidate are all fast fields — this is why ranking does not need a document fetch per
candidate.

## Weight rationale

```text
title      12.0   A title match is a strong editorial claim by the page author
headings    6.0   Structural, less curated than titles
anchors     4.0   Someone wrote this text describing a target
url         3.0   Exact and cheap, but URLs game easily, so it is not higher
body        2.0   The bulk of the signal, and the field most easily stuffed
alt         1.5   Frequently auto-generated, so low
```

`body` is deliberately the lowest of the content fields despite carrying most of the text.
Term-frequency saturation in BM25 limits stuffing, and the spam features apply a
multiplicative penalty on top. See [ranking/bm25f.md](../ranking/bm25f.md) and
[ranking/spam.md](../ranking/spam.md).

## Analyzers

| Analyzer | Pipeline | Used by |
| -------- | -------- | ------- |
| `standard` | unicode segmentation → stopword marking → stem → lowercase | title, headings, body, anchors, alt |
| `url` | split on `/ - _ . ? & =`, drop scheme and host, no stemming | url |
| `keyword` | whitespace, no analysis | domain, host, lang, content_type |
| `spelling` | lowercase, no stemming, no stopwords | spelling dictionary |
| `cjk_bigram` | character bigrams over CJK runs, lowercase | mixed into the `standard` analyzers |

Query-time analyzers are the same objects, configured from the manifest's
`tokenizer_version`. The API never configures an analyzer itself.

## Stored versus indexed

Stored (in the segment, read at result time):

- Everything needed to render a result without a second lookup: title, url, snippet source,
  language, dates, display_url.
- Everything ranking needs: all fast fields above.
- `cluster_id`, `quality_flags`, `ranking_config_version` for dedup, quality, and attribution.

Not stored:

- Raw HTML. Ever. See [ADR-0016](../adr/0016-content-storage-policy.md).
- Full text of very long documents in the SERP path — the snippet is generated from the
  stored body text, capped, and the full text is fetched only by the `documents/{id}`
  endpoint (which is also how the AI layer gets its passages).

## Snippets

Generated at query time from the stored body text, not extracted at index time:

```text
1. locate the best window of ~240 characters around the densest query-term matches
2. prefer a window that starts at a sentence boundary
3. escape HTML, strip control characters
4. mark matched terms with <mark> (tags inserted by us, content escaped)
```

Because the source is stored text rather than markup, there is nothing in a snippet that we
did not put there. Escaping is applied unconditionally — the escaping path is not optional
and not configurable.

## Shards and generations

```text
generation = { id, tokenizer_version, ranking_config_version, created_at,
               shards: [shard_id], doc_count, size_bytes, checksum }

API replicas read exactly one generation at a time.
Switching is atomic: open the new generation's readers, then swap the pointer.
The previous generation is retained for 24 h.
```

One generation per index schema. A schema change means a new generation; old generations
remain readable until retired, which is what makes schema changes and ranking changes
safe to roll back.

## Size expectations

Target: under 400 bytes per document compressed for English text.

The main contributors, in order: body text, stored snippet source, positions, the spelling
dictionary, and quality fast fields. If size grows past the target, the levers are in this
order: reduce position caps, drop the full-text copy and keep only main text, shrink the
stored metadata set. Raising the budget is not one of the levers.

## Migration notes

| Change | Impact |
| ------ | ------ |
| Add a stored fast field | New generation; readers can serve the old one until switched |
| Add a stored non-fast field | New generation; small size increase |
| Change a weight | Ranking config version bump, **no** index rebuild (weights live in the reader) |
| Change an analyzer | New generation + full rebuild |
| Add a keyword field for a new filter | New generation |
| Change a field type | New generation + migration of the document store |

The weight row is the important one: weights are reader-side, so tuning ranking does not
require a re-index. Only analysis changes do.