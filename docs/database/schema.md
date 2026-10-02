# Schema

Every table, with the reasoning that is not visible in the column list. `prune` means the row
exists only to support deletion.

## `url`

```sql
CREATE TABLE url (
  id             uuid PRIMARY KEY DEFAULT uuidv7(),
  url_hash       bytea  NOT NULL UNIQUE,   -- sha256(normalised_url)
  scheme         text   NOT NULL,
  host_id        uuid   NOT NULL REFERENCES crawl_host(id),
  path           text   NOT NULL,
  query_string   text,
  fragment       text,

  canonical_url  text   NOT NULL,          -- after normalisation
  normalised     jsonb  NOT NULL,          -- what canonicalisation changed
  first_seen     timestamptz NOT NULL DEFAULT now(),
  last_seen      timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX url_host_idx    ON url (host_id);
CREATE INDEX url_path_idx    ON url (host_id, path);
CREATE UNIQUE INDEX url_hash_idx ON url (url_hash);
```

`url_hash` is unique on the *normalised* form, so `//`, trailing-slash, case, and
tracking-parameter variants collapse to one row. `normalised` records what was changed, which is
what makes an unexpected merge diagnosable rather than mysterious.

`fragment` is stored but never crawled. Fragments are client-side state; fetching them creates
duplicate content.

## `crawl_host`

```sql
CREATE TABLE crawl_host (
  id                   uuid PRIMARY KEY DEFAULT uuidv7(),
  hostname             text   NOT NULL,
  registrable_domain   text   NOT NULL,
  resolved_ip          inet,
  ip_verified_at       timestamptz,

  -- politeness budget, shared across subdomains of one registrable domain
  rate_limit           int    NOT NULL DEFAULT 5,       -- seconds between requests
  requests_in_window   int    NOT NULL DEFAULT 0,
  window_started_at    timestamptz,
  next_request_at      timestamptz NOT NULL DEFAULT now(),
  concurrency_limit    int    NOT NULL DEFAULT 1,

  -- observed behaviour
  crawl_delay_seconds  int,
  robots_fetched_at    timestamptz,
  robots_status        int,               -- HTTP status of robots.txt
  last_success_at      timestamptz,
  last_failure_at      timestamptz,
  consecutive_failures int    NOT NULL DEFAULT 0,
  failure_kind         text,              -- timeout | 4xx | 5xx | dns | tls | blocked

  -- content-type profile, for cost estimation
  avg_content_bytes    bigint,
  avg_parse_ms         int,
  content_mix          jsonb,

  first_seen           timestamptz NOT NULL DEFAULT now(),
  updated_at           timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX crawl_host_domain_idx ON crawl_host (registrable_domain);
CREATE INDEX crawl_host_due_idx    ON crawl_host (next_request_at);
CREATE UNIQUE INDEX crawl_host_name_idx ON crawl_host (hostname);
```

The politeness columns are per **registrable domain**, not per hostname, and the bucket is
enforced in the frontier claim query
([../database/README.md](../database/README.md)). A site running a thousand subdomains gets one
budget.

`resolved_ip` plus `ip_verified_at` records the SSRF-validated address, and the crawler connects
to it rather than re-resolving. That is what closes DNS rebinding.

`content_mix` is a JSONB histogram of observed content types, used to estimate a host's crawl
cost before committing budget to it.

## `frontier`

```sql
CREATE TYPE frontier_state AS ENUM
  ('pending','claimed','fetched','failed','skipped','blocked');

CREATE TABLE frontier (
  id             uuid PRIMARY KEY DEFAULT uuidv7(),
  url_id         uuid NOT NULL REFERENCES url(id) ON DELETE CASCADE,
  host_id        uuid NOT NULL REFERENCES crawl_host(id),
  parent_url_id  uuid REFERENCES url(id),
  link_text      text,
  depth          smallint NOT NULL DEFAULT 0,

  priority       smallint NOT NULL DEFAULT 0,   -- sitemap > seed > link depth
  state          frontier_state NOT NULL DEFAULT 'pending',
  available_at   timestamptz NOT NULL DEFAULT now(),  -- backoff lives here
  attempts       smallint NOT NULL DEFAULT 0,
  last_error     text,
  failure_kind   text,

  claimed_by     text,
  claimed_at     timestamptz,
  fetched_at     timestamptz,

  created_at     timestamptz NOT NULL DEFAULT now()
) PARTITION BY RANGE (created_at);

CREATE INDEX frontier_claim_idx ON frontier (state, available_at, priority DESC)
  WHERE state = 'pending';
CREATE INDEX frontier_reap_idx  ON frontier (claimed_at) WHERE state = 'claimed';
```

Partitioning by month on `created_at` keeps the hot index small and makes retention a matter of
dropping a partition rather than deleting millions of rows.

`available_at` is where backoff lives, rather than a computed expression. That makes backoff
inspectable — `SELECT * FROM frontier WHERE available_at > now()` shows every URL waiting on a
rate limit, which is the most useful crawler diagnostic there is.

