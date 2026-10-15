# Crawl plane (`0004_crawl_plane_core.sql`, `0005_crawl_plane_ops.sql`)

The crawl graph and the queue. Nothing here is joined with anything containing
user data; it is a crawl dataset, not a user dataset.

## `domain`

The registrable domain — the unit of authority and of crawl budget. A site running
a thousand subdomains gets one budget.

| Column | Type | Notes |
| ------ | ---- | ----- |
| `id` | `uuid` PK | |
| `name` | `text` UNIQUE | registrable domain, lowercased punycode |
| `first_seen`, `updated_at` | `timestamptz` | |

## `host`

A specific hostname. Rules are per host; budget is per domain. `resolved_ip` plus
`ip_verified_at` records the SSRF-validated address, and the crawler connects to it
rather than re-resolving — that is what closes DNS rebinding.

| Column | Type | Notes |
| ------ | ---- | ----- |
| `id`, `domain_id` | `uuid` | `domain_id` → `domain(id)` FK |
| `name` | `text` UNIQUE | full hostname, lowercased punycode |
| `resolved_ip`, `ip_verified_at` | `inet`, `timestamptz` | validated address |
| `rate_limit_seconds` | `integer` | min seconds between requests, default 5 |
| `requests_in_window`, `window_started_at` | | politeness window |
| `next_request_at` | `timestamptz` | earliest next request |
| `concurrency_limit` | `integer` | per-host in-flight cap |
| `robots_fetched_at`, `robots_status` | | robots bookkeeping |
| `last_success_at`, `last_failure_at`, `consecutive_failures`, `failure_kind` | | observed behaviour |
| `avg_content_bytes`, `avg_parse_ms`, `content_mix` | | cost estimation |
| `first_seen`, `updated_at` | | |

Indexes: `host_domain_idx (domain_id)`, `host_due_idx (next_request_at)`.

## `url`

`url_hash` is `sha256(normalised_url)` and is unique, so `//`, trailing-slash,
case, and tracking-parameter variants collapse to one row. `normalised` records
what canonicalisation changed. `fragment` is stored but never crawled.

| Column | Type | Notes |
| ------ | ---- | ----- |
| `id`, `host_id` | `uuid` | `host_id` → `host(id)` FK |
| `url_hash` | `bytea` UNIQUE | canonical key |
| `scheme`, `path` | `text` | |
| `query_string`, `fragment` | `text` | nullable |
| `canonical_url` | `text` | after normalisation |
| `normalised` | `jsonb` | what was changed, for diagnosis |
| `first_seen`, `last_seen`, `updated_at` | | |

Indexes: `url_host_idx`, `url_path_idx (host_id, path)`, `url_hash_idx` (unique).

## `crawl_frontier`

The crawler's memory and its throttle. Range-partitioned by `created_at` with a
`DEFAULT` partition. `url_id` and `host_id` are **logical references** (no FK).

| Column | Type | Notes |
| ------ | ---- | ----- |
| `id` | `uuid` | PK is `(id, created_at)` |
| `url_id`, `host_id` | `uuid` | logical refs |
| `parent_url_id` | `uuid` | discovery parent |
| `link_text` | `text` | anchor that led here |
| `depth` | `smallint` | link depth |
| `priority` | `real` | higher claimed first |
| `state` | `crawl_state` | `pending`\|`ready`\|`claimed`\|`done`\|`denied`\|`banned` |
| `not_before` | `timestamptz` | backoff / politeness; the useful diagnostic |
| `attempts` | `smallint` | |
| `last_error` | `text` | |
| `generation` | `bigint` | target index generation |
| `discovery` | `discovery_source` | `outlink`\|`sitemap`\|`feed`\|`seed`\|`operator` |
| `claimed_by`, `claimed_at`, `fetched_at` | | worker bookkeeping |
| `created_at` | `timestamptz` | partition key |

The claim query (see `docs/database/README.md`) is a partial index scan over
`crawl_frontier_claim_idx (state, not_before, priority DESC) WHERE state IN
('pending','ready')`, joined to `host` for politeness, using `FOR UPDATE SKIP
LOCKED`. **Do not change the shape of that index without re-checking the plan**; a
plan regression here is an outage.

## `crawl_budget`

Per-host, per-UTC-day counters. Budgets are enforced independently of robots.

`(host_id, day)` PK; `pages_fetched`, `bytes_fetched`, `robots_delay_ms`,
`cooldown_until`.

## `crawl_attempt` (`0005`)

Diagnostic history, range-partitioned by `started_at`, 30-day retention.
`outcome` is `ok` | `error` | `blocked` | `skipped` — a technical failure and a
policy decision are different answers. `error_class` is a closed enum.

## `crawl_ban` (`0005`)

Per-URL, per-host, or per-domain bans, with a `CHECK` enforcing that the scope
matches the set column. Every ban stores a reason and an expiry.

## `crawl_stat_daily` (`0005`)

Daily rollup per host: `pages_ok`, `pages_error`, `pages_denied`, `bytes_fetched`,
`errors_by_class` (`jsonb`). Retention class `aggregate`.
