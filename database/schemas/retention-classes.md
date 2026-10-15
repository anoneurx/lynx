# Retention classes

Every table declares exactly one class. A table that cannot be assigned a class
does not get created; the rows seeded in `retention_policy` are the source of
truth for the day counts, and a retention job reads them.

| Class | Meaning | Default retention |
| ----- | ------- | ----------------- |
| `crawl_permanent` | Life of the URL in the index | until deindex / purge |
| `crawl_temporary` | Operational history | 30–90 days |
| `governance` | Legal / anti-abuse records | 90 days – 2 years |
| `aggregate` | Metrics rollups | 13 months |
| `no_user_data` | Contains no personal data by construction | n/a |

## Assignment

| Table | Class | Mechanism |
| ----- | ----- | --------- |
| `domain` | `crawl_permanent` | needed for authority history |
| `host` | `crawl_permanent` | needed for politeness accounting |
| `url` | `crawl_permanent` | soft-deleted, not hard-deleted |
| `crawl_frontier` | `crawl_temporary` | partition drop after terminal state |
| `crawl_budget` | `crawl_permanent` | politeness counters, day rows pruned by rollup |
| `crawl_attempt` | `crawl_temporary` | partition drop, 30 days |
| `crawl_ban` | `governance` | until reviewed + retention |
| `crawl_stat_daily` | `aggregate` | 13-month rollup |
| `document` | `crawl_permanent` | soft delete via `state` / takedown |
| `document_version` | `crawl_temporary` | newest retained; older pruned |
| `link` | `crawl_permanent` | source of the authority graph |
| `robots_policy` | `crawl_temporary` | TTL expiry per `expires_at` |
| `index_manifest`, `index_shard`, `index_segment` | `crawl_permanent` | generation lifetime |
| `dedup_cluster` | `crawl_permanent` | cluster identity |
| `admin_user`, `admin_role`, `admin_user_role` | `governance` | operator identity |
| `api_key`, `api_key_scope` | `governance` | developer credentials |
| `admin_audit_log` | `governance` | never edited, retained for audit |
| `abuse_report` | `governance` | personal data, encrypted |
| `takedown_request` | `governance` | deletion must be provable |
| `quality_evaluation_run` | `aggregate` | evaluation history |
| `relevance_judgement` | `aggregate` | reviewed golden set |
| `ranking_config_version` | `no_user_data` | configuration, not data |
| `retention_policy`, `schema_migrations` | `no_user_data` | operational metadata |

## What is deliberately absent

No table holds a query, a session, a user profile, a cookie, or an impression.
There is no `analytics_result`-style table in the physical schema: the shape of an
acceptable measurement exists in `docs/database/schema.md` as a specification, but
it is disabled by default and is not created by any migration. If it is ever
enabled it must arrive with an ADR, because it is a privacy-surface change.