## `document`

```sql
CREATE TYPE document_state AS ENUM ('active','shadow_suppressed','soft_404','deleted');

CREATE TABLE document (
  doc_id         text PRIMARY KEY,           -- ULID, matches the index doc_id
  url_id         uuid NOT NULL REFERENCES url(id),
  state          document_state NOT NULL DEFAULT 'active',
  content_hash   bytea,
  signature_id   uuid REFERENCES dedup_signature(id),
  cluster_id     uuid,

  lang           text,
  published_at   timestamptz,
  modified_at    timestamptz,
  content_type   text,

  -- quality and spam labels, kept for trend analysis and reprocessing
  quality_score  real,
  spam_score     real,
  spam_signals   text[],
  js_required    boolean,

  indexed_generation text,
  first_indexed_at timestamptz,
  last_changed_at  timestamptz NOT NULL DEFAULT now(),

  created_at     timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX document_cluster_idx ON document (cluster_id);
CREATE INDEX document_hash_idx     ON document (content_hash);
CREATE INDEX document_state_idx    ON document (state) WHERE state <> 'active';
CREATE INDEX document_content_idx ON document USING gist (to_tsvector('simple', lang));
```

The Postgres row is a **label** for a document whose text lives in Tantivy. `content_hash` is a
SipHash of the extracted text, used for change detection before parsing and for cheap
deduplication. SipHash rather than SHA-256 because the input is not secret but the output
should not be a rainbow-table lookup key for content.

`shadow_suppressed` means the document exists in the index but is excluded from results. It is
distinct from `deleted`, which must never be resurrectable, and from `soft_404`, which is
indexed as a 404.

## `link`

```sql
CREATE TABLE link (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  source_doc_id text NOT NULL,
  target_url_id uuid NOT NULL REFERENCES url(id),
  target_doc_id text,                        -- null until the target is indexed
  anchor_text   text,
  is_nofollow   boolean NOT NULL DEFAULT false,
  is_sponsored  boolean NOT NULL DEFAULT false,
  context       text,                        -- 'body' | 'nav' | 'footer' | 'main'

  first_seen    timestamptz NOT NULL DEFAULT now(),
  last_seen     timestamptz NOT NULL DEFAULT now(),
  UNIQUE (source_doc_id, target_url_id, is_nofollow)
);
CREATE INDEX link_target_idx   ON link (target_url_id);
CREATE INDEX link_source_idx   ON link (source_doc_id);
CREATE INDEX link_graph_idx    ON link (target_doc_id) WHERE target_doc_id IS NOT NULL;
```

`context` matters for PageRank: a link from the footer is not an editorial endorsement, and
`[../ranking/authority.md](../ranking/authority.md)` excludes navigation and footer links from
the authority graph.

`UNIQUE (source_doc_id, target_url_id, is_nofollow)` collapses repeated links to the same target,
which makes the graph an edge set rather than a link count — otherwise one page with forty links
to the same host would dominate.

## `tombstone`

```sql
CREATE TABLE tombstone (
  doc_id       text PRIMARY KEY,
  url_id       uuid REFERENCES url(id),
  reason       text NOT NULL,     -- unindex | takedown | ban | soft_404 | court_order
  requested_by text NOT NULL,     -- 'robots' | 'meta' | 'operator' | 'legal' | 'dmca'
  created_at   timestamptz NOT NULL DEFAULT now(),
  applied_to   text[] NOT NULL DEFAULT '{}',  -- generations it was removed from
  verified     boolean NOT NULL DEFAULT false
);
```

Written **before** removal. The tombstone is the durable record that a deletion was requested, so
a replay of the crawl cannot resurrect the document. `verified` is set only after absence has
been confirmed by querying the index, because a deletion that is not verified is a deletion that
has only been asked for.

## `dedup_signature`

```sql
CREATE TABLE dedup_signature (
  id             uuid PRIMARY KEY DEFAULT uuidv7(),
  structural_hash bytea NOT NULL,      -- DOM shingles, text excluded
  text_hash       bytea,               -- full-text hash, text included
  shingles        int  NOT NULL,       -- count, for banding
  cluster_id      uuid NOT NULL,
  member_count    int  NOT NULL DEFAULT 0,
  created_at      timestamptz NOT NULL DEFAULT now(),
  last_seen_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX dedup_structural_idx ON dedup_signature (structural_hash);
CREATE INDEX dedup_cluster_idx    ON dedup_signature (cluster_id);
```

Two hashes because near-duplicate detection needs both: a structural signature that ignores text
catches template farms, and a text hash catches verbatim copies. Using one for both produces
false positives on legitimate templated publishing and false negatives on paraphrased spam.

## `cluster`

