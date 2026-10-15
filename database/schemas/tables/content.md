# Content (`0006_content.sql`)

The document label plane and the link graph. The Postgres row is a **label** for a
document whose text lives in Tantivy. `doc_id` is the join key in both systems.

## `document`

| Column | Type | Notes |
| ------ | ---- | ----- |
| `doc_id` | `uuid` PK | UUID v5 from the canonical URL; **not** `uuidv7()` |
| `url_id` | `uuid` | logical ref to `url(id)` |
| `state` | `document_state` | `active`\|`shadow_suppressed`\|`soft_404`\|`deleted` |
| `content_hash` | `bytea` | SipHash of extracted text, for change detection |
| `signature_id`, `cluster_id` | `uuid` | dedup membership |
| `lang`, `published_at`, `modified_at`, `content_type` | | metadata |
| `quality_score`, `spam_score`, `spam_signals` | `real`, `real`, `text[]` | labels for trend analysis |
| `js_required` | `boolean` | needs client JS to render |
| `indexed_generation` | `bigint` | generation currently serving this doc |
| `first_indexed_at`, `last_changed_at`, `created_at`, `updated_at` | | |

`shadow_suppressed` means present in the index but excluded from results; it is
distinct from `deleted`, which must never be resurrectable, and from `soft_404`,
which is indexed as a 404.

Indexes: `document_url_idx`, `document_cluster_idx` (partial), `document_hash_idx`,
`document_state_idx` (partial, `state <> 'active'`).

## `document_version`

History. Only the newest version is indexed; the rest is retained for re-indexing
and takedown. Extracted text lives here, not an inverted index.

`(doc_id, version)` PK, `doc_id` → `document(doc_id)` `ON DELETE CASCADE`,
`content_hash`, `fetched_at`, `status_code`, `content_type`, `byte_size`, `lang`,
`extracted_text`.

## `link`

A real edge table, because authority and duplicate detection both need the graph
and deriving it from the index would mix derived and source data.

| Column | Type | Notes |
| ------ | ---- | ----- |
| `id` | `bigint` identity PK | high-volume append |
| `source_doc_id` | `uuid` | logical ref to `document(doc_id)` |
| `target_url_id` | `uuid` | FK → `url(id)` |
| `target_doc_id` | `uuid` | null until the target is indexed |
| `anchor_text` | `text` | |
| `is_nofollow`, `is_sponsored` | `boolean` | excluded from the authority graph |
| `context` | `text` | `body`\|`nav`\|`footer`\|`main` |
| `first_seen`, `last_seen` | | |

`UNIQUE (source_doc_id, target_url_id, is_nofollow)` makes the graph an edge set,
not a link count: one page with forty links to a host counts once. Navigation and
footer links are excluded from authority (`docs/ranking/authority.md`).

Indexes: `link_target_idx`, `link_source_idx`, `link_graph_idx` (partial).

## `robots_policy`

Keyed by `host_id` (PK, `ON DELETE CASCADE`). `raw` is retained because robots.txt
is required for compliance and a dispute must be answerable. `parsed` holds rules,
sitemaps, and crawl-delay. `expires_at` drives TTL refresh; `etag` /
`last_modified` enable conditional re-fetch.