```sql
CREATE TABLE cluster (
  id           uuid PRIMARY KEY DEFAULT uuidv7(),
  kind         text NOT NULL,     -- 'near_duplicate' | 'doorway' | 'template'
  confidence   real NOT NULL,
  representative_doc_id text,
  member_count int NOT NULL DEFAULT 0,
  host_diversity int NOT NULL DEFAULT 1,
  created_at   timestamptz NOT NULL DEFAULT now()
);
```

`host_diversity` separates a syndicated press release (many hosts, one legitimate document) from
a doorway network (many hosts, one template). Both look identical structurally, and the
distinction is what stops legitimate publishing from being penalised.

## `robots_cache`

```sql
CREATE TABLE robots_cache (
  host_id          uuid PRIMARY KEY REFERENCES crawl_host(id) ON DELETE CASCADE,
  raw              text,
  parsed           jsonb NOT NULL,     -- rules, sitemaps, crawl-delay
  status           int,
  etag             text,
  last_modified    text,
  fetched_at       timestamptz NOT NULL DEFAULT now(),
  expires_at       timestamptz NOT NULL,
  parse_error      text
);
```

`raw` is retained because robots.txt is required for compliance and a dispute about what a site
asked for must be answerable. It is public configuration text, not content.

## `quality_label`

```sql
CREATE TABLE quality_label (
  doc_id        text NOT NULL REFERENCES document(doc_id) ON DELETE CASCADE,
  labeler       text NOT NULL,     -- 'rule' | 'human' | 'model'
  quality       smallint NOT NULL, -- 0..3
  spam          boolean NOT NULL,
  notes         text,
  created_at    timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (doc_id, labeler)
);
```

Human labels are the evaluation set for [../ranking/spam.md](../ranking/spam.md). `labeler` is
part of the key so rule-derived and human labels coexist without overwriting each other.

## `analytics_result`

```sql
CREATE TABLE analytics_result (
  id              bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  rendered_at     timestamptz NOT NULL DEFAULT now(),
  interface_lang  text NOT NULL,
  -- the results, ranked; the query is NOT here and there is no column for it
  top_results     text[] NOT NULL,     -- doc_ids
  query_length    smallint NOT NULL,    -- length only, never the text
  term_count      smallint NOT NULL,
  config_version  text NOT NULL,
  index_generation text NOT NULL
) PARTITION BY RANGE (rendered_at);
```

This table exists to demonstrate the shape of an acceptable measurement, and it is **disabled by
default**. Note what is absent: the query. `query_length` and `term_count` are lengths, not
content. A query longer than 64 bytes is excluded entirely, because short generic queries are
the ones where a hash would be trivially reversible and a length is reversible by nobody.

No `ip`, no `user_agent`, no `session`.

## `crawl_log`

```sql
CREATE TABLE crawl_log (
  id           bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  url_id       uuid NOT NULL,
  host_id      uuid NOT NULL,
  status       int,
  content_type text,
  bytes        int,
  duration_ms  int,
  outcome      text NOT NULL,      -- ok | error | blocked | skipped
  error_kind   text,
  created_at   timestamptz NOT NULL DEFAULT now()
) PARTITION BY RANGE (created_at);
```

Diagnostic, 30-day retention. `outcome` distinguishes a technical failure from a policy
decision, because "we did not fetch it" and "the fetch failed" are different answers to the
question an operator is asking.

## Constraints worth naming

| Constraint | Reason |
| ---------- | ------ |
| `url_hash` unique | Canonicalisation must be idempotent |
| `link` unique on (source, target, nofollow) | Edge set, not link count |
| `frontier.available_at` indexed | The claim query is the hot path |
| `document.doc_id` = Tantivy `doc_id` | The join key; a mismatch is corruption |
| `crawl_host.registrable_domain` indexed | Politeness lookup |
| `content_hash` on text, not HTML | Format changes must not look like content changes |
| FK `ON DELETE CASCADE` on URL-derived rows | Deleting a URL must not orphan work |

## Triggers and rules

| Rule | Purpose |
| ---- | ------- |
| `updated_at` maintained by trigger | Avoids clock divergence in application code |
| No delete trigger on `tombstone` | A tombstone must not be removable |
| No update trigger on `analytics_result` | It is append-only by construction |
| `frontier` transitions validated in application code, not by trigger | Business logic belongs in one place |

Deliberately absent: row-level security. There is no per-user data to isolate, so RLS would add
a policy surface with nothing to protect.

## Index maintenance

| Operation | Frequency | Notes |
| --------- | --------- | ----- |
| `VACUUM ANALYZE` | Nightly | Autovacuum tuned for the frontier table |
| Partition drop | Monthly | `crawl_log`, `analytics_result`, old `frontier` partitions |
| `REINDEX` on `link` | Quarterly | The largest and hottest index |
| Statistics review | Quarterly | After a major planner-visible change |
| `EXPLAIN` review of the claim query | Quarterly | It is the hot path; a plan regression is an outage |

The last row is the one worth keeping on a schedule. The claim query is a partial index scan
over a partitioned table, and the conditions that keep it fast are exactly the ones a schema
change would quietly break.